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

- The server accepts HTTP/1.1 only, like Node's `http` module. `ServerOptions::new().h2c(true)` also accepts HTTP/2
  without TLS by prior knowledge (h2c), for a proxy or platform configured to send it (Caddy `h2c://`, Traefik
  `scheme=h2c`, nginx `grpc_pass`, Cloud Run with an `h2c` port). With h2c on, one connection carries up to 100
  concurrent requests (`http2_max_concurrent_streams`), `max_header_size` also limits the HTTP/2 header list (`431`),
  and `max_connections` limits connections rather than requests. With h2c off, a prior-knowledge connection is closed without a
  response. See [HTTP parsing and compatibility](/security/http-parsing).
- HTTP/1 requests with ambiguous framing (bare LF line endings, whitespace after a chunk size, `Content-Length` with
  `Transfer-Encoding`) are answered with `400` and the connection is closed.
  `ServerOptions::new().insecure_http_parser(true)` accepts them again for legacy clients and logs a warning at
  startup; see [HTTP parsing and compatibility](/security/http-parsing).
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
        .send_timeout(Duration::from_secs(30))       // 60 s by default; closes the connection
        .max_connections(10_000)                     // no cap by default
        .max_pending_response_bytes(4 * 1024 * 1024) // 8 MiB by default, per connection; see below
        .max_pending_response_wait(Duration::from_secs(2)) // 5 s by default; see below
        .max_header_size(16 * 1024)                  // 32 KiB by default; responds 431 (HTTP/1 and HTTP/2)
        .http2_max_concurrent_streams(50)            // 100 by default, per HTTP/2 connection
        .http2_keep_alive_interval(Duration::from_secs(30)) // no HTTP/2 PINGs by default
        .http2_keep_alive_timeout(Duration::from_secs(10)), // 20 s by default
);
```

Each timeout accepts `None` to disable it, for example `.request_timeout(None)`. When
`max_connections` is reached, new connections wait in the `accept` queue. Open WebSocket connections count until
they close, and so does a socket for up to 2 s after an early error response (`413`, `431`, ...) while the server
lets the client read it before closing. With a cap, give your gateways a heartbeat so dead clients do not hold slots.

### Clients that stop reading (`send_timeout`)

`send_timeout` (60 s by default, `None` disables it) is the longest the server waits for a client to accept response
bytes. A client that stops reading would otherwise keep its connection, its `max_connections` slot and the memory of
its responses forever, because no other timer runs while a response is in flight. It works the same over HTTP/1 and
HTTP/2, in two ways:

- **Blocked socket.** When writing to the connection's socket stays blocked for `send_timeout` without the socket
  accepting a byte, the connection is closed. This also catches an HTTP/2 client that floods PINGs and never reads
  the answers, and an HTTP/1 client that pipelines requests and never reads the responses.
- **Closed flow-control windows.** When the responses of an HTTP/2 connection wait for the client to open their
  windows and none of them has moved for `send_timeout`, the connection is closed, even if the client keeps answering
  PINGs or sending SETTINGS and WINDOW_UPDATE frames. A stalled stream next to streams that keep moving does not
  close the connection: on a slow link with many streams, some of them are always waiting for window.

The connection is dropped at once: no GOAWAY, no rest of the response, no closing drain.

It measures lack of progress, not the length of the response: progress restarts it, so a slow download is not cut.
It never runs while the server has nothing to send: a slow handler, an idle Server-Sent Events stream and an idle
keep-alive connection are not affected. A value too long to add to a clock reading (`Duration::MAX`) means no limit.

Established WebSocket connections keep only the write deadline: a write that the socket accepts no byte of for
`send_timeout` fails and the connection is destroyed, without a close frame. Idle WebSockets are not affected. See
[peers that stop reading](/websockets/gateways#peers-that-stop-reading).

**What it does not bound.** It does not limit how little a client has to read to keep its connection:

- Over HTTP/2 a client has to take about 16 KiB (one frame) from some stream every `send_timeout`.
- Over HTTP/1 the server notices progress when the operating system reports the socket writable. On Linux it sets
  `TCP_NOTSENT_LOWAT` to 64 KiB on its sockets while `send_timeout` is on, so a client that reads steadily is seen
  when it takes about 32 KiB per period (without it, Linux reports the socket writable only when about half of a send buffer
  that autotuning grows to several MiB is free, and a slow reader would be cut while reading). On Windows about 64 KiB
  per period were needed on a loopback connection. On macOS and the BSDs the option is not set and the granularity is
  that of the autotuned buffer, which can be several MiB: set a larger `send_timeout` if you serve slow clients there.
- A client that resets streams and opens new ones needs to read nothing at all: every new response counts as
  movement. The rapid-reset limits bound how long that can go on.

The memory a client holds while it does that is bounded by
[`max_pending_response_bytes`](#response-memory-per-connection-max-pending-response-bytes).

Node's `server.timeout` is the nearest option: it is off by default (0) and measures inactivity in both directions,
so it also cuts slow handlers, which is why Node turned it off. With Node's defaults a client that stops reading is
never disconnected; Corvis closes it after 60 s (nginx's `send_timeout` has the same default and semantics). A
player that pauses a progressive download for over a minute reconnects (browsers resume with `Range`); raise the value
or pass `None` if you need the old behavior.

### Response memory per connection (`max_pending_response_bytes`)

A client that does not read its responses holds them in memory. `max_pending_response_bytes` (8 MiB per connection
by default, `None` turns it off) bounds that, whatever the client reads: a client that never grants HTTP/2 window,
one that grants a trickle, one that resets and reopens streams.

- **What counts.** The whole size of an in-memory response (JSON, text, bytes; also when compression wraps it, which
  counts the uncompressed size) from the moment its handler returns until its body ends or is dropped, and, for
  streamed bodies (files, Server-Sent Events), the part of the current data frame not yet handed to the protocol layer.
  Over HTTP/1 a response counts until the server has written it to the socket.
- **Admission before the handler.** Each request reserves a share of the limit (the limit divided by the requests the
  connection may run at once: `http2_max_concurrent_streams` over HTTP/2, 1 over HTTP/1, so 83,886 bytes per handler
  with the defaults). Its handler starts only when the pending bytes plus the reservations leave room, or when the
  connection holds nothing. Until then the request waits without reaching a handler; it holds its slot of
  `http2_max_concurrent_streams`, and a client that resets it meanwhile counts as having cancelled a request. The wait
  is bounded by `max_pending_response_wait` (see below).
- **Above the limit, HTTP/2 resets.** When handlers that ran together return responses that would take a connection
  that already holds pending bytes above the limit, the server drops the extra responses and resets their streams with
  `ENHANCE_YOUR_CALM`. The first reset of a connection is logged as a warning. A response that fits its share is never
  reset, and one response is always sent whatever its size. This also happens to a client that reads at full
  speed: when many large responses are ready within the same moments (a burst of slow handlers that all return
  together), the ones beyond the limit are reset even though nobody is stalled. Size the limit for what you expect
  to be in flight, at least `http2_max_concurrent_streams` times your largest usual response, and lower
  `http2_max_concurrent_streams` if your API returns responses of several megabytes; a client that gets
  `ENHANCE_YOUR_CALM` may retry.
- **HTTP/1.** Pipelined requests run one handler at a time: the next handler waits until the previous response is out.

The bound for one connection is the larger of the limit and its biggest response, plus about 64 KiB per open HTTP/2
stream that hyper still buffers after a body ended (the last slices of each response are copied, so they never keep
the whole response alive; hyper's per-stream send buffer is 32 KiB), plus the compression encoders
(`max_per_connection`). It does not include request heads and bodies, nor memory the application keeps outside the
response: an unbounded channel feeding a stream, a cache sliced into responses. Server-wide the bound is
`max_connections` x the limit (no default for `max_connections`: set one for a server exposed to the internet).

**Waiting has a limit (`max_pending_response_wait`, 5 s by default, `None` waits without limit).** An in-memory
response counts whole until it is sent, so a client that downloads one very large response slowly delays the other
requests of its connection while it does. Behind a proxy that multiplexes several users on one HTTP/2 connection
(Envoy, HAProxy with HTTP/2 backends), that would let one slow user hold the requests of the others for ever. A
request that waits longer is refused: over HTTP/2 its stream is reset with `REFUSED_STREAM` (the handler never ran,
so the proxy or client may retry it on another connection, RFC 9113 section 8.7) and it does not count as a cancelled
request for the rapid-reset rule. The first refusal of a connection is logged. The option applies to HTTP/2 only: over
HTTP/1 the wait only delays the requests that the same client pipelined behind a response it has not finished reading,
and it ends as the client reads or when `send_timeout` closes the connection; closing it sooner would truncate a
legitimate slow download.
With `None` a client that reads a trickle every `send_timeout` can keep the other streams of its connection waiting
as long as it likes: use it only when every client is trusted, or stream large bodies.

Large payloads belong in streamed bodies (static files are): a streamed body counts only the frame it holds. A
burst of in-memory responses larger than their share can lose streams to `ENHANCE_YOUR_CALM`; raise the limit if your
API returns several megabytes per response to many concurrent streams.

Node's `http2` module has the same idea in `maxSessionMemory` (10 MB): it counts queued outgoing data and rejects new
streams while it is exceeded. Node's `http` module has no limit, and a pipelined HTTP/1 handler runs concurrently there.

### HTTP/2 connections

These options apply to every HTTP/2 connection (h2c today) and never to HTTP/1 ones.

- `max_header_size` bounds the HTTP/2 header list, counted as RFC 9113 does (name and value lengths plus 32 bytes per
  field), so the same number is comparable with HTTP/1, not byte-identical. A request over it is answered `431` on
  its stream and the other streams keep working.
- `http2_max_concurrent_streams` (default 100, minimum 1) is the number of streams a client may have open at once on
  one connection.
- `max_connections` counts connections, not streams, like Node's `server.maxConnections`. The requests in flight from
  HTTP/2 clients are bounded by `max_connections` x `http2_max_concurrent_streams`.
- `headers_timeout` and `keep_alive_timeout` apply to the connection while none of its streams is in flight, so one
  long stream keeps the connection alive. An idle connection stays in the keep-alive phase when the client sends a
  frame that starts no request (WINDOW_UPDATE, PING, SETTINGS): only a new request moves it to the headers phase. `request_timeout` applies to the body of each stream on its own: a stalled
  `POST` is answered `408` without affecting the other streams.
- `http2_keep_alive_interval` (off by default) sends a PING every interval, and `http2_keep_alive_timeout` closes the
  connection when a PING is not acknowledged in time (minimum 1 s; a zero interval means no PINGs). Set it with SSE or
  other long streams over HTTP/2, where a peer that stopped answering would otherwise never be noticed.
- A client that resets streams and opens new ones faster than their handlers stop never gets more handlers running at
  once than `http2_max_concurrent_streams` (rapid reset). A connection is also closed when the requests it cancelled
  before their response (a reset stream, also while its body was being read) exceed 500 and exceed a quarter of the
  requests it completed, over the last 10 s, or exceed `http2_max_cancelled_streams` (default 2000, about 200 per
  second sustained) whatever it completed. A proxy that multiplexes many users (Envoy, Cloud Run) and sees a few
  percent of aborts is never cut: it tolerates about 20% of aborts, and 5,000 requests/s with 2% aborted stays well
  under the ceiling. An attacker pays four completed requests per reset and is capped by the ceiling. The server sends
  a graceful GOAWAY first, so the proxy can retry unanswered requests on another connection; from then on new streams
  are reset without reaching a handler, and the connection is dropped after 1 s. Raise `http2_max_cancelled_streams`
  if a proxy of yours aborts more than that. A request counts as completed only when its response body ended: a
  stream reset after its handler returned, while the body was still being sent (or before its last, possibly empty, DATA
  frame), counts as cancelled, and so does a
  response the server drops for exceeding `max_pending_response_bytes`. A request the client aborts is not logged.
- The PING keep-alive detects peers that stopped answering, not peers that stopped reading; `send_timeout` covers
  those (see above).

## Request bodies

| Setting | Default | API |
|---|---|---|
| JSON / urlencoded parser limit | 100 kb each (`413` when exceeded) | `app.use_body_parser("json", bytes)`, `app.use_body_parser("urlencoded", bytes)` |
| Disable parsing | Enabled | `app.disable_body_parser()` |
| Transport limit for other types | 1 MiB | `app.set_body_limit(bytes)` |
| Maximum JSON depth | 256 levels (`400`) | `app.set_body_parser_max_depth(n)` |
| `req.query` parser (also the query of WebSocket handshakes and `upgrade` requests) | `simple` (as in Express 5) | `app.set_query_parser(QueryParser::Extended)` |

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
[Corvis versus Fastify, Express and Hono](/faq/corvis-vs-fastify-express-hono). A real application measured
against its Node.js original is in
[Benchmark: a migrated application versus its original](/faq/port-vs-original-benchmark).
