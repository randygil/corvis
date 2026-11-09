# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state, verified in the code at `af99b71`
(paths under `crates/corvis/` unless stated).

**Where responses are finished**

- `server::handle` (`src/server.rs:340-386`) builds `RequestInfo` from the request (`:366`), dispatches, then calls
  `express::finalize` (`:384`) and `into_hyper` (`:385`). The test client does the same at
  `src/testing.rs:380-386`, then collects the body.
- `RequestInfo` (`src/http/express.rs:47-63`) copies only `If-None-Match`, `If-Modified-Since` and `Cache-Control`
  from the request.
- `finalize` (`src/http/express.rs:233-295`) adds `X-Powered-By`, turns `Body::Json` / `Text` / `Html` (and
  `res.send(buffer)`) into `Body::Bytes` with `Content-Type`, `ETag` and `Content-Length`, answers `304` when fresh,
  and strips `Content-Type` / `Content-Length` / `Transfer-Encoding` from `204` and `304` (`:285-290`). Raw bytes and
  streams are left as they are.
- `into_hyper` (`src/server.rs:140-182`) merges `default_headers` (for example a `StreamableFile`'s `Content-Length`,
  `src/http/response.rs:565`), sets `Content-Type` from the body variant when absent (`:150-167`), sets `HEAD`'s
  `Content-Length` from the body's exact size (`:169-176`) and replaces a `HEAD` body with an empty one (`:180`).
  Before `into_hyper` a streamed body's `Content-Type` and default headers are not in `resp.headers` yet.
- `ResBody` is a boxed unsync `http_body` of `Bytes` with `io::Error`; streams become `StreamBody` frames
  (`:156-159`).
- Bodies: `Body::{Empty, Json, Text, Html, Bytes, Stream}` (`src/http/response.rs:44-52`). Files are read in 64 KiB
  chunks (`file_stream`, `:17-42`). Static files (`src/http/serve_static.rs`) set `Accept-Ranges` (`:496`),
  `Content-Length` (`:573`), a weak `ETag`, and `206` + `Content-Range` for a single range (`:559-570`); the body is a
  stream (`:577-580`). `StreamableFile` defaults to `application/octet-stream` (`response.rs:446`).
- SSE (`src/http/sse.rs:95-102`) is a `text/event-stream` stream with `Cache-Control: no-cache`, `Connection:
  keep-alive` and `X-Accel-Buffering: no`; no `no-transform`.
- `Vary` handling exists: `router::cors::vary_append` (`src/router/cors.rs:329-346`), the `vary` package's rules, also
  used by `Res::vary` (`src/http/res.rs:688-694`).
- Request bodies with any `Content-Encoding` other than `identity` are rejected with `415`
  (`src/http/body_parser.rs:108-117`); nothing in Corvis decompresses.
- No compression crate is in `Cargo.lock` (`flate2`, `brotli`, `async-compression`, `tower-http` are absent);
  `crates/corvis/Cargo.toml:50-54` has the features `jwt`, `websockets` (default) and `health-tls`; CI runs clippy per
  feature (`.github/workflows/ci.yml:63-69`). `deny.toml` allows MIT, Apache-2.0, BSD-3-Clause, Zlib and others.
- `THIRD_PARTY_NOTICES.md:35-39` already lists mime-types, mime-db, accepts, negotiator and vary.
- Docs: `docs/http/compression.md` (status `unavailable`), `docs/http/server-sent-events.md:64` (SSE headers),
  `docs/faq/raw-body.md:68` (links compression for the `415`).

**Reference (`compression` 1.8.2, the latest 1.x on 2026-11-09, read from the npm tarball, with negotiator 0.6.4,
compressible 2.0.18 and mime-db 1.54.0)**

- Defaults: `threshold` 1024 (`bytes.parse`), `filter` = `shouldCompress`, zlib options passed as they are (level
  default `Z_DEFAULT_COMPRESSION`, that is 6), brotli `BROTLI_PARAM_QUALITY` 4, `enforceEncoding` `'identity'`.
- Encodings when Node has brotli (every supported Node): `SUPPORTED = ['br', 'gzip', 'deflate', 'identity']`,
  `PREFERRED = ['br', 'gzip']`; `deflate` is `zlib.createDeflate` (zlib format).
- At `writeHead` (`onHeaders`), in this order: `filter(req, res)` false → nothing; `Cache-Control` matching
  `/(?:^|,)\s*?no-transform\s*?(?:,|$)/i` → nothing; **`vary(res, 'Accept-Encoding')`**; `Number(Content-Length) <
  threshold` or (when `res.end(chunk)` came first without a `Content-Length`) `chunk length < threshold` → no
  compression; `Content-Encoding` (default `'identity'`) `!== 'identity'` → none; `HEAD` → none; negotiator; no
  `Accept-Encoding` header → `enforceEncoding`; `!method || method === 'identity'` → none. Otherwise create the
  stream, `setHeader('Content-Encoding', method)`, `removeHeader('Content-Length')`. `ETag` and `Accept-Ranges` are
  not touched. Status is not checked: a `206` with a compressible type above the threshold is compressed with its
  `Content-Range` kept; `204` / `304` from `res.send` have no `Content-Type` (Express removes it), so the filter
  rejects them before `Vary`.
- A body written with `res.write` before `res.end` has no known length, so it is compressed whatever its size. zlib
  output is only flushed when the app calls `res.flush()`.
- `compressible(type)`: strip parameters, lowercase, look up mime-db's `compressible` flag; if absent, `/^text\/|\+
  (?:json|text|xml)$/i`, else `undefined` (falsy). mime-db 1.54.0 has 2,522 types, 687 marked compressible and 135
  marked not. Notable values: `application/json`, `text/html`, `image/svg+xml`, `application/octet-stream` → true;
  `image/png` → false; `text/event-stream` and `application/x-ndjson` → no entry (the first matches `^text/`).
- negotiator 0.6.4 `lib/encoding.js`: split on `,`, each entry `^\s*([^\s;]+)\s*(?:;(.*))?$` with `q` from the first
  `q=` parameter (`parseFloat`); `identity` is appended with the lowest listed `q` when not listed (exactly or through
  `*`); each available encoding takes the best matching entry (exact name beats `*`); entries with `q <= 0` or `NaN`
  are dropped; with a preferred list the sort is `q` descending, then preferred order, then specificity, header order.
- Original SSE (`@nestjs/core` 12.4.0, `router/sse-stream.js:96-105`) sends `Cache-Control: private, no-cache,
  no-store, must-revalidate, max-age=0, no-transform`, so the middleware never compresses it.

**Rust crates (versions available in the local registry)**

- `tower-http` 0.6.11 `CompressionLayer`: a tower `Service` layer; adds `Vary` only when it compresses
  (`compression/future.rs:43-61`), skips `Content-Range` and existing `Content-Encoding` (`:43-46`), removes
  `Accept-Ranges` (`:112`), no `no-transform` check, its own q-value parser and preference order, default predicate
  "size above 32 bytes, not images, not SSE, not gRPC". Uses `async-compression` internally.
- `async-compression` 0.4.42: async `bufread` / `write` encoders for tokio and futures-io over `compression-codecs`;
  `deflate` there is raw deflate and `zlib` is the HTTP `deflate` coding.
- `flate2` 1.1 (MIT OR Apache-2.0; default pure-Rust `miniz_oxide` backend): synchronous `write::GzEncoder` /
  `ZlibEncoder` with `flush` (sync flush) and `finish`.
- `brotli` 9.1 (BSD-3-Clause AND MIT, pure Rust): `CompressorWriter` with quality and window size, `flush`.

## Goals / Non-Goals

**Goals:**
- An app that used `app.use(compression())` gets the same headers and decoded bodies after migrating, checked by a
  differential test.
- Compression never changes what a client decodes, never delays a streamed chunk, and never applies unless the app
  opts in at build and run time.
- CPU and memory per response stay bounded and documented.

**Non-Goals:**
- Request decompression, extra encodings or zlib tuning options, a concurrency cap (proposal, Out of scope).

## Decisions

1. **A function on the wire response, not middleware or a tower layer.** `compression::apply(resp:
   hyper::Response<ResBody>, req: &RequestInfo, opts: &CompressionOptions) -> hyper::Response<ResBody>` runs right
   after `into_hyper`, where `Content-Type`, default headers and `HEAD`'s length are final. A new
   `server::finish_response(resp, &info, &settings, head)` does finalize + `into_hyper` + compression and is called by
   both `handle` and the test client, so they cannot drift.
   Alternatives considered: a `CorvisMiddleware` like `app.use(compression())` (the spike rejected it: middleware runs
   before `finalize` and sees `Body::Json` without length or `ETag`); running between `finalize` and `into_hyper` on
   `Response` (a stream's `Content-Type` and a `StreamableFile`'s `Content-Length` are not in `headers` yet, so the
   logic would duplicate `into_hyper`); `tower-http`'s `CompressionLayer` (Corvis does not run tower services, and its
   rules differ from the reference on `Vary`, `no-transform`, `Accept-Ranges`, negotiation and the content-type
   predicate, which is what the differential test checks).
2. **Encoders: `flate2` and `brotli` directly, driven by a small body wrapper.** `CompressedBody` implements
   `http_body::Body`: it polls the inner body, feeds each data frame to a synchronous encoder writing into a `Vec<u8>`
   in slices of at most 64 KiB (one slice per poll, yielding the produced bytes), flushes the encoder (sync flush) when
   the inner body returns `Pending` after input was fed since the last flush, calls `finish` at the end, and passes
   errors through. `size_hint` is never exact, so hyper uses chunked framing (HTTP/1) or no length (HTTP/2).
   `gzip` → `flate2::write::GzEncoder`, `deflate` → `flate2::write::ZlibEncoder` (zlib format, like Node), `br` →
   `brotli::CompressorWriter`.
   Alternatives considered: `async-compression` with `tokio-util`'s `StreamReader` / `ReaderStream` (more crates and
   two adapters around an already chunked body, and flushing on `Pending` is not something its `bufread` encoders
   promise); zlib through a C library (`flate2` with `zlib-ng`; a C toolchain for a size win the reference does not
   need).
3. **The reference's rules in the reference's order, as one pure function.** `decide(&RequestInfo, &Parts, known_len)
   -> Decision { add_vary, encoding }` implements, in order: filter; `no-transform`; add `Vary` with `vary_append`;
   threshold (`Content-Length` parsed as a number, else the exact body size when known; unknown → compress); existing
   `Content-Encoding` other than `identity`; `HEAD`; then the two divergences of Decision 6; then negotiation. Keeping
   it pure makes each spec scenario a unit test and the differential test a table.
4. **Negotiation ported from negotiator 0.6.4, not shared with `res.rs`.** `accepts` in `src/http/res.rs:451` ports
   negotiator 1 for MIME types; the encoding algorithm of 0.6.4 (identity fallback at the lowest `q`, preferred list,
   `parseFloat` semantics, `*`) is a separate ~80-line function with its own table test recorded from the package.
   `parseFloat` is reproduced closely enough for the table: a leading number is taken (`0.5abc` → 0.5), anything else
   is `NaN` and drops the entry.
5. **Compressible types from mime-db, generated.** A generator script in the fixture folder reads mime-db 1.54.0 and
   writes `src/http/compression/compressible.rs`: a sorted `&[(&str, bool)]` of the 822 types that have a
   `compressible` flag, looked up with a binary search, then the `^text/|\+(?:json|text|xml)$` fallback. The default
   filter adds one rule of its own: `text/event-stream` is never compressible (Decision 6). The table is checked by a
   test against the generator's output for a list of types (including parameters and mixed case).
   Alternative considered: a short hand-written list (JSON, text, JS, CSS, SVG...). Rejected: `application/octet-stream`
   and hundreds of vendor types would differ from the reference.
6. **Two deliberate divergences, both "do not compress".**
   - `text/event-stream`: the reference compresses it unless `no-transform` is set and the app calls `res.flush()`;
     the original's SSE sets `no-transform`, so in practice SSE is never compressed there. Corvis's SSE does not send
     `no-transform`, so the filter excludes the type. Since the exclusion is in the filter, no `Vary` is added, which
     is also what the original's SSE gets.
   - `Content-Range` (and bodyless statuses 1xx / 204 / 304): a compressed `206` keeps a `Content-Range` that refers to
     the uncompressed bytes, which breaks resumed downloads; tower-http and common proxies skip ranges too. The check
     sits after `Vary`, so the only visible difference from the reference is the missing `Content-Encoding` on `206`.
     Bodyless statuses normally fail the filter already (finalize removes `Content-Type`); the explicit check covers
     responses built without `finalize`'s treatment.
   Both are listed under "Differences from the original middleware" in the docs and excluded from the differential
   fixture's comparison by name.
7. **Options and defaults.** `CompressionOptions { threshold: 1024, level: 6, brotli_quality: 4, filter: None }` with
   builder setters; `level` clamped to 0..=9, `brotli_quality` to 0..=11. The brotli window is chosen per response:
   the smallest window (between 2^16 and 2^22, Node's default) that covers the body when its length is known, 2^20 for
   streams of unknown length, so a small JSON response does not allocate a 4 MiB ring buffer. The filter is
   `Arc<dyn Fn(&CompressionContext) -> bool + Send + Sync>`; `CompressionContext` exposes the request method, path and
   headers and the response status and headers; `should_compress(&CompressionContext)` is public, like
   `compression.filter`. The request headers are cloned into `RequestInfo` only when a custom filter is set;
   `Accept-Encoding` is always kept (one header value).
8. **Where the options live.** `HttpSettings` (`src/http/express.rs:31-45`) gains `#[cfg(feature = "compression")]
   compression: Option<Arc<CompressionOptions>>`; `CorvisApplication::enable_compression` sets it like `set_etag`
   (`src/application.rs:366`). The test client reads the same `shared.http_settings`, so `Test::create_testing_module`
   apps compress exactly like the server.
9. **Differential test against the original stack.** `crates/corvis/tests/fixtures/compression/` holds a `package.json`
   pinning `compression` 1.8.2, `@nestjs/common`, `@nestjs/core`, `@nestjs/platform-express` 12.4.0 (the versions of
   the throttler fixture), `reflect-metadata`, `rxjs`, and a `generate.mjs` that starts an app of the original stack (`@nestjs/core` with `@nestjs/platform-express`) on `127.0.0.1` with
   `app.use(compression())` (and a second app with `threshold: 0`), sends each request of the spec's replay scenario
   with several `Accept-Encoding` values (none, `gzip`, `deflate`, `br`, `gzip, deflate, br`, `br;q=0.5, gzip`,
   `identity`, `*`, `identity;q=0, gzip;q=0`), and records status, `Content-Encoding`, `Vary`, `Content-Length` (value
   when not compressed, presence otherwise), `ETag` and the SHA-256 of the decoded body. It also records
   `negotiator.encoding(SUPPORTED, PREFERRED)` for a list of `Accept-Encoding` values and `compressible(type)` for a list
   of types, and the generator of Decision 5 lives next to it. The Rust test declares the same routes, replays with the
   test client and one listening server, decodes with `flate2` / `brotli` (dev-dependencies), and compares; compressed
   bytes are never compared (zlib and miniz_oxide produce different streams).

## Risks / Trade-offs

- [BREACH: a response that is compressed and mixes a secret (CSRF token, session-bound value) with input the attacker
  controls lets an attacker who sees the encrypted sizes recover the secret] → compression is opt-in twice; the docs
  explain the attack and the mitigations (do not reflect input next to secrets, mask tokens per response, exclude such
  routes with `filter` or `Cache-Control: no-transform`, or compress in a proxy only for static assets).
- [CPU: any client can ask for compressed responses of the largest compressible resources] → moderate defaults (gzip 6,
  brotli 4, the reference's), the threshold, mime-db's list keeping images and archives out, and at most 64 KiB per
  compression step so one body cannot hold a worker thread; documented with rough throughput numbers measured by the
  implementer. `application/octet-stream` is compressible per mime-db, as in the reference: the docs tell apps serving
  binary downloads to set a precise type or filter them.
- [Memory: an encoder per compressed response in flight (a few hundred KiB for gzip, to be measured; brotli depends on the
  window)] → the brotli window of Decision 7; the implementer measures both and the docs give the per-response figure
  and the bound `max_connections × streams × encoder size`.
- [Compressed bytes differ from Node's] → only decoded bodies and headers are compared; clients only see decodable
  streams.
- [Strong `ETag` kept on a compressed response (`app.set_etag(Strong)`)] → same as the reference; weak tags (the
  default) are correct; documented.
- [Flushing on `Pending` costs a few bytes per flush on slow streams] → it is what makes compressed streams usable
  without a `res.flush()` API; the cost is small next to the latency it removes.
- [Size: the generated table adds about 25 KB of strings to binaries built with the feature] → only with the feature.

## Open questions

1. **Concurrency cap.** Should a later change add a cap on responses being compressed at once (sending `identity`
   beyond it) to bound CPU and memory under load, or is `max_connections` enough?
2. **Range divergence.** Confirm not compressing responses with `Content-Range` (proposed) over matching the reference,
   which compresses `206` responses.
3. **Brotli window.** Confirm the per-response window of Decision 7 over Node's fixed 2^22 (same decoded output, less
   memory per response).

## Review notes (2026-11-09)

- Q1: no concurrency cap in this change; a later change may add one.
- Q2: confirmed: responses with `Content-Range` and `text/event-stream` responses are not compressed. Both divergences
  from the reference are documented.
- Q3: confirmed: per-response brotli window.

## Implementation notes

- **Reference versions recorded** (task 1.1): `compression` 1.8.2, negotiator 0.6.4, compressible 2.0.18, mime-db 1.54.0,
  `@nestjs/*` 12.4.0 (express 5.3.0), Node v24.13.0. The generated table has 822 entries. The fixture records 220
  requests, 63 negotiation inputs and 46 types.
- **Layout.** `src/http/compression/{mod,negotiate,body,compressible,tests}.rs`. `compressible()` and `should_compress()`
  are public (`corvis::http::compression`, with `CompressionOptions`, `CompressionContext`, `should_compress` re-exported at
  the crate root and in the prelude).
- **`Vary`: its own function.** `router::cors::vary_append` re-joins the values trimmed, while the `vary` package keeps the
  original header text and appends `, field` (`Origin,Cookie` becomes `Origin,Cookie, Accept-Encoding`). Compression uses
  `compression::vary_append`, a faithful port of the package, so the header is the same byte for byte; `cors` and
  `Res::vary` are untouched. Several `Vary` header lines are joined with `, ` and replaced by one, like Node's array handling.
- **Header values.** Several `Accept-Encoding` lines are joined with `, ` (Node's `req.headers`); several
  `Cache-Control` lines are all checked for `no-transform`. `Content-Length` is read with a port of JavaScript's
  `Number()` (so `0x10` is 16 and `abc` is `NaN`, which is not below the threshold); `q` values with a port of `parseFloat`.
- **Differential replay goes through a listening server, not the test client.** The test client does not add the
  `Content-Length` of raw bodies (hyper does on the wire), so comparing `Content-Length` presence with the original needs
  the real server. The test client is covered by the scenario tests and by `test_client_and_server_agree`.
- **Fixture details.** The recorded `ETag` of static files is compared for presence only (it contains the mtime); the
  `encoded` route (a gzip body with its own `Content-Encoding`) is compared on the gunzipped body and without the exact
  `ETag` or `Content-Length` (Node's zlib and `miniz_oxide` produce different bytes). The `Range` cases are recorded
  against the original (which compresses the `206`) and asserted against the documented Corvis behavior.
- **Task 4.3, measured peak heap above the idle server** (`tests/compression_memory.rs`, a counting allocator; the
  figures include the output that the test client collects):

  | Body | gzip | deflate | brotli |
  |---|---|---|---|
  | 1 KiB | 347 KiB | 347 KiB | 559 KiB (window 2^16) |
  | 5 MiB stream, no length | 413 KiB | 413 KiB | 7.6 MiB (window 2^20) |
  | 5 MiB static file | 2.4 MiB | 2.4 MiB | 11.1 MiB (window 2^22) |

  A small brotli response is far below Node's 4 MiB window. A large brotli body of known length does need about 11 MiB
  (4 MiB window plus quality-4 tables); this is the per-response bound the docs state, and a reason for the open question
  about a concurrency cap.
- **Throughput** (one core, release, 8 MiB of JSON rows, encoders only): gzip 1 / 6 / 9 at 640 / 84 / 46 MiB/s, brotli
  quality 0 / 4 / 6 / 11 at 694 / 131 / 65 / 0.9 MiB/s. Used in the docs.
- **CI.** `compression` is in the per-feature clippy loop, and the Linux test job runs
  `cargo test -p corvis --features compression`. `actionlint` is not installed here, so the workflow was not linted.
- **Not done: 7.5.** The parity check is run by the maintainer's session, as the implementation brief says.
- **`cargo deny check`** passes (advisories, bans, licenses, sources); the added crates are `flate2`, `miniz_oxide`,
  `crc32fast`, `adler2`, `simd-adler32`, `brotli`, `brotli-decompressor`, `alloc-no-stdlib`, `alloc-stdlib`. Cargo.lock also
  lists `zlib-rs` (an optional backend of `flate2`, not compiled). No new license was needed.
