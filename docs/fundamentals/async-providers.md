---
status: documentado
---

# Providers asíncronos

A veces un provider no se puede crear hasta que termina una operación asíncrona: abrir un pool de conexiones, descargar una clave pública, leer un archivo de configuración. En Corvis toda la construcción del contenedor es asíncrona, así que no hace falta nada especial: una fábrica de `Provider::use_factory` es un `async` y el arranque espera a que termine.

```rust
use corvis::prelude::*;

#[module(
    providers = [
        Provider::use_factory("JWKS", |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let path: String = config.get_or("JWKS_PATH", "./jwks.json".to_string());
            let raw = corvis::tokio::fs::read_to_string(&path).await?;
            let keys: Value = corvis::serde_json::from_str(&raw)?;
            Ok(keys)
        }),
        KeysService,
    ],
)]
pub struct KeysModule;

#[injectable]
pub struct KeysService {
    #[inject("JWKS")]
    jwks: Arc<Value>,
}
```

Puntos a tener en cuenta:

- **Se resuelve una vez.** Una fábrica con scope por defecto se ejecuta una sola vez, al arrancar, y todos los consumidores reciben el mismo `Arc`.
- **Las dependencias esperan.** Si `KeysService` pide `"JWKS"`, el contenedor ejecuta la fábrica primero. El orden de la lista `providers` no importa.
- **Los errores detienen el arranque.** Un `Err` de la fábrica hace que `CorvisFactory::create` devuelva el error, envuelto como `Error while instantiating JWKS: ...`, y lo registra con el logger `ExceptionHandler`. La aplicación no llega a escuchar.
- **No bloquees el runtime.** La fábrica corre dentro de Tokio: usa `tokio::fs`, clientes asíncronos o `tokio::task::spawn_blocking` para trabajo bloqueante.

## Un ejemplo real: la base de datos de Tertulia

`OrmModule::for_root_async` recibe una fábrica asíncrona que arma las opciones de conexión a partir de `ConfigService`. El módulo registra esas opciones con `Provider::use_factory` y, a partir de ellas, crea `OrmService`:

```rust
OrmModule::for_root_async(true, |ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let url: String = config.get_or("DATABASE_URL", "file:./tertulia.db".to_string());
    Ok(OrmModuleOptions::new(url)
        .log_queries(config.get_or("LOG_QUERIES", false))
        .push::<(Room, ChatMessage)>())
})
```

`JwtModule::register_async` sigue el mismo patrón para construir `JwtService` con el secreto que viene de la configuración. Cómo escribir tus propios métodos `*_async` está en [Módulos dinámicos](/fundamentals/dynamic-modules#opciones-asincronas).

## Trabajo asíncrono después de crear el provider

Si el provider se puede construir enseguida y lo asíncrono es una preparación posterior, impleméntala en un hook del ciclo de vida en lugar de en la fábrica. `RoomsService` de Tertulia crea las salas por defecto en `on_application_bootstrap`, cuando todas las dependencias ya existen:

```rust
#[async_trait]
impl OnApplicationBootstrap for RoomsService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        for (name, topic) in DEFAULT_ROOMS {
            if self.find(name).await?.is_none() {
                // crea la sala
            }
        }
        Ok(())
    }
}
```

Ver [Eventos del ciclo de vida](/fundamentals/lifecycle-events).
