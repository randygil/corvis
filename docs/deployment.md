---
status: documented
---

# Deployment

A Corvis application is deployed as a single native binary. There is no runtime to install on the server: you build in release mode, copy the executable along with the files it reads at runtime (`.env`, static folders, migrations), and run it.

## Building for production

```bash
corvis build --release
# equivalent to: cargo build --release
```

The binary ends up in `target/release/<crate-name>` (`.exe` on Windows). Build on the same platform as the target (or in a container with the same one) to avoid system library compatibility problems.

Because Corvis is not published on crates.io yet, `Cargo.toml` uses a path dependency (`corvis = { path = "..." }`). Make sure that path exists on the machine or image where you build; in a `Dockerfile`, copy the framework code into the build context.

## Configuration

Read configuration from the environment with `ConfigModule` instead of hard-coding it:

```rust
#[module(
    imports = [ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() })],
    /* ... */
)]
pub struct AppModule;
```

`ConfigModule` loads `.env` if it exists and then the process variables. In production it is usual to pass the variables from the orchestrator (systemd, Docker, Kubernetes) and not deploy the `.env`. To fail at startup if something is missing, use the `validate` option or, as in `examples/tertulia`, return an error from the factory of an async module:

```rust
JwtModule::register_async(true, |ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let secret = config.get::<String>("JWT_SECRET").filter(|s| !s.trim().is_empty());
    let secret = secret.ok_or_else(|| corvis::anyhow::anyhow!("JWT_SECRET is required"))?;
    Ok(JwtModuleOptions::new(secret))
}),
```

See [Configuration](/application/configuration).

## Address and port

```rust
let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
app.listen(port).await
```

| `listen` argument | Address |
|---|---|
| `3000` (a number) | `0.0.0.0:3000`: all interfaces |
| `"127.0.0.1:3000"` | Local only |
| `"0.0.0.0:3000"`, `"[::]:3000"` | All IPv4 or IPv6 interfaces |
| A `SocketAddr` | As is |

Inside a container you need to listen on all interfaces (`0.0.0.0`), not on `127.0.0.1`.

## Behind a reverse proxy

Corvis serves unencrypted HTTP/1.1 (HTTP/2 with `h2c(true)`); it **does not terminate TLS**. In production, put it behind a reverse proxy or load balancer (nginx, Caddy, Traefik, your provider's) that handles HTTPS.

Behind a proxy, the socket IP is the proxy's. For `req.ip()`, `#[ip]`, and the `ThrottlerGuard` to see the client's, say whom to trust:

```rust
use corvis::TrustProxy;

app.set_trust_proxy(1usize);                                         // one proxy in front
app.set_trust_proxy(TrustProxy::parse("loopback, 10.0.0.0/8")?);    // addresses or subnets
```

Lists accept IPs, CIDR ranges (`fc00::/7`), masks (`10.0.0.0/255.0.0.0`) and the names `loopback`, `linklocal` and `uniquelocal`. `X-Forwarded-For` is read from right to left starting at the socket: the first untrusted address is the client's. Don't use `set_trust_proxy(true)` unless **all** requests go through a proxy that rewrites `X-Forwarded-For`: otherwise any client can choose its IP. See [Rate limiting](/security/rate-limiting).

## Server limits

The defaults follow those of Node.js and protect against slow clients:

```rust
use std::time::Duration;
use corvis::ServerOptions;

app.set_server_options(
    ServerOptions::new()
        .headers_timeout(Duration::from_secs(30))    // default 60 s
        .request_timeout(Duration::from_secs(120))   // default 300 s (responds 408)
        .keep_alive_timeout(Duration::from_secs(10)) // default 5 s
        .send_timeout(Duration::from_secs(30))       // default 60 s (closes a client that stops reading)
        .max_connections(10_000)                     // default: no cap
        .max_pending_response_bytes(4 * 1024 * 1024) // default 8 MiB of unread responses per connection
        .max_pending_response_wait(Duration::from_secs(2)) // default 5 s a request waits for room (then refused)
        .max_header_size(16 * 1024),                 // default 32 KiB (responds 431; HTTP/1 and HTTP/2)
);
```

Each timeout accepts `None` to disable it (`.request_timeout(None)`). Connections beyond `max_connections` wait in the `accept` queue. An HTTP/1 request head is limited to 32 KiB (Node uses 16 KiB). A connection that sends nothing or drips its headers is closed, while requests in progress (a slow handler, an SSE response) do not count as idle. `send_timeout` closes a connection whose client accepts no response byte for that long (it never runs while the server has nothing to send); see [Performance](/http/performance#clients-that-stop-reading-send-timeout). `max_pending_response_bytes` bounds the response memory one connection holds for a client that does not read; see [Performance](/http/performance#response-memory-per-connection-max-pending-response-bytes). Behind a proxy, `max_connections` times that value bounds the server.

`ServerOptions::h2c(true)` enables HTTP/2 without TLS if your proxy is configured to send it (off by default). HTTP/2 connections carry up to 100 concurrent streams (`http2_max_concurrent_streams`), `max_connections` counts connections rather than streams, and `http2_keep_alive_interval` turns on PING keep-alive for long streams; see [Performance](/http/performance). Call `set_server_options` before `listen`. See [Keep-alive connections](/faq/keep-alive-connections).

## Graceful shutdown

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.enable_shutdown_hooks();
app.listen(port).await
```

With `enable_shutdown_hooks()`, on receiving `SIGINT` (Ctrl+C) or `SIGTERM` (on Unix) the server stops accepting connections, waits up to 10 seconds for in-flight requests to finish, and runs `on_module_destroy`, `before_application_shutdown`, and `on_application_shutdown` (see [Lifecycle events](/fundamentals/lifecycle-events)). This is what orchestrators expect when replacing an instance. On Windows only Ctrl+C is listened for.

## Health checks

Expose an endpoint so the orchestrator knows whether the instance is alive, with `TerminusModule` and `HealthCheckService`:

```rust
use corvis::MemoryHealthIndicator;

#[controller("health")]
pub struct HealthController {
    health: Arc<HealthCheckService>,
}

#[routes]
impl HealthController {
    #[get]
    async fn check(&self) -> HttpResult<Value> {
        self.health
            .check(vec![
                HealthIndicator::new("database", || async { Ok(json!({})) }),
                MemoryHealthIndicator::check_rss("memory_rss", 300 * 1024 * 1024),
            ])
            .await
    }
}

#[module(imports = [TerminusModule], controllers = [HealthController])]
pub struct HealthModule;
```

Every indicator has a 5 second time limit, so a hung dependency is reported `down` instead of hanging the endpoint, and `check_rss` measures on Linux, Windows and macOS.

See [Health checks](/reliability/terminus).

## Logs

The default logger writes to the console with colors. In production, if an aggregator processes the logs, the JSON format is preferable:

```rust
use corvis::ConsoleLogger;

app.use_logger(ConsoleLogger::new().json(true));
```

See [Logging](/application/logger).

## Containers

A typical two-stage `Dockerfile` (adjust the paths to the framework's location in your build context):

```dockerfile
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p my-api

FROM debian:bookworm-slim
COPY --from=build /src/target/release/my-api /usr/local/bin/my-api
ENV PORT=3000
EXPOSE 3000
CMD ["my-api"]
```

Corvis applications require the Rust 2024 edition (Rust 1.85 or later). If you use `corvis-orm` with PostgreSQL and TLS, the default `tls-rustls` feature does not need OpenSSL in the final image.

## Checklist

- Built with `--release`.
- Environment variables defined and validated at startup.
- `enable_shutdown_hooks()` enabled.
- TLS terminated at the proxy and `set_trust_proxy` configured if there is a proxy.
- Swagger routes, static files, and plain `corvis::ws` listeners protected separately if they are not public: global guards don't cover them. Gateways do get the global guards: mark public gateways explicitly.
- A health check endpoint.
