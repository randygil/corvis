---
status: documented
---

# Health checks

An orchestrator (Kubernetes, a load balancer, an external monitor) needs to know whether an instance is healthy so it can send it traffic or restart it. `TerminusModule` provides `HealthCheckService`, which runs a list of indicators and builds a standard response: `200` if all are fine and `503` if any fails.

## Register the module

```rust
#[module(
    imports = [TerminusModule],
    controllers = [AppController],
)]
pub struct AppModule;
```

`TerminusModule` is a static module (imported by name, without `for_root`) that registers and exports `HealthCheckService`.

## A health endpoint

`examples/cats-app` exposes `/health` in its root controller:

```rust
#[controller]
#[set_metadata(IS_PUBLIC_KEY, true)]
pub struct AppController {
    app_service: Arc<AppService>,
    health: Arc<HealthCheckService>,
}

#[routes]
impl AppController {
    #[get("health")]
    #[version(VERSION_NEUTRAL)]
    async fn health(&self) -> HttpResult<Value> {
        self.health
            .check(vec![
                corvis::MemoryHealthIndicator::check_rss("memory_rss", 512 * 1024 * 1024),
                HealthIndicator::new("app", || async { Ok(json!({ "uptime": "ok" })) }),
            ])
            .await
    }
}
```

Two details in that example: the `IS_PUBLIC_KEY` metadata makes the app's global authentication guard let the route through, and `VERSION_NEUTRAL` keeps it reachable without a version prefix. A health check that requires a token or a version is usually a problem for the orchestrator.

## Response format

All indicators run in parallel, each with a [time limit](#time-limits). With all of them up, the response is `200`:

```json
{
  "status": "ok",
  "info": { "app": { "status": "up", "uptime": "ok" } },
  "error": {},
  "details": { "app": { "status": "up", "uptime": "ok" } }
}
```

If any fails, `check` returns an `Err` with status `503 Service Unavailable` and the same body with `"status": "error"`; the failed indicators appear in `error` with `"status": "down"` and whatever details they returned.

## Indicators

### Custom

`HealthIndicator::new(key, closure)` takes an async closure that returns `Ok(details)` if it is up and `Err(details)` if it is down; the details are a JSON object that is merged with `status`. An indicator for the database with `corvis-orm`:

```rust
#[controller("health")]
pub struct HealthController {
    health: Arc<HealthCheckService>,
    db: Arc<OrmService>,
}

#[routes]
impl HealthController {
    #[get]
    async fn check(&self) -> HttpResult<Value> {
        let db = self.db.clone();
        self.health
            .check(vec![HealthIndicator::new("database", move || {
                let db = db.clone();
                async move {
                    match db.query_raw("SELECT 1", vec![]).await {
                        Ok(_) => Ok(json!({})),
                        Err(e) => Err(json!({ "message": e.to_string() })),
                    }
                }
            })])
            .await
    }
}
```

The closure has to be `'static`, which is why the `Arc` is cloned before being moved inside.

### Built-in

| Indicator | What it checks |
|---|---|
| `corvis::HttpHealthIndicator::ping_check(key, "http://host:port/path")` | Sends an HTTP `GET` and is `up` only on a 2xx response (up to 5 redirects are followed). `https://` needs the `health-tls` feature of `corvis` |
| `corvis::HttpHealthIndicator::tcp_check(key, "host:port")` | That a TCP connection to that address can be opened. It sends no request: use it for services without HTTP, such as Redis or SMTP |
| `corvis::MemoryHealthIndicator::check_rss(key, bytes)` | That the process resident memory does not exceed the threshold. It measures on Linux, Windows and macOS; where it cannot measure, it reports `down` with a message instead of a silent `up` |

When `ping_check` fails, the entry in `error` has a `message` and, if the server answered, `statusCode` and `statusText`:

```json
{ "status": "down", "message": "Request failed with status code 500", "statusCode": 500, "statusText": "Internal Server Error" }
```

`check_rss` reports `{ "status": "down", "message": "Used rss exceeded the set threshold" }` over the threshold, and `up` with no extra fields otherwise.

::: warning Migrating from `ping_check("host:port")`
`ping_check` used to open a TCP connection only. It now takes a URL and makes an HTTP request, and a value without `http://` or `https://` is reported `down` with a message that names `tcp_check`. Replace `ping_check(key, "host:port")` with `tcp_check(key, "host:port")` to keep the old behavior.
:::

For `https://` URLs, enable the feature in `Cargo.toml`:

```toml
corvis = { version = "0.1", features = ["health-tls"] }
```

Without it, an `https://` URL is reported `down` with a message naming the feature, and no request is sent.

### Redirects

`ping_check` follows up to 5 redirects, but only within the same origin (same scheme, host and port; an `http` to `https` upgrade of the same host also counts). A redirect to another origin is not followed: the indicator is `down` with `statusCode` `302` (or the status received) and a message saying so. A redirect from `https` to `http` is never followed. This differs from the original library, which follows redirects anywhere (axios default), because a redirect is controlled by the target server and could point the health check at an internal address.

Change the behavior with `ping_check_with` and `PingOptions`:

```rust
use corvis::{HttpHealthIndicator, PingOptions};

HttpHealthIndicator::ping_check_with(
    "api",
    "https://api.example.com/health",
    PingOptions::new().max_redirects(0), // 0 = do not follow any redirect
);

HttpHealthIndicator::ping_check_with(
    "cdn",
    "https://example.com/",
    PingOptions::new().follow_cross_origin(true),
);
```

Even with `follow_cross_origin(true)`, a redirect to a loopback, link-local or private address (`127.0.0.1`, `10.x`, `192.168.x`, `169.254.x`, `localhost`, and the IPv6 equivalents) is refused unless the configured URL already pointed to one. Only IP literals and `localhost` are recognized: host names are not resolved.

Messages about invalid URLs and redirects are fixed text: the configured URL (which could contain credentials) and the `Location` header of the response are never copied into the health response.

### Exposure of details

The `down` details of an indicator go into the health response as they are: `ping_check` and `tcp_check` can include the target `host:port` and the operating system error text (for example `connect ECONNREFUSED 10.0.0.5:6379`), and custom indicators include whatever they return. If the endpoint is public, anyone can read them, so keep sensitive text out of the details or protect the route.

Each call to a health endpoint also opens outbound connections: one per `ping_check` or `tcp_check` (plus redirects), each held for up to the indicator time limit. A public endpoint that is called often can be used to generate that traffic, so rate limit it (see [Rate limiting](/security/rate-limiting)) or cache the result for a few seconds.

## Time limits

Every indicator runs with a time limit of 5 seconds. One that does not finish in time is reported `down` with `"message": "timeout of <ms>ms exceeded"`, and the other indicators are still reported, so a hung dependency cannot hang the health endpoint. Indicators run concurrently: the response time is bounded by the largest limit, not by their sum.

Change the limit of one indicator with `timeout`:

```rust
use std::time::Duration;

HttpHealthIndicator::ping_check("api", "http://127.0.0.1:8080/ready").timeout(Duration::from_millis(500))
```

An orchestrator whose probe times out sooner than 5 seconds needs a lower limit here. The default cannot be changed for the whole application.

## Limits

- There are no built-in indicators for databases, disk or specific services: write them with `HealthIndicator::new`.
