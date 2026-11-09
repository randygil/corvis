# Tasks

## 1. Reference fixture

- [x] 1.1 Add `crates/corvis/tests/fixtures/compression/package.json` pinning `compression` 1.8.2,
  `@nestjs/common`, `@nestjs/core` and `@nestjs/platform-express` 12.4.0, `reflect-metadata` and `rxjs` (check that
  the installed `negotiator`, `compressible` and `mime-db` resolve to 0.6.4, 2.0.18 and 1.54.0 and record the
  versions in design.md); verify `npm install` in that folder and commit `package-lock.json` (not `node_modules`)
- [x] 1.2 Add `generate.mjs` (design.md, Decision 9): an app of the original stack on `127.0.0.1` with `app.use(compression())` and a
  second with `threshold: 0`, the routes of the spec's replay scenario (small and large JSON, text, HTML, `image/png`,
  `application/octet-stream`, `no-transform`, existing `Content-Encoding`, a static file via `useStaticAssets`, a
  stream without length, an `@Sse` route), each requested with the `Accept-Encoding` values of Decision 9 plus `HEAD`,
  a `Range` request and a conditional `GET`; record status, `Content-Encoding`, `Vary`, `Content-Length`, `Content-Range`,
  `ETag` and the SHA-256 of the decoded body in `compression.json`; also record negotiator results for a list of
  `Accept-Encoding` values and `compressible(type)` for a list of types; verify by running `node generate.mjs` once and
  committing the JSON
- [x] 1.3 Add `compressible.mjs`, which reads the installed mime-db and writes
  `crates/corvis/src/http/compression/compressible.rs` (sorted `(type, bool)` pairs of every type with a
  `compressible` flag, with a header comment naming the mime-db version and the command); verify by running it and
  checking the file has 822 entries for mime-db 1.54.0

## 2. Feature and options

- [x] 2.1 Add the `compression` feature to `crates/corvis/Cargo.toml` with optional `flate2` (default features, pure
  Rust) and `brotli`; add `flate2` and `brotli` as dev-dependencies; add `compression` to the per-feature clippy loop in
  `.github/workflows/ci.yml`; verify `cargo build -p corvis --no-default-features --features compression` and
  `cargo build -p corvis` both build, and `cargo deny check licenses` passes
- [x] 2.2 Add `CompressionOptions` (threshold, level, brotli quality, filter; clamps of design.md, Decision 7),
  `CompressionContext` and `should_compress` in `crates/corvis/src/http/compression/mod.rs`; the `compression` field
  of `HttpSettings`; `CorvisApplication::enable_compression`; exports in `crates/corvis/src/lib.rs` (and the prelude)
  under the feature; verify a unit test of the clamps and that `cargo doc -p corvis --features compression --no-deps`
  has no broken links

## 3. Decision logic (pure, unit-tested)

- [x] 3.1 Port negotiator 0.6.4's encoding negotiation (design.md, Decision 4); verify a unit test that replays the
  recorded negotiator table of `compression.json` and matches every entry
- [x] 3.2 Add the compressible lookup over the generated table with the regexp fallback, and `should_compress` with
  the `text/event-stream` exclusion; verify a unit test that replays the recorded `compressible(type)` table (the
  only expected difference is `text/event-stream` in `should_compress`)
- [x] 3.3 Add `decide(..)` with the checks in the order of design.md, Decision 3, including the divergences of
  Decision 6; verify one unit test per spec scenario that does not need a body (filter, `no-transform`, `Vary` with an
  existing value and with `*`, threshold 1023 / 1024, unknown length, existing `Content-Encoding`, `HEAD`, 204, 304,
  `Content-Range`, the negotiation scenarios)

## 4. Compressed body

- [x] 4.1 Add `CompressedBody` (design.md, Decision 2) with the gzip, zlib and brotli encoders, 64 KiB input slices,
  flush on `Pending`, `finish` at the end, error pass-through, inexact `size_hint`, and the brotli window choice of
  Decision 7; verify unit tests that decode each encoding with the dev-dependencies for an empty body, a 1-byte body,
  a 5 MiB body and a stream of many small chunks, and that a body ending in an error ends the compressed body with an
  error
- [x] 4.2 Add a test with a stream that yields one chunk and then stays `Pending`: the compressed body yields bytes that
  decode to the first chunk before the stream resumes; verify it passes
- [x] 4.3 Measure the peak memory of one encoder per encoding (default settings, 1 KiB and 5 MiB bodies) with
  `memory-stats` (already a dependency) or an allocation counter in a test, and record the figures in design.md for
  the docs; verify the brotli figure for a 1 KiB body is well under Node's 4 MiB window

## 5. Wiring

- [x] 5.1 Add `server::finish_response` (finalize + `into_hyper` + compression when enabled, design.md, Decision 1),
  keep `Accept-Encoding` in `RequestInfo` (and the request method, path and headers when a custom filter is set), and
  call it from `server::handle` and `TestRequest::send`; verify `cargo test -p corvis` passes without the feature and
  with `--features compression` (no response changes while compression is not enabled)
- [x] 5.2 Add `crates/corvis/tests/compression.rs` (gated on the feature): one integration test per spec scenario that
  needs the HTTP stack (opt-in, test client and server agree, image, no-transform, octet stream, SSE, custom filter, no
  `Accept-Encoding`, small response, existing `Vary` from CORS, threshold, stream without length, HEAD, range on a static file, already encoded,
  preference, quality, deflate in zlib format, nothing acceptable, JSON with `ETag`, large static file, slow stream
  over a listening server, out-of-range options); verify they pass with
  `cargo test -p corvis --features compression --test compression`
- [x] 5.3 Add the differential replay: declare the fixture's routes in Corvis, replay `compression.json` through the
  test client (and the static-file and stream cases through a listening server too), decode and compare as
  design.md, Decision 9 says, skipping only the cases named as divergences; verify it passes
- [x] 5.4 Make sure the feature's tests run in CI: add `--features compression` to the workspace test command or a
  dedicated `cargo test -p corvis --features compression --test compression` step in `.github/workflows/ci.yml`;
  verify the workflow file lints (`actionlint` if available) and the step appears in the job

## 6. Docs and notices

- [x] 6.1 Rewrite `docs/http/compression.md` (status `documented`): enabling (feature + `enable_compression`),
  options and defaults, when a response is compressed (the rules in order, `Vary`), encodings, streaming and SSE,
  differences from the original middleware (`text/event-stream`, `Content-Range`, no `enforceEncoding` or zlib
  tuning, no `res.flush()` needed), security (BREACH and its mitigations, CPU, the measured memory per response,
  `application/octet-stream`), and when a proxy is still the better place; keep the note that request bodies are not
  decompressed (`415`); same in `docs/es/http/compression.md`
- [x] 6.2 `docs/http/server-sent-events.md`: SSE responses are never compressed by Corvis; same in
  `docs/es/http/server-sent-events.md`
- [x] 6.3 `THIRD_PARTY_NOTICES.md`: rows for `compression` (rules and defaults) and `compressible` (type check);
  verify the existing rows for negotiator, vary and mime-db still describe what Corvis takes and extend them if not
- [x] 6.4 Run `npm run docs:check` in `docs/`; verify it passes

## 7. Definition of done

- [x] 7.1 `cargo build --workspace --all-targets` with no warnings, and the same with `-p corvis --features compression`
- [x] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature clippy loop with
  `compression` passes
- [x] 7.3 `cargo test --workspace` passes, plus `cargo test -p corvis --features compression`, including the unit tests
  of 3 and 4, the integration tests of 5.2 and the differential replay of 5.3
- [x] 7.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 7.5 Framework runtime code changed (`server::handle`, the test client): run the `corvis-parity` check and verify
  the three port apps keep 0 apidiff differences (they do not enable compression)
- [x] 7.6 `cargo deny check` passes with the new crates

## Security review fixes

- [x] 8.1 Cap the brotli window at 2^19; verify the unit test of the window
- [x] 8.2 Add `max_concurrent`, the slots held by the compressed body, the `br` to `gzip` to identity fallback and
  `Vary`; verify the unit and integration tests
- [x] 8.3 Docs (EN and ES) for the cap, the window cap and the stalled-client note; verify `npm run docs:check`

- [x] 8.4 Add the per-connection cap (`max_per_connection`, default 4) with the connection state in the request
  extensions; verify the unit tests and the h2c integration test
- [x] 8.5 Docs (EN and ES): long-lived streams hold a slot, stalled clients and the per-connection mitigation

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #118, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
