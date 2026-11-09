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
- The window slides: each request counts for `ttl` milliseconds from the moment it was made, and the
  count of a request is the number of hits of that key in the last `ttl`. A client cannot send twice the
  limit around the edge of a fixed window.
- The request that takes the count over the limit blocks the key (see [Blocking](#blocking)). Requests
  rejected while the key is blocked are not counted and do not extend the block.
- With several limits in the vector, they are checked in ascending `ttl` order (ties keep the order
  given) and the first one that rejects the request stops the
  check: later limits do not count that request. To
  tell them apart, `ThrottlerOptions` has a public `name` field (`"default"` by default):

  ```rust
  ThrottlerModule::for_root(vec![
      ThrottlerOptions { name: "short".into(), ttl: 1_000, limit: 3 },
      ThrottlerOptions { name: "long".into(), ttl: 60_000, limit: 100 },
  ])
  ```

## Blocking

A key that goes over its limit is blocked for the **block duration**, counted from the moment of the
block: every request in that time answers `429` with `Retry-After` set to the time left. The first request
after the block starts counting again from one. The block duration is the throttler's `ttl` (after
`THROTTLE_TTL`) unless you set it:

- For a throttler, on the storage, by throttler name: `ThrottlerStorage::new(...).with_block_duration("default", 300_000)`
  (milliseconds). A name that matches no throttler panics when the storage is built.
- For a route or controller, with the `THROTTLE_BLOCK_DURATION` metadata (milliseconds, for every throttler);
  it takes precedence over the storage value.
- `0` turns the block off: a request is rejected while the key already holds `limit` hits from the last
  `ttl` and is not counted; `Retry-After` is the time until the oldest of those hits expires.

A limit of `0` rejects every request, including the first one after a block. The original library
(6.7.1) lets one request through each time a block ends; Corvis does not, like the library's current source.

This changes the behavior of apps that used the guard before: the block now lasts a whole block duration
from the moment of the block instead of the rest of a fixed window, and hits are counted in a sliding
window.

## Changing the limit per route

The guard reads four metadata keys, on the method or on the controller:

| Constant | Effect |
|---|---|
| `SKIP_THROTTLE` | `#[set_metadata(SKIP_THROTTLE, true)]` skips the limit |
| `THROTTLE_LIMIT` | Changes the maximum number of requests |
| `THROTTLE_TTL` | Changes the window, in milliseconds |
| `THROTTLE_BLOCK_DURATION` | Changes the block duration, in milliseconds (`0`: no block) |

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

`THROTTLE_LIMIT`, `THROTTLE_TTL` and `THROTTLE_BLOCK_DURATION` apply to all configured limits, not to one by name.

## Storage

Counters live in memory, in `ThrottlerStorage`. It stores the `sha256` of each key and one timestamp for
every hit it still counts. Keys with no live hit, an ended window and no block are deleted on every request.
Memory has two caps: at most 100,000 keys (`with_max_entries`) and at most 1,000,000 hit timestamps in total
(`with_max_hits`, about 16 MB). A key holds at most `limit + 1` hits. When either cap is reached the keys
that become idle soonest are discarded, which resets their counters. Under an attack with many addresses
that means other clients' counters can be reset too, so size the caps for your traffic; a `limit` above
roughly 10,000 per key needs a larger `with_max_hits`. To change the caps:

```rust
use corvis::ThrottlerStorage;

ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)])
        .with_max_entries(500_000)
        .with_max_hits(5_000_000),
)
```

Being process memory, each instance of the application counts separately. There is no shared
storage (Redis or other) included.

## Limits from another provider

`ThrottlerModule::for_root_async(factory)` builds the limits from a factory that can resolve other providers, such as `ConfigService`. The factory returns a `Vec<ThrottlerOptions>` or a configured `ThrottlerStorage`; the module is global, like `for_root`:

```rust
ThrottlerModule::for_root_async(|ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let limit = config.get_or("THROTTLE_LIMIT", 10u32);
    Ok(vec![ThrottlerOptions::new(60_000, limit)])
})
```

If the factory fails, the application does not start (`Error while instantiating ThrottlerStorage: ...`).

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

## Response headers

Like the original library, `ThrottlerGuard` tells the client where it stands. For every throttler that counts a request and lets it through it sends:

| Header | Value |
|---|---|
| `X-RateLimit-Limit` | The effective limit (after `THROTTLE_LIMIT`) |
| `X-RateLimit-Remaining` | The limit minus the hits counted in the last `ttl`, never below 0 |
| `X-RateLimit-Reset` | Seconds until the key's reporting window ends, rounded up |

When a throttler blocks the request, the `429` carries `Retry-After` (seconds until the block ends, at least 1) and no `X-RateLimit-*` header for that throttler. Headers of throttlers checked earlier stay on the `429`. Routes skipped with `SKIP_THROTTLE` get none of them.

For a throttler not named `default` the header names end in `-<name>`: with the `short` and `long` throttlers above a response carries `X-RateLimit-Limit-short`, `X-RateLimit-Remaining-long`, `Retry-After-long` and so on. The throttler name becomes part of a header name, so use a plain token (letters, digits and `-`) that says nothing internal; a name that is not a valid header name makes its headers be dropped.

The headers are on by default and tell every client each limit, its window and the throttler names. To hide them, keep counting and blocking as they are, and turn them off on the storage:

```rust
ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)]).with_headers(false),
)
```

Browsers only let a page read these headers if the CORS configuration exposes them (`Access-Control-Expose-Headers` in `CorsOptions`). `Retry-After` and the `X-RateLimit-*` values follow the original library for the same requests. A WebSocket message is counted but gets no headers. Other guards and interceptors can set headers with `ctx.switch_to_http().set_response_header` (see [Guards](/guards#setting-response-headers)).

## Limitations

- `SKIP_THROTTLE` skips every throttler, while the original's `@SkipThrottle()` skips only the throttler named `default` unless the others are listed.
- A global `ThrottlerGuard` also counts WebSocket gateway messages, per handshake IP and handler, so a long-lived connection can exhaust its limit. `app.apply_global_enhancers_to_gateways(false)` leaves gateways out. Plain `corvis::ws` listeners are never counted.
