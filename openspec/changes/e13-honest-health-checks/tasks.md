# Tasks

## 1. Time limit per indicator

- [x] 1.1 Add `timeout: Duration` to `HealthIndicator` (`crates/corvis/src/modules/health.rs`), a
  `DEFAULT_INDICATOR_TIMEOUT` constant of 5 s and a `timeout(self, Duration) -> Self` builder; wrap each indicator in
  `tokio::time::timeout` inside `HealthCheckService::check`, reporting `"timeout of <ms>ms exceeded"` on expiry; verify
  with a new `crates/corvis/tests/health.rs` test: a never-completing indicator with `.timeout(100ms)` plus an `Ok`
  indicator gives a 503 with the expected `error` and `info`, in well under a second
- [x] 1.2 Add a test for the default limit using `tokio::time::pause()` / `advance` so it does not wait 5 real seconds;
  verify `cargo test -p corvis --test health`

## 2. TCP check under its own name

- [x] 2.1 Add `HttpHealthIndicator::tcp_check(key, addr)` with the current TCP-connect behavior (without the fixed 3 s
  limit, now covered by 1.1) and re-export nothing new beyond the method; verify with tests against a bound
  `TcpListener` on `127.0.0.1:0` (up) and a port freed right after binding (down)

## 3. HTTP ping check

- [x] 3.1 Enable hyper's `client` feature in `crates/corvis/Cargo.toml`; implement `ping_check(key, url)` with
  `hyper::client::conn::http1::handshake`: parse the URL with `url::Url`, reject missing/unsupported schemes with the
  messages from the spec, send `GET` with `Host` and `User-Agent` headers, `up` only for 2xx, down details `message`,
  `statusCode`, `statusText`; verify with tests against local servers bound to `127.0.0.1:0` answering 200 and 500,
  a closed port and a `host:port` argument
- [x] 3.2 Follow up to 5 redirects (relative and absolute `Location`), failing with a message after the fifth; verify
  with a test server answering `302` → `200` and one that redirects forever
- [x] 3.3 Add the optional `health-tls` feature (`tokio-rustls`, `webpki-roots`) for `https://`; without it, an `https`
  URL reports `down` naming the feature; verify both with `cargo test -p corvis --test health` and
  `cargo test -p corvis --test health --features health-tls` (the TLS test uses a local server with a self-signed
  certificate added to the root store through a test-only constructor, or is limited to the "feature disabled" case
  if that needs too much setup; record the choice here)

## 4. RSS on every platform

- [x] 4.1 Replace `rss_bytes()` with `memory_stats::memory_stats()` (physical memory); report `down` with
  `"rss could not be measured on this platform"` when it returns `None`; verify with tests for a `u64::MAX` threshold
  (up) and a threshold of 1 (down with `"Used rss exceeded the set threshold"`) on the current OS
- [x] 4.2 Run `cargo deny check` and confirm the new dependency passes licenses and advisories

## 5. Differential fixture against @nestjs/terminus

- [ ] 5.1 Add `crates/corvis/tests/fixtures/terminus/` with a `package.json` pinning `@nestjs/terminus`,
  `@nestjs/core`, `@nestjs/common`, `@nestjs/axios`, `axios`, and a `generate.mjs` that runs the scenarios of the
  spec (200, 500, 302→200, closed port, RSS huge and tiny thresholds) and writes `terminus.json`; verify by running
  `node generate.mjs` once and committing the JSON (not `node_modules`)
- [ ] 5.2 Add a Rust test that replays the same scenarios and compares with `terminus.json` (exact keys and status
  values; OS-dependent message text compared by presence); settle the `rss` key question from design.md and record
  the decision here; verify `cargo test -p corvis --test health`

## 6. Docs and example

- [x] 6.1 Update `docs/reliability/terminus.md`: built-in table (`ping_check` HTTP with URL, `tcp_check`, `check_rss`
  on Linux/Windows/macOS), a "Time limits" section with `.timeout()`, remove the obsolete "Limits" bullets, a note on
  migrating `ping_check("host:port")` to `tcp_check`; update the health snippet in `docs/deployment.md` if needed;
  verify the snippets match the final API
- [x] 6.2 Translate the same changes in `docs/es/reliability/terminus.md` and `docs/es/deployment.md`; verify
  `npm run docs:check` in `docs/`
- [x] 6.3 Update the module doc comment at the top of `health.rs`; verify `cargo doc -p corvis --no-deps` has no
  warnings for the module

## 7. Definition of done

- [x] 7.1 `cargo build --workspace --all-targets` with no warnings (default features and `--features corvis/health-tls`)
- [x] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 7.3 `cargo test --workspace` passes, including the new `crates/corvis/tests/health.rs` regression tests and the
  differential fixture test
- [x] 7.4 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 7.5 corvis-parity: the three port apps (consi, facten, houseplan) keep 0 apidiff differences (framework runtime
  code changed, even though the ports do not use health checks)

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive`
- [ ] Close issue [#19](https://github.com/randygil/corvis/issues/19)
- [ ] Tick #19 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [ ] Move the card to Done in the Corvis Roadmap project
