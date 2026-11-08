---
status: documented
---

# Visión general

Corvis no te ata a una base de datos. El acceso a datos es un provider más: lo registras en un módulo, lo inyectas donde lo necesitas y su ciclo de vida (conectar al arrancar, desconectar al cerrar) se engancha a los [eventos del ciclo de vida](/es/fundamentals/lifecycle-events). Tienes dos caminos.

## Opción integrada: `corvis-orm`

El workspace incluye `corvis-orm`, un ORM sobre `sqlx` para **PostgreSQL** y **SQLite** que ya viene preparado para la inyección de dependencias:

- `OrmModule::for_root(...)` / `for_root_async(...)` registran `OrmService`, que se conecta en `on_module_init` y se desconecta en `on_module_destroy`.
- Los modelos son structs con `#[derive(Model)]`; también se pueden generar desde un archivo existente (es compatible con `schema.prisma`).
- `OrmExceptionFilter` traduce los errores de la base a respuestas HTTP (`409` para una clave duplicada, `404` para un registro inexistente).

`examples/blog-orm` y `examples/tertulia` lo usan. La guía completa está en [corvis-orm](/es/data/corvis-orm).

## Cualquier otro crate, como provider

Si prefieres `sqlx` directamente, otro cliente SQL o una base que `corvis-orm` no soporta, registra el cliente o el pool con un provider de fábrica. La fábrica es asíncrona y puede leer otros providers, como `ConfigService`:

```rust
use corvis::prelude::*;
use sqlx::PgPool;

#[module(
    imports = [ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() })],
    providers = [
        Provider::use_factory(Token::of::<PgPool>(), |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let url: String = config.try_get("DATABASE_URL")?;
            Ok(PgPool::connect(&url).await?)
        }),
        UsersRepository,
    ],
    exports = [Token::of::<PgPool>()],
)]
pub struct DatabaseModule;

#[injectable]
pub struct UsersRepository {
    pool: Arc<PgPool>,
}

impl UsersRepository {
    pub async fn count(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&*self.pool).await
    }
}
```

Puntos clave:

- La fábrica devuelve `corvis::Result<T>`, así que `?` funciona con cualquier error que se convierta en `anyhow::Error`. Si falla, la aplicación no arranca y el mensaje indica qué provider no se pudo crear.
- `Token::of::<PgPool>()` registra el pool por su tipo, y los consumidores lo piden como `Arc<PgPool>`. Si prefieres un token de texto, usa `Provider::use_factory("PG_POOL", ...)` y `#[inject("PG_POOL")] pool: Arc<PgPool>`.
- Exporta el token si otros módulos van a usarlo, o marca el módulo como global.
- `sqlx` (o el crate que uses) es una dependencia de tu aplicación, no de Corvis.

Las fábricas, los tokens y los providers asíncronos se explican en [Providers personalizados](/es/fundamentals/custom-providers) y [Providers asíncronos](/es/fundamentals/async-providers).

## Errores de la base en los handlers

Corvis convierte cualquier error que implemente `std::error::Error` en un `HttpException` con estado `500`, así que `?` funciona en un handler que devuelve `HttpResult<T>`. Para dar respuestas más precisas, mapea los errores tú mismo o registra un [filtro de excepciones](/es/exception-filters) que los reconozca con `exception.downcast_cause::<TuError>()`, que es lo que hace `OrmExceptionFilter`.

## Integraciones que no existen

No hay módulos de integración para TypeORM, Drizzle, MongoDB, MikroORM o Sequelize: son librerías de Node.js. Las páginas de esta sección con esos nombres lo indican y proponen la alternativa en Rust.
