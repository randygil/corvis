---
status: documented
---

# Compression

Corvis compresses responses with `br`, `gzip` or `deflate` when you turn it on, with the same rules as the
`compression` middleware (1.8.2): the same headers, the same decisions, the same decoded bodies. It is **off**
twice over: the code is behind the `compression` cargo feature, and nothing is compressed until the application
calls `enable_compression`. Without both, responses are exactly what they were.

Corvis does not decompress requests: a body with `Content-Encoding: gzip`, `deflate` or `br` is still rejected with
`415 unsupported content encoding` (see [Raw body](/faq/raw-body)).

## Enabling it

```toml
[dependencies]
corvis = { version = "0.1", features = ["compression"] }
```

```rust
use corvis::prelude::*;

let mut app = CorvisFactory::create::<AppModule>().await?;
app.enable_compression(CompressionOptions::new());
app.listen("127.0.0.1:3000").await?;
```

`enable_compression` plays the role of `app.use(compression())`. The test client (`corvis::testing`) goes through the
same code as the server, so a test application compresses exactly like the real one.

## Options

| Method | Default | `compression` option |
|---|---|---|
| `.threshold(bytes)` | 1024 | `threshold` |
| `.level(0..=9)` | 6 | `level` (gzip and deflate) |
| `.brotli_quality(0..=11)` | 4 | `brotli.params[BROTLI_PARAM_QUALITY]` |
| `.filter(\|ctx\| ...)` | `should_compress` | `filter` |

Values out of range are clamped (`level(20)` is 9, `brotli_quality(50)` is 11).

`filter` receives a `CompressionContext` with the request method, path and headers and the response status and
headers. `corvis::should_compress` is the default filter and is public, like `compression.filter`, so a custom filter can
fall back to it:

```rust
app.enable_compression(
    CompressionOptions::new()
        .threshold(2048)
        .filter(|ctx| !ctx.request_headers.contains_key("x-no-compression") && should_compress(ctx)),
);
```

The request headers are copied for the filter only when you set one.

## When a response is compressed

The checks run in this order, as in the middleware:

1. **Filter.** The default one accepts a `Content-Type` that mime-db 1.54.0 marks as compressible (JSON, HTML, text,
   JavaScript, CSS, SVG, XML, `application/octet-stream` and hundreds more) and, for types it has no flag for, `text/*`
   and the `+json`, `+text` and `+xml` suffixes. Images, archives, fonts and video are left alone. A response without
   `Content-Type` is not compressed.
2. **`Cache-Control: no-transform`** (as a comma-separated token, case-insensitive).
3. **`Vary: Accept-Encoding`** is added to every response that passed the first two checks, whether or not it ends up
   compressed, keeping existing values (`Vary: Origin` becomes `Origin, Accept-Encoding`) and leaving `Vary: *`.
4. **Threshold.** A `Content-Length` below the threshold, or without that header a body whose size is known and below
   it, is not compressed. A streamed body of unknown length is compressed whatever its size.
5. **Existing `Content-Encoding`** other than `identity`.
6. **`HEAD`.** A `HEAD` response keeps the `Content-Length` the `GET` would have had.
7. **Ranges and bodyless statuses** (see [differences](#differences-from-the-original-middleware)).
8. **Negotiation.** The encoding comes from `Accept-Encoding` the way negotiator 0.6.4 does it: `q` values, `*`,
   `identity` acceptable at the lowest listed quality, `q=0` excluded, and `br`, then `gzip`, then `deflate` on ties.
   Without the header, or when nothing else is acceptable, the response is sent as it is (never a `406`).

A compressed response gets `Content-Encoding` and loses `Content-Length`. Its status, `ETag`, `Accept-Ranges` and
the other headers are kept, as in the middleware.

`deflate` is the zlib format (RFC 1950), the one browsers expect and the one Node's `createDeflate` produces.

The compression runs on the final response, after `X-Powered-By`, the `ETag`, the `304` handling and the headers
set by middleware, interceptors, exception filters and `#[header]`.

## Streams and Server-Sent Events

Streamed bodies (static files, `StreamableFile`, `Response::stream`) are compressed as they are read. At most
64 KiB goes through the encoder in one step, and the task yields to the runtime between steps, so one large body
does not hold a worker thread. When the source has nothing ready, the encoder is flushed: a slow stream is never held
back by the compressor's buffers. You do not need anything like `res.flush()`. The cost is a few bytes per flush on
very slow streams.

`text/event-stream` is never compressed. See [Server-Sent Events](/http/server-sent-events).

## Differences from the original middleware

- **`text/event-stream` is never compressed.** The middleware compresses it unless `Cache-Control: no-transform` is
  set and the app calls `res.flush()`; the original SSE implementation sends `no-transform`, so in practice SSE is not
  compressed there either. Corvis's `Sse` does not send `no-transform`, so the filter excludes the type. No `Vary` is
  added, which is also what the original's SSE gets.
- **Responses with `Content-Range` (`206`, `416`) and bodyless statuses (1xx, `204`, `304`) are never compressed.** The
  middleware compresses a `206` and keeps a `Content-Range` that refers to the uncompressed bytes, which corrupts
  resumed downloads. `Vary: Accept-Encoding` is still added, so the only visible difference is the missing
  `Content-Encoding` on a `206`.
- Not available: `enforceEncoding`, `chunkSize`, `memLevel`, `windowBits`, `strategy` and custom brotli `params`;
  `res.flush()` is not needed.
- The brotli window is chosen per response: the smallest from 64 KiB to 4 MiB (Node's fixed size) that covers the body,
  1 MiB for a stream of unknown length. The decoded output is the same; a small response does not allocate a 4 MiB
  window.
- The compressed bytes are not the ones Node's zlib produces, only equivalent: clients decode both the same.

## Security

**BREACH.** A response that is compressed and mixes a secret (a CSRF token, a session-bound value) with text the attacker
can influence lets an attacker who watches the encrypted sizes recover the secret byte by byte. Compression is opt-in
for this reason. To reduce the risk:

- do not reflect request input next to secrets in the same response;
- mask tokens with a fresh random value per response;
- exclude those routes with `filter` or by sending `Cache-Control: no-transform`;
- or compress only static assets, in a proxy.

**CPU.** Any client can ask for compressed responses of the largest compressible resources. The defaults are the
middleware's (gzip 6, brotli 4); the threshold and the type list keep small bodies, images and archives out; and one
compression step handles at most 64 KiB. Rough throughput of the encoders on JSON, measured on one core of a
development machine in a release build:

| Encoding | Speed | Compressed size |
|---|---|---|
| gzip level 1 | about 640 MiB/s | 17% |
| gzip level 6 (default) | about 85 MiB/s | 12.6% |
| gzip level 9 | about 45 MiB/s | 12.6% |
| brotli quality 4 (default) | about 130 MiB/s | 13.6% |
| brotli quality 11 | about 1 MiB/s | 5.1% |

Do not raise `brotli_quality` above 6 for dynamic responses.

`application/octet-stream` is compressible according to mime-db, as in the middleware. If you serve binary downloads,
give them a precise type or exclude them with `filter`.

**Memory.** Each compressed response in flight holds one encoder. Peak heap use above the idle server, measured
with `crates/corvis/tests/compression_memory.rs`:

| Body | gzip / deflate | brotli |
|---|---|---|
| 1 KiB | about 350 KiB | about 560 KiB |
| 5 MiB stream of unknown length | about 410 KiB | about 7.6 MiB (1 MiB window) |
| 5 MiB file of known length | about 2.4 MiB* | about 11 MiB (4 MiB window) |

\* Includes the compressed output that the test client collects; the server streams it.

Plan for `max_connections` times the streams per connection times these figures. Corvis does not cap concurrent
compressions in this version; set `max_connections` accordingly or compress large brotli bodies in a proxy.

**Strong ETags.** With `app.set_etag(EtagMode::Strong)` the `ETag` of the uncompressed body is kept on the compressed
one, as in the middleware. The default weak `ETag` is correct for both.

## When a proxy is still better

If you already run nginx, Caddy, Traefik, a load balancer or a CDN in front of the application, compressing there does
not use the CPU of the threads that serve requests, and you configure it once for all services:

```nginx
gzip on;
gzip_types application/json text/plain text/css application/javascript;
gzip_min_length 1024;
```

Do not compress [Server-Sent Events](/http/server-sent-events) in the proxy either: it could accumulate events in its
buffer.
