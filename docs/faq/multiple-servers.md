---
status: documented
---

# HTTPS and multiple servers

## HTTPS

Corvis can terminate TLS itself with the `tls` cargo feature (off by default, built on rustls, no OpenSSL):

```toml
corvis = { version = "0.1", features = ["tls"] }
```

```rust
use corvis::tls::TlsOptions;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let app = CorvisFactory::create::<AppModule>().await?;
    // Errors in the files (missing, encrypted key, key of another certificate) surface here, before the app starts.
    let tls = TlsOptions::from_pem_files("cert.pem", "key.pem")?;
    app.listen_tls("0.0.0.0:3443", tls).await
}
```

`listen_tls(addr, tls)` and `listen_tls_in_background(addr, tls)` work like `listen` and `listen_in_background`: the
application is initialized (including the route posture check) before the port opens, and the same routes, middleware
and gateways are served. `ServerOptions` (`max_connections`, timeouts, `max_header_size`, the HTTP/1 framing check,
`send_timeout`, `max_pending_response_bytes`, the HTTP/2 limits) apply to HTTPS connections as to plain ones.
`app.get_url()` returns an `https://` URL when the last listener started is a TLS one, and WebSocket gateways answer on
`wss://`.

### Building the options

- `TlsOptions::from_pem_files(cert_chain, key)` and `TlsOptions::from_pem(cert_chain_bytes, key_bytes)`: a PEM
  certificate chain (leaf first, then intermediates) and exactly one private key (PKCS#8, PKCS#1 or SEC1). Encrypted
  keys are rejected with a message that says to decrypt them first (`openssl pkcs8 -in key.pem -out key-decrypted.pem`).
  Errors name files, never key material.
- `TlsOptions::from_server_config(config)`: a ready `rustls::ServerConfig` (`corvis::tls::rustls` re-exports the version
  Corvis uses). Use it for several certificates chosen by server name (`ResolvesServerCertUsingSni`), client
  certificates, a resolver that reloads files, or TLS 1.3 only. Corvis applies its hardening to a copy.
- `.handshake_timeout(Duration)` and `.http2(bool)`.

### Defaults

- TLS 1.2 and 1.3 only, rustls' default cipher suites (AEAD with forward secrecy), no client certificates, the same
  certificate for every server name (or none).
- 0-RTT early data is always refused, also with a configuration of your own: replayed requests are not safe for
  handlers that are not idempotent.
- ALPN offers `h2` and `http/1.1`. A connection that negotiates `h2` is served as HTTP/2 only; one that negotiates
  `http/1.1` (or nothing) is served as HTTP/1.x only, with the HTTP/1 framing check, and the HTTP/2 preface is refused
  even with `h2c(true)`. `.http2(false)` offers only `http/1.1`, which is what an Express HTTPS server does.
- The handshake runs in the connection's own task, limited to **10 seconds** (Node's `https` waits 120): a client that
  stalls its handshake cannot hold a connection slot for long, and connections in the handshake count against
  `max_connections`. A failed handshake is closed without a response and is not logged.
- The cryptography provider is the process-wide rustls default when the application installed one (for example
  `aws-lc-rs`, for FIPS), and `ring` otherwise.

### Certificates

Files are read once, when the options are built: renewing a certificate needs a restart. A custom
`ResolvesServerCert` passed through `from_server_config` can reload certificates without one. Client certificates (mTLS)
are possible through `from_server_config`, but handlers cannot read the peer certificate yet.

### HTTPS on 443 plus an internal HTTP port

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.enable_shutdown_hooks();
app.listen_in_background("127.0.0.1:9000").await?; // plain, internal
let tls = TlsOptions::from_pem_files("cert.pem", "key.pem")?;
app.listen_tls("0.0.0.0:443", tls).await
```

### When a reverse proxy is still the better choice

Put nginx, Caddy, Traefik or a cloud load balancer in front when you need automatic certificates (ACME), certificate
reloads without restarts, HTTP/3, one port shared by several applications, or a WAF. In that case:

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
