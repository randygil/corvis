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

All indicators run in parallel. With all of them up, the response is `200`:

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
| `corvis::HttpHealthIndicator::ping_check(key, "host:port")` | That a TCP connection to that address can be opened in under 3 seconds. It does not make an HTTP request |
| `corvis::MemoryHealthIndicator::check_rss(key, bytes)` | That the process resident memory does not exceed the threshold. It reads `/proc/self/statm`, so it only measures on Linux; on other systems it always reports `up` with no data |

## Limits

- There are no built-in indicators for databases, disk or specific services: write them with `HealthIndicator::new`.
- `check` does not apply a maximum time to custom indicators. If one can hang, wrap it with `tokio::time::timeout` inside the closure.
