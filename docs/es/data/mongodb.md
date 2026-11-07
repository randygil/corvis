---
status: no-disponible
---

# MongoDB

Corvis no incluye integración con MongoDB, y `corvis-orm` tampoco lo soporta: solo trabaja con PostgreSQL y SQLite.

## Alternativa

Usa el driver oficial de MongoDB para Rust y comparte el cliente como provider. La fábrica es asíncrona, así que puede conectar y leer la URL desde `ConfigService`:

```rust
Provider::use_factory("MONGO", |ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let url: String = config.try_get("MONGO_URL")?;
    Ok(mongodb::Client::with_uri_str(&url).await?)
})
```

El crate `mongodb` es una dependencia de tu aplicación. Los consumidores lo piden con `#[inject("MONGO")] client: Arc<mongodb::Client>` (ver [Providers personalizados](/es/fundamentals/custom-providers)).
