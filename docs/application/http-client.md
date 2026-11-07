---
status: unavailable
---

# HTTP client

Corvis does not include an HTTP client module (an injectable service for calling other APIs with per-module configuration).

## Alternative

Use a Rust HTTP client, such as `reqwest`, and share it as a provider. A client of this kind keeps a connection pool, so it is best to create a single instance:

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

`reqwest` is not a Corvis dependency: add it to your `Cargo.toml`. `Provider::use_factory` and `#[inject("TOKEN")]` are explained in [Custom providers](/fundamentals/custom-providers).
