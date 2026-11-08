---
status: documented
---

# Eventos del ciclo de vida

Una aplicación de Corvis pasa por tres fases: **arranque** (se crea el contenedor y se instancian los providers), **ejecución** (atiende peticiones) y **apagado**. En los límites entre fases Corvis llama a *hooks* que tus providers, controladores y módulos pueden implementar para preparar o liberar recursos.

## Los hooks

Cada hook es un trait asíncrono:

| Trait | Método | Cuándo se llama |
|---|---|---|
| `OnModuleInit` | `on_module_init(&self)` | Al inicializar, con todas las dependencias ya resueltas |
| `OnApplicationBootstrap` | `on_application_bootstrap(&self)` | Después de todos los `on_module_init`, antes de aceptar conexiones |
| `OnModuleDestroy` | `on_module_destroy(&self)` | Al cerrar la aplicación (`app.close()` o una señal) |
| `BeforeApplicationShutdown` | `before_application_shutdown(&self, signal: Option<String>)` | Después de todos los `on_module_destroy` |
| `OnApplicationShutdown` | `on_application_shutdown(&self, signal: Option<String>)` | Al final del apagado |

Todos devuelven `corvis::Result<()>`. Basta con implementar el trait: `#[injectable]`, `#[controller]` y `#[module]` detectan la implementación al compilar, sin registro adicional.

```rust
use corvis::prelude::*;

#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

#[async_trait]
impl OnApplicationBootstrap for RoomsService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        for (name, topic) in DEFAULT_ROOMS {
            if self.find(name).await?.is_none() {
                self.db
                    .room()
                    .create(json!({ "data": { "name": name, "topic": topic, "createdBy": "tertulia" } }))
                    .await?;
            }
        }
        Ok(())
    }
}
```

(`examples/tertulia/src/rooms/rooms_service.rs`: crea las salas por defecto al arrancar.)

En `examples/tertulia/src/firehose.rs`, `FirehoseService` usa `on_application_bootstrap` para registrar un listener `upgrade` en el servidor HTTP: en ese momento el contenedor ya está completo y la aplicación todavía no acepta conexiones.

## La secuencia completa

### Arranque

1. `CorvisFactory::create::<AppModule>()` recorre el grafo de módulos y crea todos los providers y controladores (salvo los de scope `Transient`, que se crean al inyectarse). Si algo falla aquí, `create` devuelve el error.
2. `app.init()` (que `listen` llama por ti si no lo hiciste) registra las rutas, los middleware, los gateways y los handlers de eventos, y después:
   1. llama a `on_module_init` en todos los módulos;
   2. monta las tareas programadas (`#[cron]`, `#[interval]`, `#[timeout]`);
   3. llama a `on_application_bootstrap` en todos los módulos.
3. `listen` abre el socket y empieza a atender peticiones.

Después de un `init()` correcto, volver a llamarlo (directamente, o a través de `listen`, `handle` o `TestingModule::init`) no hace nada: los hooks no se repiten.

Después de un `init()` **fallido** la aplicación no puede arrancar de nuevo. La primera llamada devuelve el error original; cada llamada posterior devuelve `corvis::InitFailedError`, cuyo mensaje dice que la aplicación falló al inicializarse antes, cita el primer error y pide crear una aplicación nueva. Los hooks que ya se ejecutaron (conexiones abiertas, tareas lanzadas) no se pueden deshacer, así que reintentar sobre la misma aplicación los repetiría o los omitiría. Crea una aplicación nueva con `CorvisFactory::create`. Puedes reconocer el error con `err.downcast_ref::<corvis::InitFailedError>()`. Un `listen` que falla al enlazar después de un `init()` correcto no es un fallo de init: puedes llamarlo de nuevo con otra dirección.

### Apagado

`app.close().await` (o una señal, ver abajo) ejecuta:

1. `on_module_destroy` en todos los módulos;
2. detiene las tareas programadas;
3. `before_application_shutdown(signal)`;
4. `on_application_shutdown(signal)`.

`close()` también es idempotente.

### Orden entre módulos

Los hooks se recorren por módulo, empezando por los **más alejados** del módulo raíz (los importados más profundamente) y terminando en el raíz. Dentro de cada módulo el orden es: controladores, providers y, por último, la propia clase del módulo. El apagado sigue el mismo orden que el arranque.

Si un hook devuelve `Err`, el recorrido se detiene y el error sale de `init()`, `listen()` o `close()`. Tras un fallo de `init()`, mira [la secuencia de arranque](#arranque) para saber qué pasa al reintentar.

### Qué instancias reciben hooks

- Providers con scope por defecto (singleton) y controladores.
- La clase del módulo (`#[module]` también es un provider): un módulo puede implementar `OnModuleInit` para tareas de configuración.
- **No** los reciben los providers `Transient` (no hay una instancia única que avisar) ni los guards, pipes, interceptors o filtros que Corvis construye bajo demanda sin estar en `providers`.

## Señales del sistema

Por defecto los hooks de apagado solo corren si llamas a `app.close()`. Para que la aplicación reaccione a las señales del sistema, actívalo antes de escuchar:

```rust
#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = tertulia::create_app().await?;
    app.enable_shutdown_hooks();
    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 5150);
    app.listen(format!("127.0.0.1:{port}")).await
}
```

Con `enable_shutdown_hooks()`, al recibir la señal:

1. el servidor deja de aceptar conexiones y espera a que terminen las que están en curso;
2. se ejecuta la secuencia de apagado, con `signal` igual a `Some("SIGINT")` o `Some("SIGTERM")`;
3. `listen` retorna.

En Unix se escuchan `SIGINT` (Ctrl+C) y `SIGTERM`. En Windows solo Ctrl+C, que se informa como `"SIGINT"`. Cuando cierras con `app.close()`, `signal` es `None`.

```rust
#[async_trait]
impl BeforeApplicationShutdown for JobsService {
    async fn before_application_shutdown(&self, signal: Option<String>) -> corvis::Result<()> {
        self.logger.log(format!("Shutting down ({})", signal.as_deref().unwrap_or("manual")));
        self.flush().await
    }
}
```

::: info Conexiones durante el apagado
Cuando el apagado lo inicia una señal, las conexiones HTTP ya terminaron antes de llamar a `on_module_destroy`. Los hooks sirven para cerrar recursos propios (colas, conexiones a bases de datos, archivos), no para seguir atendiendo peticiones.
:::

## Contextos sin servidor

`CorvisFactory::create_application_context::<M>()` crea el contenedor y ejecuta `on_module_init` y `on_application_bootstrap` sin registrar rutas ni abrir sockets. Es la base de los scripts y workers (ver [Aplicaciones standalone](/es/standalone-applications)). Cierra con `app.close().await` para que corran los hooks de apagado.

`create_application_context` también registra los handlers de `#[handlers]` (`#[on_event]`, `#[cron]`, `#[interval]`, `#[timeout]`) antes de `on_module_init` y arranca los programados antes de `on_application_bootstrap`, así que un worker recibe sus eventos y ejecuta sus tareas. `app.close().await` los detiene. Llamar a `app.init()` después nunca los registra dos veces.

En los tests, `TestingModule::init()` y `TestingModule::close()` disparan las mismas fases (ver [Testing](/es/fundamentals/testing)).
