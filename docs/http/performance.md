---
status: partial
---

# HTTP adapters

Corvis has a single HTTP server, built on [hyper](https://hyper.rs) 1 and Tokio's
multithreaded runtime. There are no interchangeable adapters (there is no "on Express" variant and
another "on Fastify"): the built-in server reproduces the semantics of an Express 5 application in
what the client observes (body parser, router, `ETag`, default headers, errors) and it is the one
measured in the repository benchmark.

This page gathers what can be configured on that server.

## Protocols

- HTTP/1.1 and HTTP/2 without TLS with *prior knowledge* (h2c), enabled by default.
  `ServerOptions::new().h2c(false)` accepts HTTP/1.x only. Whether h2c stays on by default is tracked in issue
  #144; see [HTTP parsing and compatibility](/security/http-parsing).
- There is no HTTPS: terminate TLS in a reverse proxy. See [HTTPS and multiple servers](/faq/multiple-servers).
- WebSocket upgrades, with gateways or with `corvis::ws`. See [Gateways](/websockets/gateways).

## Timeouts and connections

`app.set_server_options(ServerOptions)` uses Node.js's values by default:

```rust
use std::time::Duration;
use corvis::ServerOptions;

app.set_server_options(
    ServerOptions::new()
        .headers_timeout(Duration::from_secs(30))    // 60 s by default
        .request_timeout(Duration::from_secs(120))   // 300 s by default; responds 408
        .keep_alive_timeout(Duration::from_secs(10)) // 5 s by default
        .max_connections(10_000)                     // no cap by default
        .max_header_size(16 * 1024),                 // 32 KiB by default; responds 431
);
```

Each timeout accepts `None` to disable it, for example `.request_timeout(None)`. When
`max_connections` is reached, new connections wait in the `accept` queue. Open WebSocket connections count until
they close, and so does a socket for up to 2 s after an early error response (`413`, `431`, ...) while the server
lets the client read it before closing. With a cap, give your gateways a heartbeat so dead clients do not hold slots.

## Request bodies

| Setting | Default | API |
|---|---|---|
| JSON / urlencoded parser limit | 100 kb each (`413` when exceeded) | `app.use_body_parser("json", bytes)`, `app.use_body_parser("urlencoded", bytes)` |
| Disable parsing | Enabled | `app.disable_body_parser()` |
| Transport limit for other types | 1 MiB | `app.set_body_limit(bytes)` |
| Maximum JSON depth | 256 levels (`400`) | `app.set_body_parser_max_depth(n)` |
| `req.query` parser | `simple` (as in Express 5) | `app.set_query_parser(QueryParser::Extended)` |

Only `application/json` and `application/x-www-form-urlencoded` are parsed; everything else leaves the body
unparsed and available in `req.raw_body()` (see [Raw body](/faq/raw-body)). `multipart/*`
bodies are read by a file interceptor (see [File upload](/http/file-upload)).
`QueryParser` is in `corvis::http::body_parser`.

## Responses

- Weak `ETag` on JSON, text and HTML bodies, with `304` for conditional `GET`/`HEAD`.
  `app.set_etag(EtagMode::Strong)` or `app.disable("etag")` change it (`EtagMode` is in `corvis::http`).
- `X-Powered-By: corvis`. `app.disable("x-powered-by")` removes it and
  `app.set_x_powered_by("...")` changes the value.

## Client IP and proxies

By default `req.ip()` is the socket address and `X-Forwarded-For` is ignored. Behind a reverse
proxy, configure whom you trust with the semantics of `proxy-addr`:

```rust
use corvis::TrustProxy;

app.set_trust_proxy(1);                                          // one hop
app.set_trust_proxy(TrustProxy::parse("loopback, 10.0.0.0/8")?); // addresses or subnets
app.set_trust_proxy(true);                                       // all hops
```

With that, `req.ip()` is the first address in `X-Forwarded-For` (from right to left) that
is not trusted, `req.ips()` returns the trust chain and `req.hostname()` uses
`X-Forwarded-Host` only if the socket is trusted. It also affects `#[ip]` and the
[`ThrottlerGuard`](/security/rate-limiting).

## Accessing the server from providers

`HttpAdapterHost` is injectable into any provider and gives access to the server, for example to
register listeners for the `upgrade` event. See [HTTP adapter](/faq/http-adapter).

## Performance

The repository's `bench/` directory contains a reproducible benchmark (`bash bench/run.sh`)
against an equivalent Node.js application. The results and the measurement environment are in
[Corvis versus Fastify, Express and Hono](/faq/corvis-vs-fastify-express-hono).
