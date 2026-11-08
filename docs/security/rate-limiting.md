---
status: documented
---

# Rate limiting

Limiting how many requests a client can make in a time window protects the API from abuse and
brute-force attacks. Corvis solves this with `ThrottlerModule`, which stores the counters, and
`ThrottlerGuard`, which checks them on every request.

## Configuration

Import the module with one or more limits and register the guard. `ThrottlerOptions::new(ttl, limit)`
takes the window in **milliseconds** and the maximum number of requests per window:

```rust
use corvis::prelude::*;

#[module(
    imports = [
        ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 100)]),
        AuthModule,
        CatsModule,
    ],
    providers = [
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::app_guard::<AuthGuard>(),
    ],
)]
pub struct AppModule;
```

`ThrottlerModule::for_root` is global: you do not need to import it in every module. With
`Provider::app_guard` the guard is applied to all controller routes; you can also
apply it only to a controller or to a method with `#[use_guards(ThrottlerGuard)]`.

When the limit is exceeded, the request receives:

```json
{ "statusCode": 429, "message": "ThrottlerException: Too Many Requests" }
```

## How counting works

- The key of each counter combines the limit name, the client IP (`req.ip()`), the
  controller and the handler. Each route has its own counter per client.
- The window is fixed: the first access creates the counter with its expiry and later
  requests increment it until it expires.
- With several limits in the vector, all are checked and it is enough for one to be exceeded. To
  tell them apart, `ThrottlerOptions` has a public `name` field (`"default"` by default):

  ```rust
  ThrottlerModule::for_root(vec![
      ThrottlerOptions { name: "short".into(), ttl: 1_000, limit: 3 },
      ThrottlerOptions { name: "long".into(), ttl: 60_000, limit: 100 },
  ])
  ```

## Changing the limit per route

The guard reads three metadata keys, on the method or on the controller:

| Constant | Effect |
|---|---|
| `SKIP_THROTTLE` | `#[set_metadata(SKIP_THROTTLE, true)]` skips the limit |
| `THROTTLE_LIMIT` | Changes the maximum number of requests |
| `THROTTLE_TTL` | Changes the window, in milliseconds |

```rust
#[routes]
impl AuthController {
    #[post("login")]
    #[set_metadata(THROTTLE_LIMIT, 5)]
    #[set_metadata(THROTTLE_TTL, 60_000)]
    async fn sign_in(&self, #[body] dto: SignInDto) -> HttpResult<Value> {
        // ...
    }

    #[get("health")]
    #[set_metadata(SKIP_THROTTLE, true)]
    fn health(&self) -> &'static str {
        "ok"
    }
}
```

`THROTTLE_LIMIT` and `THROTTLE_TTL` apply to all configured limits, not to one by name.

## Storage

Counters live in memory, in `ThrottlerStorage`. It stores the `sha256` of each key, deletes expired
counters on every request and does not exceed 100,000 entries; when full it discards the one that
expires soonest. To change that cap:

```rust
use corvis::ThrottlerStorage;

ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)]).with_max_entries(500_000),
)
```

Being process memory, each instance of the application counts separately. There is no shared
storage (Redis or other) included.

## Behind a proxy

The key uses `req.ip()`, which by default is the socket address. Behind a reverse proxy all
requests would come from the proxy's IP and share a counter. Configure `trust proxy` so that
`req.ip()` reads `X-Forwarded-For`:

```rust
app.set_trust_proxy(1); // one proxy in front
```

Use `true` (trust all hops) only if every request goes through a proxy that
rewrites `X-Forwarded-For`: otherwise, a client can choose its IP and bypass the limit. See
[HTTP adapters](/http/performance).

## Limitations

- No informational headers are sent (`Retry-After`, `X-RateLimit-*`).
- `ThrottlerGuard` does not apply to WebSocket gateways even if it is registered as global.
