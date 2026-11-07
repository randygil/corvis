---
status: unavailable
---

# Supabase

Corvis **has no specific Supabase integration**: there is no client for its API, its authentication, its storage or its realtime channels.

## What does work

**The database.** The database of a Supabase project is PostgreSQL, and `corvis-orm` connects to PostgreSQL with TLS (feature `tls-rustls`, enabled by default). Use the project's connection string as `DATABASE_URL`:

```rust
use corvis::prelude::*;
use corvis_orm::OrmModule;

#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        OrmModule::for_root_from_config(true),
    ],
)]
pub struct AppModule;
```

In production use `sslmode=verify-full` in the URL. See [corvis-orm](/data/corvis-orm).

**Sign-in tokens.** They are JWTs. If your project signs them with a shared secret, `JwtService` can verify them by configuring `JwtModule` with that secret, and a [guard](/guards) protects the routes. If it uses asymmetric keys, configure `JwtModuleOptions` with `.algorithm(..)` and `.public_key(pem)`. Check in your project which algorithm and which keys it uses before relying on the verification.

The rest of the services (storage, functions, realtime) are consumed through their HTTP API; Corvis does not ship an HTTP client of its own (see [HTTP client](/application/http-client)).
