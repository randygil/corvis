---
status: documented
---

# Overview

Corvis does not tie you to a database. Data access is just another provider: you register it in a module, inject it where you need it, and its lifecycle (connect on startup, disconnect on shutdown) hooks into the [lifecycle events](/fundamentals/lifecycle-events). You have two paths.

## Built-in option: `corvis-orm`

The workspace includes `corvis-orm`, an ORM on top of `sqlx` for **PostgreSQL** and **SQLite** that is already prepared for dependency injection:

- `OrmModule::for_root(...)` / `for_root_async(...)` register `OrmService`, which connects in `on_module_init` and disconnects in `on_module_destroy`.
- Models are structs with `#[derive(Model)]`; they can also be generated from an existing file (it is compatible with `schema.prisma`).
- `OrmExceptionFilter` translates database errors into HTTP responses (`409` for a duplicate key, `404` for a missing record).

`examples/blog-orm` and `examples/tertulia` use it. The full guide is in [corvis-orm](/data/corvis-orm).

## Any other crate, as a provider

If you prefer `sqlx` directly, another SQL client, or a database that `corvis-orm` does not support, register the client or the pool with a factory provider. The factory is async and can read other providers, such as `ConfigService`:

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

Key points:

- The factory returns `corvis::Result<T>`, so `?` works with any error that converts into `anyhow::Error`. If it fails, the application does not start and the message says which provider could not be created.
- `Token::of::<PgPool>()` registers the pool by its type, and consumers request it as `Arc<PgPool>`. If you prefer a string token, use `Provider::use_factory("PG_POOL", ...)` and `#[inject("PG_POOL")] pool: Arc<PgPool>`.
- Export the token if other modules will use it, or mark the module as global.
- `sqlx` (or whichever crate you use) is a dependency of your application, not of Corvis.

Factories, tokens and async providers are explained in [Custom providers](/fundamentals/custom-providers) and [Async providers](/fundamentals/async-providers).

## Database errors in handlers

Corvis converts any error that implements `std::error::Error` into an `HttpException` with status `500`, so `?` works in a handler that returns `HttpResult<T>`. For more precise responses, map the errors yourself or register an [exception filter](/exception-filters) that recognizes them with `exception.downcast_cause::<YourError>()`, which is what `OrmExceptionFilter` does.

## Integrations that do not exist

There are no integration modules for TypeORM, Drizzle, MongoDB, MikroORM or Sequelize: they are Node.js libraries. The pages in this section with those names say so and suggest the Rust alternative.
