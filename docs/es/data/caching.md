---
status: documentado
---

# Caché

Guardar en memoria el resultado de una operación costosa evita repetirla en cada petición. `CacheModule` ofrece un almacén en memoria con caducidad (`CacheManager`) y un interceptor que cachea respuestas `GET` completas (`CacheInterceptor`).

## Registrar el módulo

```rust
#[module(
    imports = [CacheModule::register(CacheOptions { is_global: true, ..Default::default() })],
)]
pub struct AppModule;
```

(Así lo registra `examples/cats-app`.) `CacheOptions`:

| Campo | Por defecto | Significado |
|---|---|---|
| `ttl` | `5_000` | Tiempo de vida por defecto, en milisegundos (`0` = no caduca) |
| `max` | `100` | Número máximo de entradas |
| `is_global` | `false` | Registra el módulo como global |

Sin `is_global: true`, importa `CacheModule::register(...)` en cada módulo que lo use. Ten en cuenta que cada llamada a `register` crea un almacén distinto.

## Cachear respuestas

Aplica `CacheInterceptor` a una ruta (o a todo el controlador) con `#[use_interceptors]`:

```rust
#[get("count")]
#[use_interceptors(CacheInterceptor)]
#[set_metadata(CACHE_TTL_METADATA, 5_000)]
fn count(&self) -> Value {
    json!({ "count": self.cats_service.count() })
}
```

(De `examples/cats-app/src/cats/cats_controller.rs`.) Comportamiento:

- Solo actúa en peticiones `GET`; los demás métodos pasan sin tocar la caché.
- La clave es la URL de la petición (con su query string). `#[set_metadata(CACHE_KEY_METADATA, "clave")]` la reemplaza por una fija.
- `#[set_metadata(CACHE_TTL_METADATA, ms)]` cambia el tiempo de vida para esa ruta o controlador.
- Solo se guardan respuestas exitosas con cuerpo JSON, texto o HTML. Los archivos y los streams no se cachean.
- Un acierto responde `200` con el cuerpo guardado; las cabeceras de la respuesta original no se guardan.

`CacheInterceptor` es un provider que necesita `CacheManager`: el módulo donde se usa debe poder verlo (importando `CacheModule` o registrándolo como global). Para aplicarlo a toda la aplicación, regístralo como interceptor global con inyección de dependencias:

```rust
#[module(
    imports = [CacheModule::register(CacheOptions { is_global: true, ..Default::default() })],
    providers = [Provider::app_interceptor::<CacheInterceptor>()],
)]
pub struct AppModule;
```

## Usar el almacén directamente

Inyecta `Arc<CacheManager>` (o, por token, `#[inject(corvis::CACHE_MANAGER)] cache: Arc<CacheManager>`):

```rust
#[injectable]
pub struct RatesService {
    cache: Arc<CacheManager>,
}

impl RatesService {
    pub async fn usd_rate(&self) -> f64 {
        self.cache
            .wrap("rates:usd", Some(60_000), || async { fetch_rate().await })
            .await
    }

    pub async fn invalidate(&self) {
        self.cache.del("rates:usd").await;
    }
}
```

| Método | Descripción |
|---|---|
| `get::<T>(clave).await` | `Option<T>`; `None` si no existe, caducó o no se puede convertir a `T` |
| `set(clave, valor, ttl).await` | Guarda cualquier valor `Serialize`; `ttl` en ms (`None` = el de las opciones) |
| `del(clave).await` | Borra una entrada |
| `clear().await` | Vacía el almacén |
| `wrap(clave, ttl, closure).await` | Devuelve lo guardado o ejecuta la closure, guarda y devuelve su resultado |
| `get_value(clave)` / `set_value(clave, valor, ttl)` | Versiones síncronas con `serde_json::Value` |

Los valores se guardan como JSON. Cuando se supera `max`, se descarta la entrada más antigua por orden de inserción (no la menos usada).

## Límites

- El almacén vive en la memoria del proceso: no se comparte entre instancias ni sobrevive a un reinicio. No hay adaptadores para almacenes externos como Redis; si los necesitas, crea un provider propio con el cliente del almacén (ver [Visión general](/es/data/overview)).
- La caché de respuestas no distingue usuarios: no la apliques a rutas cuyo contenido depende de quién hace la petición, salvo que fijes una clave que lo incluya.
