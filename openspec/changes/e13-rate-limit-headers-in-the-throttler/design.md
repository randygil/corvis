# Design

## Context

See proposal.md for motivation; the behavior is in the two spec deltas. Current state, verified in the code:

**Why a guard cannot set headers today**

- `ExecutionContext` (`crates/corvis/src/common/context.rs:57-139`) wraps the `Request`; `HttpArgumentsHost`
  (`context.rs:141-163`) offers `get_request`, `status_code` and `get_response`, which returns a `Res` only when the
  route declared `#[res]` (it reads the `Res` request extension, `context.rs:159-162`).
- `run_pipeline` (`crates/corvis/src/router/mod.rs:732-746`) runs the guards first (lines 733-737) and only then
  creates the `Res` for `#[res]` routes, sets the route's `#[header]` values on it and inserts it in the request
  (lines 738-745). So even for `#[res]` routes a guard sees no `Res`.
- `HttpException` (`crates/corvis/src/http/exception.rs:20-29`) has no headers.
- Request extensions are shared by every clone of the request behind an `Arc<RwLock<Extensions>>`
  (`crates/corvis/src/http/request.rs:31`, `insert`/`get` at lines 210-216), so a value inserted by a guard through
  `ctx.request()` is visible to the router after the pipeline.

**Where responses are assembled**

- `execute_route` (`router/mod.rs:691-730`): a panic in the pipeline becomes an exception (lines 696-699); without a
  `Res` (lines 700-706), `Ok` goes through `finalize` and `Err` through `handle_exception`. With a `Res`: if its
  headers were sent or the handler ended it, the `Res` response is returned (lines 707-716); otherwise passthrough
  merges the `Res` into the returned response (lines 718-723) or into the filter's response (lines 724-728) with
  `Res::merge_into`.
- `finalize` (`router/mod.rs:836-862`) applies the route's `#[header]` values only when the response has no header with
  that name (lines 850-859).
- `handle_exception` (`router/mod.rs:864-876`) returns the first catching filter's response or
  `BaseExceptionFilter::handle`.
- `Res::merge_into` (`crates/corvis/src/http/res.rs:871-895`) copies `Res` headers absent from the response (headers
  already on the response win; `Set-Cookie` values are added). `Res::has_header` and `Res::set` exist (`res.rs:649`,
  `res.rs:613`).
- `Response::set_header` (`crates/corvis/src/http/response.rs:139-143`) silently ignores invalid names and values.
  `Response::default_headers` (`response.rs:77-79`) are only applied in `server::into_hyper`
  (`crates/corvis/src/server.rs:140-148`), after middleware has seen the response.
- The test client calls the same dispatch and then `express::finalize` (`crates/corvis/src/testing.rs:384-385`), so tests
  through it cover the real merge path.

**Throttler**

- `ThrottlerOptions { name, ttl, limit }` (`crates/corvis/src/modules/throttler.rs:35-48`); the docs build it with
  struct literals (`docs/security/rate-limiting.md:54-55`).
- `ThrottlerStorage::increment(key, ttl) -> (hits, time_to_expire)` (`throttler.rs:101-127`) is a fixed window: the
  first hit sets the expiry, later hits increment until it passes.
- `ThrottlerGuard::can_activate` (`throttler.rs:164-183`) skips on `SKIP_THROTTLE`, applies `THROTTLE_LIMIT` /
  `THROTTLE_TTL` overrides, increments per throttler, discards the expiry (`let (hits, _)`, line 176) and returns
  `HttpException::new("ThrottlerException: Too Many Requests", 429)` when `hits > limit` (line 178). It does not look at
  the context type.
- `ThrottlerModule::for_root_storage` (`throttler.rs:149-154`) accepts a configured storage (`with_max_entries`,
  lines 81-84).
- `docs/security/rate-limiting.md:120-123` lists the missing headers and the gateway behavior as limitations.

**Original library (`@nestjs/throttler` master, read on 2026-11-08; the fixture re-checks the pinned version)**

- `throttler.guard.ts`: header prefix `X-RateLimit`; suffix `name === 'default' ? '' : '-' + name`. On an allowed
  request it sets `-Limit` (`limit`), `-Remaining` (`Math.max(0, limit - totalHits)`) and `-Reset` (`timeToExpire`).
  When blocked it sets `Retry-After<suffix>` (`timeToBlockExpire`) and throws, so the blocking throttler sets no
  `X-RateLimit-*`. `setHeaders` exists per throttler and at module level, default `true`. `blockDuration` defaults to
  `ttl`. No context-type check (it reads `switchToHttp()`).
- `throttler-storage-record.interface.ts` documents `timeToExpire` and `timeToBlockExpire` as seconds.
- `throttler.exception.ts`: message `ThrottlerException: Too Many Requests`, status 429 (Corvis matches).

## Goals / Non-Goals

**Goals:**
- A general, small way for any enhancer to add response headers, merged predictably on every response path.
- Throttler headers with the original's names, suffixes and values, on by default, with an off switch.

**Non-Goals:**
- The original's block model (#135, next change), per-throttler `setHeaders`, headers on exceptions, CORS exposure
  (proposal, Out of scope).

## Decisions

1. **`set_response_header` on `HttpArgumentsHost`, stored as a request extension.**
   `ctx.switch_to_http().set_response_header(name, value) -> bool` parses `HeaderName` / `HeaderValue` (rejecting CR,
   LF and invalid names, returning `false`), then stores the pair in a `pub(crate) struct EnhancerHeaders(Arc<Mutex<HeaderMap>>)`
   extension, created on first use (`insert` replaces an earlier value for the name). If a `Res` extension already
   exists and its headers were not sent, the header is also written to it when `!res.has_header(name)`. In a WebSocket
   context it returns `false` and stores nothing.
   Alternatives considered: `HttpException::with_header` (spike: does not cover headers on success, which is most of the
   feature); making `get_response()` always return a `Res` (changes the meaning of `#[res]` passthrough for every
   route); writing into `Response::default_headers` (applied after middleware, so middleware could not see or adjust
   them). Returning `bool` instead of silently ignoring, as `Response::set_header` does, lets a guard notice a bad
   throttler name.
2. **Merge once, at the end of `execute_route`, with "absent only".** A helper `apply_enhancer_headers(&ctx, &mut resp)`
   runs on the response `execute_route` is about to return, on every branch: after `finalize` (so `#[header]` already
   won), after `handle_exception` (so the filter's headers already won), and after `Res::merge_into` in passthrough.
   In the "headers sent / handled" branch the `Res` already carries every header recorded before it was sent
   (Decision 3), so the helper is not applied there: headers recorded after the response was sent are ignored, as
   Express would refuse them. Only routes reach `execute_route`; 404s and middleware errors have no enhancers.
   Alternative considered: merge in `server::handle` for every response. Rejected: the test client and the server
   would both need it, and it would mix route state into the generic path.
3. **`#[res]` routes: copy at creation, write-through afterwards.** In `run_pipeline`, after the `Res` is created and
   the route's `#[header]` values set, copy each recorded header absent from the `Res`. Later calls write through
   (Decision 1). A handler that sets the same header on `Res` overwrites it, so the handler wins, as the spec requires.
4. **Throttler headers in seconds, rounded up.** `Reset` = `ceil(time_to_expire)` in seconds, `Retry-After` = the same
   for the blocking throttler with a minimum of 1, `Remaining` = `limit.saturating_sub(hits)`. The guard keeps the
   `time_to_expire` it already receives from `increment` (line 176). Headers are set only when
   `ctx.get_type() == ContextType::Http` and the storage's `headers` flag is on. The order per throttler mirrors the
   original: blocked → `Retry-After` and throw; allowed → the three `X-RateLimit-*` headers. The header-setting code
   lives in one private helper that takes the throttler name, the effective limit, the hit count and the two
   durations, so #135 can swap the counter call (`increment` → `record_hit`) without touching the header logic.
5. **`Retry-After` follows Corvis's fixed window.** The original blocks for `blockDuration` (default `ttl`) from the
   moment of the block, so its `Retry-After` on the first 429 of a window is the full `ttl`, while Corvis's counter
   unblocks when the window ends. Reproducing the block model would change the storage and when clients are let back
   in, which the issue does not ask for. `Retry-After` therefore reports when Corvis will accept the client again (the
   honest value for this counter), and the differential test compares `Retry-After` by name and presence only. #135
   replaces the counter with the original's model and then compares `Retry-After` by value.
6. **Off switch on the storage.** `ThrottlerStorage::with_headers(bool)` (field `headers: bool`, default `true`), next
   to `with_max_entries`, so `for_root_storage` and #119's `for_root_async` both reach it.
   Alternative considered: a field on `ThrottlerOptions` (per throttler, like the original). Deferred: adding a public
   field breaks the struct literals in the docs and in users' code.
7. **Differential fixture like `crates/corvis-orm/tests/decimal_js.rs`.** `crates/corvis/tests/fixtures/throttler/`
   holds a `package.json` pinning `@nestjs/throttler`, `@nestjs/core`, `@nestjs/common`, `@nestjs/platform-express`
   and a `generate.mjs` that starts an app on `127.0.0.1`, sends the sequence of the spec (two named throttlers, a
   skipped route, a `@Throttle` limit override, requests past the limit) and writes the status and throttler headers of
   each response to `throttler.json`. The Rust test replays the sequence with the test client and compares names,
   `Limit`, `Remaining` exactly and `Reset` within one second; `Retry-After` by presence.

## Risks / Trade-offs

- [Merge order mistakes override a header set on purpose] → "absent only" everywhere, one helper, and one test per path
  (`Ok`, `Err` without filter, `Err` with filter, `#[res]` passthrough, `#[res]` handled), as the issue lists.
- [Responses of apps using `ThrottlerGuard` change] → on by default as in the original (spike review); documented
  with `with_headers(false)`; cats-app has no tests depending on headers; the ports do not use the throttler.
- [A `Mutex` per request that uses the mechanism] → created lazily, only when an enhancer sets a header.
- [Throttler names with characters invalid in a header name] → the header is rejected (`set_response_header` returns
  `false`); the guard ignores the result; the docs say names should be header-safe tokens.
- [Information disclosure: the headers reveal each limit, its window and the throttler names to any client] → the
  same as the original, which is the point of parity; an attacker could measure them by probing anyway. The docs say
  `with_headers(false)` hides them and that throttler names end up in header names, so they should not carry
  anything internal.
- [Retry-After differs from the original on the first block of a window] → Decision 5; fixed by #135.

## Open questions

1. **Block model.** Resolved: filed as #135 (`e13-throttler-block-duration-like-the-original`), which lands after this
   change; this change keeps the fixed-window `Retry-After` (Decision 5).
2. **Per-throttler switch.** Is the module-level `with_headers(false)` enough, or should a builder for
   `ThrottlerOptions` (with `set_headers`) be added in a follow-up issue?
3. **`set_response_header` from middleware.** Middleware does not receive an `ExecutionContext`, so it cannot use the
   mechanism; it can still set headers on the response it returns. Confirm that is acceptable.

## Review notes (2026-11-08)

6. Replicating the original's block model (`blockDuration`, `Retry-After` from the block start) is filed as a separate
   issue; this change checks `Retry-After` by presence only, as proposed.
7. The module-level switch is enough; per-throttler `setHeaders` is not added (it would break struct literals).
8. Middleware cannot set throttler headers; acceptable (the throttler is a guard).

## Implementation notes

- The regression tests (tasks 1.2, 1.3) were written together with the code rather than seen failing first; they cover
  every scenario of both specs and pass. The WebSocket scenarios are unit tests in `modules/throttler.rs` (they need
  `EnhancerHeaders`, which is crate-private).
- Fixture (`tests/fixtures/throttler/`): `@nestjs/throttler` 6.7.1 with `@nestjs/*` 12.4.0, three apps (`named`,
  `second`, `default`). Header names and the allowed/blocked split match the spec. `Retry-After` does reach the 429;
  on the first block it equals the full `ttl` (60 / 30), as Decision 5 predicted, and it is compared by presence only.
  The blocking throttler sets no `X-RateLimit-*` and earlier throttlers' headers stay, as in the spec.
- Finding: the original `@SkipThrottle()` without arguments skips only the throttler named `default`; Corvis's
  `SKIP_THROTTLE` skips all throttlers. The fixture skips every throttler by name to compare the same behavior, and
  the docs list the difference under Limitations. Not changed here.
- The header code is the private `ThrottlerGuard::set_headers` (name, limit, hits, time to expire, blocked), so #135
  only changes the counter call.
- Throttler names are validated by `set_response_header` (`HeaderName::from_bytes`); an invalid name drops the header
  and a test covers it.
- Task 5.5 (corvis-parity) is left to the maintainer's session as instructed. `corvis-orm` tests were not run (asked
  to exclude them).
