---
status: documented
---

# Caching

Keeping the result of an expensive operation in memory avoids repeating it on every request. `CacheModule` provides an in-memory store with expiration (`CacheManager`) and an interceptor that caches complete `GET` responses (`CacheInterceptor`).

## Register the module

```rust
#[module(
    imports = [CacheModule::register(CacheOptions { is_global: true, ..Default::default() })],
)]
pub struct AppModule;
```

(This is how `examples/cats-app` registers it.) `CacheOptions`:

| Field | Default | Meaning |
|---|---|---|
| `ttl` | `5_000` | Default time to live, in milliseconds (`0` = never expires) |
| `max` | `100` | Maximum number of entries |
| `is_global` | `false` | Registers the module as global |

Without `is_global: true`, import `CacheModule::register(...)` in every module that uses it. Keep in mind that each call to `register` creates a separate store.

## Cache responses

Apply `CacheInterceptor` to a route (or to the whole controller) with `#[use_interceptors]`:

```rust
#[get("count")]
#[use_interceptors(CacheInterceptor)]
#[set_metadata(CACHE_TTL_METADATA, 5_000)]
fn count(&self) -> Value {
    json!({ "count": self.cats_service.count() })
}
```

(From `examples/cats-app/src/cats/cats_controller.rs`.) Behavior:

- It only acts on `GET` requests; other methods pass through without touching the cache.
- The key is the request URL (including its query string). `#[set_metadata(CACHE_KEY_METADATA, "key")]` replaces it with a fixed one.
- `#[set_metadata(CACHE_TTL_METADATA, ms)]` changes the time to live for that route or controller.
- Only successful responses with a JSON, text or HTML body are stored. Files and streams are not cached.
- A hit responds `200` with the stored body; the headers of the original response are not stored.

`CacheInterceptor` is a provider that needs `CacheManager`: the module where it is used must be able to see it (by importing `CacheModule` or by registering it as global). To apply it to the whole application, register it as a global interceptor with dependency injection:

```rust
#[module(
    imports = [CacheModule::register(CacheOptions { is_global: true, ..Default::default() })],
    providers = [Provider::app_interceptor::<CacheInterceptor>()],
)]
pub struct AppModule;
```

## Use the store directly

Inject `Arc<CacheManager>` (or, by token, `#[inject(corvis::CACHE_MANAGER)] cache: Arc<CacheManager>`):

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

| Method | Description |
|---|---|
| `get::<T>(key).await` | `Option<T>`; `None` if it does not exist, has expired or cannot be converted to `T` |
| `set(key, value, ttl).await` | Stores any `Serialize` value; `ttl` in ms (`None` = the one from the options) |
| `del(key).await` | Deletes an entry |
| `clear().await` | Empties the store |
| `wrap(key, ttl, closure).await` | Returns the stored value, or runs the closure, stores its result and returns it |
| `get_value(key)` / `set_value(key, value, ttl)` | Synchronous versions using `serde_json::Value` |

Values are stored as JSON. When `max` is exceeded, the oldest entry by insertion order is discarded (not the least recently used).

## Limits

- The store lives in the process memory: it is not shared between instances and does not survive a restart. There are no adapters for external stores such as Redis; if you need one, create your own provider with the store's client (see [Overview](/data/overview)).
- The response cache does not tell users apart: do not apply it to routes whose content depends on who makes the request, unless you set a key that includes it.
