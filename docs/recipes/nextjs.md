---
status: unavailable
---

# Next.js

There is no Corvis integration with Next.js: Corvis cannot run the Next.js server or its server-side rendering, which need Node.js.

## Alternatives

**Static export.** If your Next.js application can be exported as static files, Corvis serves it with `ServeStaticModule`, including the *fallback* of a single-page application:

```rust
#[module(
    imports = [
        ServeStaticModule::for_root(vec![
            ServeStaticOptions::new("./web/out")
                .exclude(&["/api/(.*)"])
                .spa_fallback(true),
        ]),
    ],
    controllers = [/* your API */],
)]
pub struct AppModule;
```

With `app.set_global_prefix("api")`, the API and the frontend live on the same origin. See [Static files](/recipes/serve-static).

**Two services.** If you need server-side rendering, run Next.js and the Corvis API as separate processes and join them behind a reverse proxy, or enable [CORS](/security/cors) on the API.
