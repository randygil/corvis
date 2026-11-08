---
status: unavailable
---

# MongoDB

Corvis does not include a MongoDB integration, and `corvis-orm` does not support it either: it only works with PostgreSQL and SQLite.

## Alternative

Use the official MongoDB driver for Rust and share the client as a provider. The factory is async, so it can connect and read the URL from `ConfigService`:

```rust
Provider::use_factory("MONGO", |ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let url: String = config.try_get("MONGO_URL")?;
    Ok(mongodb::Client::with_uri_str(&url).await?)
})
```

The `mongodb` crate is a dependency of your application. Consumers request it with `#[inject("MONGO")] client: Arc<mongodb::Client>` (see [Custom providers](/fundamentals/custom-providers)).
