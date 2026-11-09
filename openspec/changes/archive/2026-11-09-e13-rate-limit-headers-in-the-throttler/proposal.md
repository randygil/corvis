# Proposal

Implements [randygil/corvis#120](https://github.com/randygil/corvis/issues/120) (epic [#13](https://github.com/randygil/corvis/issues/13)).
Design notes come from the #23 spike, feature 6
(`openspec/changes/archive/2026-11-08-e13-missing-core-features-users-expect/design.md`).

## Why

`ThrottlerGuard` answers 429 without `Retry-After` and never sends the `X-RateLimit-*` headers that
`@nestjs/throttler` sends on every counted request, so clients migrated from the original cannot back off or show
remaining quota, and the docs list it as a limitation. The guard cannot do it today: a guard has no way to add a
response header unless the handler declares `#[res]` (and the `Res` only exists after the guards ran), and
`HttpException` carries no headers.

## What Changes

- A response-header mechanism for enhancers: `ctx.switch_to_http().set_response_header(name, value)`. Headers set this
  way by guards, interceptors or pipes are merged into the final response on success and on error (also when an
  exception filter builds the response), never replacing a header the response already has, and are copied into the
  `Res` of `#[res]` routes.
- `ThrottlerGuard` sends `X-RateLimit-Limit`, `X-RateLimit-Remaining` and `X-RateLimit-Reset` on every counted
  request and `Retry-After` on a 429, with a `-<name>` suffix for throttlers not named `default`, as
  `@nestjs/throttler` does. Values are in seconds. **Behavior change:** apps using `ThrottlerGuard` gain these headers
  (on by default, as in the original).
- `ThrottlerStorage::with_headers(false)` turns the headers off.
- No headers in a WebSocket context (the guard still counts).
- A differential fixture recorded from `@nestjs/throttler` and a Rust test that replays it.
- Docs (EN + ES): rate limiting, and the new header API where guards and interceptors are documented.

## Capabilities

### New Capabilities

- `core/enhancers`: response headers set by enhancers through the execution context. (The capability is also created
  by #17's change; both only add requirements, so either archive order works.)
- `security/rate-limiting`: the headers `ThrottlerGuard` sends, their values, suffixes, and how to turn them off.

### Modified Capabilities

None (no archived spec covers these areas yet).

## Impact

- `crates/corvis/src/common/context.rs` (`HttpArgumentsHost::set_response_header` and the per-request header store).
- `crates/corvis/src/router/mod.rs` (`execute_route`, `run_pipeline`: merge points).
- `crates/corvis/src/modules/throttler.rs` (`ThrottlerGuard`, `ThrottlerStorage::with_headers`).
- Tests in a new `crates/corvis/tests/response_headers.rs`, a throttler test, and a fixture under
  `crates/corvis/tests/fixtures/throttler/`.
- Docs: `security/rate-limiting.md`, `guards.md`, `interceptors.md` and their Spanish translations under `docs/es/`.
- `examples/cats-app` uses `ThrottlerGuard` globally: its HTTP responses gain the headers (it has no tests to update).
- Port apps: none uses the throttler; the merge only runs when an enhancer sets a header, so apidiff should not move.
  The parity check confirms it.
- Lands after #17, which touches the same dispatch path (`execute_route`, filter order), or rebases on it.
- Lands **before** #135 (`e13-throttler-block-duration-like-the-original`): #135 replaces the counter behind these
  headers with the original's block model and modifies two requirements of this change's `security/rate-limiting`
  spec, so this change must be archived first.

## Out of scope

- The original's block model (`blockDuration`, defaulting to `ttl` from the moment of the block) and its sliding hit
  count: filed as #135, implemented after this change. Until then `Retry-After` reports the rest of Corvis's fixed
  window (design.md, Decision 5).
- A per-throttler `setHeaders` option: `ThrottlerOptions` is built with struct literals in the docs, and a new public
  field would break them; possible new issue together with a builder.
- `HttpException::with_header` for headers on user exceptions: possible new issue.
- Exposing the headers to browsers through CORS (`Access-Control-Expose-Headers`): users configure it in
  `CorsOptions`; the docs say so.
- Throttling on WebSocket gateways (decided by #116) and custom trackers or error messages for the throttler.
