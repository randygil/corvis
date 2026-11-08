# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state, verified in the code (all paths
under `crates/corvis/`):

**Corvis storage and guard (`src/modules/throttler.rs`)**

- `ThrottlerOptions { name, ttl, limit }` (lines 35-48), all public fields, `ttl` in milliseconds; the docs build it
  with struct literals (`docs/security/rate-limiting.md:52-57`).
- `Counters` (lines 55-61) maps `sha256(key)` to `(hits, expiration)` and keeps a `BTreeSet<(Instant, KeyHash)>`
  ordered by expiration. `ThrottlerStorage` (lines 69-73) holds the throttlers in the order given (`new`, lines
  76-78), the counters and `max_entries` (`with_max_entries`, lines 81-84; default 100,000, line 51).
  `memory_usage` (lines 96-99) multiplies the number of counters by a fixed per-entry size.
- `increment(key, ttl) -> (hits, time_to_expire)` (lines 101-127) is a fixed window: it purges expired counters in
  expiration order (lines 108-114), increments an existing counter with `saturating_add` and returns its remaining
  time (lines 115-118), so **requests rejected with 429 are counted too**; otherwise it evicts the counter closest to
  expiring while the table is full (lines 120-123) and creates one expiring at `now + ttl` (lines 119-126).
- `ThrottlerGuard::can_activate` (lines 166-182) returns early on `SKIP_THROTTLE` (lines 168-170), iterates the
  throttlers in the configured order (line 172), applies the `THROTTLE_LIMIT` / `THROTTLE_TTL` overrides to every
  throttler (lines 173-174), keys by `name:ip:class:handler` (line 175), discards the expiry (`let (hits, _)`, line
  176) and fails with `ThrottlerException: Too Many Requests` / 429 when `hits > limit` (lines 177-179), so later
  throttlers are not counted for that request.
- Unit tests (lines 193-230) cover the guard's name, the memory bound and per-key reset after the TTL. Integration
  tests: `tests/modules.rs:140-156` (limit 2, third request 429) and `tests/input_hardening.rs:127-138` (limit 3,
  `X-Forwarded-For` rotation: 200, 200, 200, 429, 429).
- Exports: `src/lib.rs:275-277` (`modules::throttler` re-export) and the prelude list at `src/lib.rs:297-298`.
- Docs: `docs/security/rate-limiting.md:43-57` describes the fixed window; lines 59-87 the metadata table; lines
  89-104 the storage cap.
- #120 is not implemented yet (`set_response_header` does not exist in `crates/`); this change assumes its code is
  merged: the guard sets `X-RateLimit-*` and `Retry-After` through one helper fed by `increment`'s
  `time_to_expire` (#120 design.md, Decision 4).

**Original (`@nestjs/throttler` 6.7.1, the latest release on 2026-11-08; read from the published package's
`dist/throttler.service.js` and `dist/throttler.guard.js`, and compared with `src/` on the master branch)**

`ThrottlerStorageService.increment(key, ttl, limit, blockDuration, throttlerName)`, with `now = Date.now()` in
milliseconds:

1. Create the record if missing: `totalHits = 0`, `expiresAt = now + ttl`, `blockExpiresAt = 0`, `isBlocked = false`.
2. `pruneExpiredHits`: drop hit expirations `<= now`; `totalHits` = the number left. Each hit expires `ttl` after it
   was counted (`fireHitCount` pushes `now + ttl`): a sliding window. (Earlier releases decremented each hit with a
   `setTimeout`; same semantics.)
3. `timeToExpire = ceil((expiresAt - now) / 1000)`; if `<= 0`, `expiresAt = now + ttl` and recompute. This window
   only feeds `X-RateLimit-Reset`; it does not reset the count.
4. If `blockDuration <= 0`: `incrementWithoutBlock` — blocked when `totalHits >= limit`; the hit is counted only when
   not blocked; `timeToBlockExpire` = seconds until the oldest live hit expires (or `timeToExpire` when there is
   none); the returned `totalHits` adds 1 when blocked.
5. Otherwise: if not blocked, count the hit. If `totalHits > limit` and not blocked: `isBlocked = true`,
   `blockExpiresAt = now + blockDuration`. So the request that goes over the limit is counted and blocked, and
   requests during the block are not counted and do not extend it.
6. `timeToBlockExpire = ceil((blockExpiresAt - now) / 1000)`. If `<= 0` while blocked: `resetBlockedRequest`
   (unblock, clear the hits) and count this hit, so the first request after a block starts from one.
7. Return `{ totalHits, timeToExpire, isBlocked, timeToBlockExpire }`; all times in seconds, rounded up.

On master, step 6's "block elapsed" check runs **before** step 5 (lift the block, then count the hit against the
limit). Both orders give the same result for `limit >= 1`; with `limit` 0, 6.7.1 lets one request through each time
a block ends, and master blocks it again.

`ThrottlerGuard`:

- `onModuleInit` sorts the throttlers by numeric `ttl` ascending (`Array.prototype.sort`, stable) and names unnamed
  ones `default`.
- Per throttler: `blockDuration = routeOrClassBlockDuration ?? namedThrottler.blockDuration ?? ttl`, where `ttl` is
  the effective one after the route's `@Throttle` override; the route value comes from
  `THROTTLER_BLOCK_DURATION + name` metadata.
- Blocked → `Retry-After<suffix>: timeToBlockExpire`, then throw (later throttlers are not counted). Allowed →
  `X-RateLimit-Limit<suffix>: limit`, `-Remaining: max(0, limit - totalHits)`, `-Reset: timeToExpire`.
- The storage key is `sha256("<class>-<handler>-<name>-<tracker>")`, the tracker being `normalizeIp(req.ip)` (IPv6 to
  a /64 by default). Records with no live hits, an expired window and no active block are swept every 60 s; that
  sweep is not observable, because a fresh record behaves like the stale one (step 1 versus steps 2, 3 and 6).

## Goals / Non-Goals

**Goals:**
- For the same throttlers, metadata and request timing, Corvis blocks the same requests and reports the same
  `Retry-After` and `X-RateLimit-*` values as `@nestjs/throttler` 6.7.1, checked by a differential test.
- A `blockDuration` equivalent with the original's default, per throttler and per route.
- Memory stays bounded under attack, as the security audit required of the current storage.

**Non-Goals:**
- Per-throttler skips and overrides, IPv6 normalization, user-agent and `skipIf` options, custom storages, a
  `ThrottlerOptions` builder (proposal, Out of scope).

## Decisions

1. **Replicate the 6.7.1 algorithm per record, with master's order for lifting an elapsed block.** Each key gets a
   record `{ hits, window_end, blocked_until: Option<Instant> }` and `record_hit` follows steps 1-7 above, except that
   an elapsed block is lifted before the hit is counted (master). That only changes `limit` 0, where 6.7.1 lets a
   request through after every block; Corvis already rejects every request with `limit` 0 today and keeps doing so
   (security over parity, documented).
   Alternative considered: keep the fixed window and add only the block. Rejected: `X-RateLimit-Remaining` and which
   request gets blocked would still differ once a sequence crosses a window boundary, failing the issue's "values
   equal to the original" check, and the fixed window keeps the 2×limit burst at boundaries.
2. **Hit expirations in a per-record deque, idle time in the ordered index.** `hits: VecDeque<Instant>` holds the
   expiry of each counted hit. A key always uses the same `ttl` from the guard (the metadata is static), so expiries
   arrive in order and pruning pops from the front; `record_hit` and `increment` still insert in order when a caller
   passes different TTLs for one key. The ordered index changes from expiration to **idle time**:
   `max(window_end, newest hit expiry, blocked_until)`, updated when a record changes. The purge on each call and the
   eviction when full keep their current policy, keyed by idle time. Removing an idle record is not observable (see
   the sweep note above), so the purge does not break parity.
   Alternative considered: one `setTimeout`-like task per hit (the original up to 6.5). Rejected: a task per request.
3. **A global hit budget besides the record cap.** Records keep at most `limit + 1` hits (block mode) or `limit`
   (no-block mode), so the worst case is `max_entries × (limit + 1)` timestamps. `with_max_hits(n)` (default
   1,000,000, roughly 16 MB of `Instant`s; the implementer may store milliseconds since the storage's creation to
   halve it) caps the total; when a hit would exceed it, records are evicted by idle time like a full table.
   `memory_usage` counts the hits. `with_max_hits` is clamped to at least 1, like `with_max_entries`.
   Alternatives considered: an approximate sliding window (two weighted fixed buckets, constant memory). Rejected:
   values differ from the original, which is the point of the issue. A per-record cap below `limit`. Rejected: the
   count would be wrong for large limits.
4. **Block duration on the storage and in metadata, not on `ThrottlerOptions`.**
   `ThrottlerStorage::with_block_duration(name, ms)` stores a per-name duration; `THROTTLE_BLOCK_DURATION`
   (`"THROTTLER:BLOCK_DURATION"`, milliseconds, `u64`) overrides it for a route or controller, for every throttler,
   like `THROTTLE_LIMIT` and `THROTTLE_TTL`. The effective value is metadata, else the storage value, else the
   effective `ttl`, as in the original. A name that matches no throttler panics with a message naming it: it is a
   configuration error found at startup, and ignoring it would leave a client unblocked sooner than the author
   meant. This follows #120's choice of `with_headers` on the storage and reaches both `for_root_storage` and #119's
   `for_root_async`.
   Alternatives considered: a `block_duration: Option<u64>` field on `ThrottlerOptions` (closest to the original;
   breaks every struct literal, including the docs); a `ThrottlerOptions` builder (still needs a new field, so it
   breaks literals too). Left as Open question 2.
5. **`0` turns the block off, as in the original.** With an effective block duration of 0, `record_hit` follows
   `incrementWithoutBlock` (step 4). `u64` milliseconds cannot be negative, so `<= 0` becomes `== 0`.
6. **Throttlers sorted by `ttl` in `ThrottlerStorage::new`.** A stable sort by `ttl` ascending reproduces the
   original's order, which decides which throttler blocks first and so which headers a 429 carries and which counters
   grow. #119's `From<Vec<ThrottlerOptions>>` delegates to `new`, so every construction path sorts.
7. **New storage method; `increment` keeps its signature.** `record_hit(&self, key, ttl: Duration, limit: u32,
   block_duration: Duration) -> ThrottlerRecord`, with `ThrottlerRecord { total_hits, time_to_expire, is_blocked,
   time_to_block_expire }` (`Duration`s; `time_to_block_expire` is zero when not blocked; `#[non_exhaustive]`, it is
   only returned). `increment(key, ttl)` becomes `record_hit(key, ttl, u32::MAX, ttl)` and returns
   `(total_hits, time_to_expire)`; its rustdoc says it counts in a sliding window now. The guard rounds the durations
   up to whole seconds when setting headers, as #120 already does, with the `Retry-After` minimum of 1 kept.
   Alternative considered: change `increment` to the original's five-argument signature. Rejected: a breaking change
   for no gain.
8. **A private clock seam for exact tests.** `record_hit` calls a private `record_hit_at(key, now, ..)`; unit tests in
   `throttler.rs` drive it with `base + Duration::from_millis(t)`. Integration tests cannot reach it, so the exact
   storage comparison lives in the unit tests and the HTTP tests use real time.
9. **Two differential tests.**
   - **Storage, exact.** `crates/corvis/tests/fixtures/throttler/storage.mjs` imports `ThrottlerStorageService` from
     the pinned package, replaces `Date.now` with a counter it advances, runs scripted sequences of
     `{ t, key, ttl, limit, blockDuration }` calls, and writes each returned record to `storage.json`. Sequences: a
     block with retries during it and the first request after it; hits expiring across a window boundary (the
     sliding count); a block longer and one shorter than `ttl`; `blockDuration` 0; two keys interleaved; a different
     `ttl` per key. A unit test replays them through `record_hit_at` and compares `total_hits`, `is_blocked`, and the
     two times in seconds (`timeToBlockExpire` compared as `max(0, ·)` when not blocked) exactly. `limit` 0 is left
     out of the fixture (Decision 1) and has its own unit test.
   - **HTTP, tolerant.** #120's `generate.mjs` / replay gains a sequence with a 2-second throttler that blocks, a retry
     during the block and a request after it, and the replay compares `Retry-After` by value within one second (it
     compared presence only). Request offsets are kept at least 300 ms away from any expiry so timing jitter cannot
     change which side of a boundary a request lands on.
   The fixtures use the same `package.json` as #120; if #120 pinned a version other than 6.7.1, both fixtures move to
   one version and the design notes are re-checked against it.

## Risks / Trade-offs

- [Apps block longer than before] → stated as a behavior change in the docs, with `with_block_duration(name, 0)` or
  `THROTTLE_BLOCK_DURATION` 0 for the original's no-block mode.
- [More memory per key (one timestamp per counted hit)] → bounded by `with_max_hits` (Decision 3); the docs give the
  worst case and how to raise or lower the caps.
- [Eviction under attack resets other clients' counters] → already true of `max_entries`; the hit budget adds a second
  trigger. Evicting by idle time drops the records that would have been removed soonest anyway. Documented.
- [A very large `limit` lets one client fill the hit budget] → with the default budget, a limit above roughly 10,000
  per key on a busy app needs a larger `with_max_hits`; the docs say so.
- [HTTP replay timing] → the value comparison that matters is the exact storage test; the HTTP replay keeps a
  one-second tolerance and requests away from boundaries.
- [Panic on an unknown throttler name] → happens while the module graph is built, before serving; the message names
  the method and the name.
- [Size] → the issue is sized S; the sliding count and the memory budget make this closer to M.

## Open questions

1. **Sliding count.** The issue asks for the block; the sliding hit count is what makes `X-RateLimit-Remaining` equal
   across windows (Decision 1). Confirm it belongs here, or keep the fixed window and limit the comparison to
   sequences inside one window.
2. **Where the block duration is configured.** `ThrottlerStorage::with_block_duration(name, ms)` (proposed,
   non-breaking) versus a `block_duration` field on `ThrottlerOptions` (closest to the original, breaks struct
   literals) versus a `ThrottlerOptions` builder in a follow-up issue together with #120's per-throttler
   `setHeaders`.
3. **Default hit budget.** Is 1,000,000 stored hits (about 16 MB) the right default?
4. **`limit` 0.** Confirm following the original's master order (every request rejected) over 6.7.1 (one request let
   through after each block).
5. **Unknown name.** Panic (proposed) or ignore a `with_block_duration` name that matches no throttler?
