---
status: no-disponible
---

# Cliente HTTP

Corvis no incluye un módulo de cliente HTTP (un servicio inyectable para llamar a otras APIs con configuración por módulo).

## Alternativa

Usa un cliente HTTP de Rust, como `reqwest`, y compártelo como provider. Un cliente de este tipo mantiene un pool de conexiones, así que conviene crear una sola instancia:

```rust
#[module(
    providers = [
        Provider::use_factory("HTTP_CLIENT", |_ctx| async move {
            Ok(reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build()?)
        }),
        PaymentsService,
    ],
)]
pub struct PaymentsModule;

#[injectable]
pub struct PaymentsService {
    #[inject("HTTP_CLIENT")]
    http: Arc<reqwest::Client>,
}
```

`reqwest` no es una dependencia de Corvis: agrégala a tu `Cargo.toml`. `Provider::use_factory` y `#[inject("TOKEN")]` se explican en [Providers personalizados](/es/fundamentals/custom-providers).
