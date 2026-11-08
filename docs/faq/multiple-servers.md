---
status: partial
---

# HTTPS and multiple servers

## HTTPS

Corvis does not terminate TLS: the server only accepts unencrypted HTTP/1.1 (HTTP/2 with `h2c(true)`). There are no options
to load a certificate and a key into the application.

The recommended way to serve HTTPS is to put a reverse proxy in front (nginx, Caddy, Traefik, a cloud
load balancer) that terminates TLS and forwards to the application over HTTP on the internal network. In that
case:

- Listen only on the internal interface or on `127.0.0.1`, for example
  `app.listen("127.0.0.1:3000").await`. Careful: `app.listen(3000)` (a number) listens on `0.0.0.0`.
- Configure `trust proxy` so that `req.ip()`, `req.ips()` and `req.hostname()` read the proxy's
  `X-Forwarded-*` headers: `app.set_trust_proxy(1)`. See [HTTP adapters](/http/performance).
- Set `Strict-Transport-Security` in the proxy or with a middleware. See
  [Security headers](/security/helmet).

## Listening on multiple ports

`listen` consumes the application and blocks until it finishes, but
`listen_in_background(addr)` starts a server in the background and returns the address it is
listening on. You can combine them to serve the same application on two addresses:

```rust
#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.enable_shutdown_hooks();
    let internal = app.listen_in_background("127.0.0.1:9000").await?;
    Logger::new("Bootstrap").log(format!("Internal listener on {internal}"));
    app.listen("0.0.0.0:3000").await
}
```

Both servers share routes, providers and configuration (initialized only once). Keep in
mind that:

- The `set_server_options` options apply to both.
- With `enable_shutdown_hooks`, on receiving the signal the `listen` server is stopped and the
  shutdown hooks run; the `listen_in_background` one is not drained: it ends when the process ends.
- `app.get_url()` returns the address of the last server that started listening.

`listen_in_background("127.0.0.1:0")` picks a free port, which is also useful in
*end-to-end* tests.

There is no way to serve different routes on each port: if you need that, create two applications with
different root modules.
