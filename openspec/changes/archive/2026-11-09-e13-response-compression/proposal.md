# Proposal

Implements [randygil/corvis#118](https://github.com/randygil/corvis/issues/118) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Design notes come from the #23 spike, feature 4
(`openspec/changes/archive/2026-11-08-e13-missing-core-features-users-expect/design.md`, and its review notes: the
latest 1.x of the `compression` middleware is the reference; `compression` stays out of the default features).

## Why

Corvis never compresses responses (`docs/http/compression.md` has status `unavailable` and recommends the reverse
proxy). Applications built on the original stack commonly call `app.use(compression())`; when they migrate, their
JSON and static assets go out uncompressed, and an app deployed without a compressing proxy pays for it in bandwidth
and latency. Matching the middleware's rules matters beyond size: `Vary`, `Content-Length` and `Content-Encoding`
decide what caches and clients store.

## What Changes

- A `compression` cargo feature, **off by default**, adding `flate2` (pure-Rust backend) and `brotli`.
- `app.enable_compression(CompressionOptions)`; compression stays off unless it is called. `CompressionOptions`
  has `threshold(bytes)` (default 1024), `level(0..=9)` for gzip and deflate (default 6, zlib's), `brotli_quality(0..=11)`
  (default 4, the reference's) and `filter(fn)` with `should_compress` as the default filter, like
  `compression({ filter })` and `compression.filter`.
- Applied after the Express response defaults (`express::finalize`) and the conversion to the wire response, in the
  server and in the test client, so it sees the final status, headers and body.
- Same rules as `compression` 1.8.2, in its order: compressible `Content-Type` per mime-db (the data `compressible`
  uses), `Cache-Control: no-transform`, then `Vary: Accept-Encoding` on every response that passed those two checks,
  then the 1 KiB threshold (from `Content-Length` or the known body length; streams of unknown length are compressed),
  an existing `Content-Encoding`, `HEAD`, and `Accept-Encoding` negotiation as negotiator 0.6.4 does it (q-values,
  `*`, `identity`, `br` then `gzip` preferred on ties, no compression without the header). Compressed responses get
  `Content-Encoding` and lose `Content-Length`; `ETag` and `Accept-Ranges` are kept, as in the reference.
- Encodings: `br`, `gzip`, `deflate` (the zlib format, as Node's `createDeflate`).
- A cap on compressions in progress (`max_concurrent`, default 8 times the available parallelism, `br` limited to half
  of the slots, falling back from `br` to `gzip` and then to no compression, `Vary` always set) and a brotli window of
  at most 2^19 (a divergence from Node's 2^22), added after the security review: a client that stops reading holds its
  compressor until the connection closes.
- Streamed bodies (static files, `StreamableFile`, `Response::stream`) are compressed as they are read, in slices of
  at most 64 KiB, and the encoder is flushed whenever the source has nothing ready, so a slow stream is never held
  back.
- **Deliberate divergences** (documented): `text/event-stream` is never compressed (the original's SSE avoids it with
  `Cache-Control: no-transform`, which Corvis's SSE does not send); responses with `Content-Range` (`206`, `416`) and
  bodyless statuses are never compressed (the reference compresses a `206`, which corrupts ranged downloads).
- A differential test against `compression` 1.8.2 on the original stack (`@nestjs/platform-express` with `app.use(compression())`):
  the same requests to both, comparing status, `Content-Encoding`, `Vary`, presence of `Content-Length`, `ETag`, and the
  decoded body; plus exact unit-level comparisons of the encoding negotiation and the compressible-type check.
- Docs (EN + ES): the compression page (status `documented`), with BREACH, CPU and memory notes; a note on the SSE page;
  third-party notices for `compression` and `compressible`.

## Capabilities

### New Capabilities

- `http/compression`: when and how responses are compressed, negotiation, headers, streaming, the differences from the
  original middleware, and the opt-in.

### Modified Capabilities

None.

## Impact

- New `crates/corvis/src/http/compression.rs` (or a `compression/` folder with the generated compressible table), behind
  `#[cfg(feature = "compression")]`.
- `crates/corvis/Cargo.toml`: the `compression` feature and the optional `flate2` / `brotli` dependencies; dev-dependencies
  `flate2` and `brotli` for the tests to decode.
- `crates/corvis/src/http/express.rs`: `HttpSettings` gains the compression options; `RequestInfo` keeps
  `Accept-Encoding` (and, when a custom filter is set, the request method, path and headers).
- `crates/corvis/src/server.rs` (`handle`) and `crates/corvis/src/testing.rs` (`TestRequest::send`): one shared
  function runs finalize, the wire conversion and compression.
- `crates/corvis/src/application.rs`: `enable_compression`.
- `crates/corvis/src/lib.rs`: exports under the feature.
- `.github/workflows/ci.yml`: `compression` added to the per-feature clippy loop.
- `THIRD_PARTY_NOTICES.md`: `compression` and `compressible` rows (negotiator, vary and mime-db are already listed).
- Fixture `crates/corvis/tests/fixtures/compression/` and tests `crates/corvis/tests/compression.rs`.
- Docs: `docs/http/compression.md`, `docs/http/server-sent-events.md` and their copies under `docs/es/`.
- `cargo deny`: the new crates' licenses (`flate2` MIT OR Apache-2.0, `miniz_oxide`, `crc32fast`, `adler2`, `brotli`
  BSD-3-Clause AND MIT, `brotli-decompressor`, `alloc-no-stdlib`, `alloc-stdlib`) are in the allow list; checked by the
  implementer.
- Port apps: compression is opt-in and none of them uses it (#23 spike), and the default build does not include it, so
  apidiff should not move; the parity check confirms it.

## Out of scope

- Decompressing request bodies: still `415 unsupported content encoding`
  (`crates/corvis/src/http/body_parser.rs:108-117`); it needs its own limits against decompression bombs.
- `zstd` and other encodings the reference does not offer.
- `enforceEncoding`, `chunkSize`, `memLevel`, `windowBits`, `strategy` and custom brotli `params`: possible new issue
  if a migrated app needs one.
- A `res.flush()` equivalent: the flush-when-idle rule makes it unnecessary for Corvis bodies.
- A send timeout for clients that stop reading (#157): until then such a client keeps its compressor until the
  connection closes, which the cap on concurrent compressions bounds.
- Sending `Cache-Control: no-transform` from SSE like the original's SSE does: a separate parity item for the SSE
  headers; possible new issue.
