# Tasks

## 1. Reference behavior and failing tests

- [x] 1.1 Add `crates/corvis/tests/fixtures/throttler/` with a `package.json` pinning `@nestjs/throttler`,
  `@nestjs/core`, `@nestjs/common`, `@nestjs/platform-express`, `reflect-metadata` and `rxjs`, and a `generate.mjs`
  that runs the sequence of design.md, Decision 7, on `127.0.0.1` and writes `throttler.json` (status and throttler
  headers per response); record in design.md whether `Retry-After` reaches the 429 response and its value on the first
  block; stop for a maintainer decision if header names or the allowed/blocked split differ from the spec; verify by
  running `node generate.mjs` once and committing the JSON (not `node_modules`)
- [x] 1.2 Add failing tests in `crates/corvis/tests/response_headers.rs` (new), one per `core/enhancers` scenario:
  guard header on `Ok`, invalid value, WebSocket no-op, route header wins, handler header wins, `Err` without filter,
  `Err` with filter, interceptor after the handler, `#[res]` passthrough, `#[res]` handled (with and without the
  handler setting the same header); verify they fail to compile or fail for the expected reason with
  `cargo test -p corvis --test response_headers`
- [x] 1.3 Add failing throttler tests (in `crates/corvis/tests/modules.rs` or a new `throttler_headers.rs`), one per
  `security/rate-limiting` scenario except the replay; verify they fail for the expected reason

## 2. Response-header mechanism

- [x] 2.1 Add `HttpArgumentsHost::set_response_header(name, value) -> bool` and the lazily created `EnhancerHeaders`
  request extension in `crates/corvis/src/common/context.rs` (validation, WebSocket no-op, write-through to an existing
  `Res` when absent and not sent); verify the invalid-value and WebSocket tests pass
- [x] 2.2 In `run_pipeline` (`crates/corvis/src/router/mod.rs`), copy recorded headers into the new `Res` after the
  route's `#[header]` values, when absent; verify the two `#[res]` tests pass
- [x] 2.3 Add `apply_enhancer_headers` and call it on the `Ok`, `Err` and passthrough branches of `execute_route`
  (never on the sent/handled branch); verify every test of 1.2 passes and `crates/corvis/tests/res.rs` and
  `crates/corvis/tests/express_parity.rs` still pass

## 3. Throttler

- [x] 3.1 Add `headers: bool` (default `true`) and `ThrottlerStorage::with_headers(bool)` in
  `crates/corvis/src/modules/throttler.rs`; verify the existing unit tests in that file still pass
- [x] 3.2 In `ThrottlerGuard::can_activate`, keep `time_to_expire`, and when the context is HTTP and headers are on,
  set `Retry-After<suffix>` before the 429 and the three `X-RateLimit-*<suffix>` headers on allowed requests (seconds,
  rounded up; `Retry-After` at least 1); verify the tests of 1.3 pass
- [x] 3.3 Add the Rust test that replays `throttler.json` with the test client and compares as design.md, Decision 7,
  says; verify it passes

## 4. Docs

- [x] 4.1 `docs/security/rate-limiting.md`: a "Response headers" section (names, suffixes, seconds, 429,
  `with_headers(false)`, throttler names become header names so they must be header-safe and say nothing internal, CORS `Access-Control-Expose-Headers`, `Retry-After` follows the fixed window, which #135
  changes later); remove the missing-headers limitation; same in `docs/es/security/rate-limiting.md`
- [x] 4.2 `docs/guards.md` and `docs/interceptors.md`: document `ctx.switch_to_http().set_response_header` and the
  merge rule (enhancer headers never replace headers the response already has; kept on errors and filter responses);
  same in `docs/es/`
- [x] 4.3 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes, including the regression tests of 1.2 and 1.3 and the replay of 3.3
- [x] 5.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 5.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences (none uses the throttler or sets enhancer headers)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #120, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
