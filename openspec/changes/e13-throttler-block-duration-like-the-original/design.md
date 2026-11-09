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

## Review notes (2026-11-08)

1. The sliding window is in scope, as in the original 6.7.1: each hit expires `ttl` after it was made.
2. The block duration is configured on the storage (`ThrottlerStorage::with_block_duration`) and through the `THROTTLE_BLOCK_DURATION` metadata; `ThrottlerOptions` gets no new field.
3. `with_max_hits` defaults to 1,000,000.
4. With `limit` 0 every request is rejected (the divergence from 6.7.1 is confirmed and documented).
5. `with_block_duration` with an unknown throttler name panics at registration.
6. The size is M. Implement after #120.

## Implementation notes

- Reference version: `crates/corvis/tests/fixtures/throttler/package.json` pins `@nestjs/throttler` 6.7.1, the version
  design.md was written against; no re-read was needed.
- The recorded storage fixture shows two behaviors worth knowing: while a key is blocked, `totalHits` is the number of
  hits still unexpired (hits expire during a block, so it can drop below the limit while `isBlocked` stays true), and
  `timeToBlockExpire` is negative when the key is not blocked (the replay compares `max(0, .)`). `record_hit`
  reproduces both; `total_hits` of a blocked key is the live hit count.
- `ThrottlerStorage::hits_len()` was added (public, read-only) so tests and operators can see the stored timestamps.
- Hits are stored as `Instant` (not milliseconds since creation); the budget is counted in timestamps.
- Clock arithmetic never panics: every `Instant + Duration` goes through `after`, which clamps the span to ten years and
  falls back to `now` if the addition would still overflow. Values of `u64::MAX` ms for the ttl and the block duration
  are covered by a test.
- When one key alone fills `max_hits` (for example through `increment`, which has no limit), its oldest hits are dropped,
  so its count is capped at the budget; other keys are evicted by idle time first.
- A key used with different `ttl` values keeps its hits in expiry order (`partition_point`), as Decision 2 allows.
- The HTTP fixture gained a `block` app (2 s throttler, limit 2) and `generate.mjs` accepts requests of the form
  `{ path, at }` (milliseconds after the first request). The replay compares `Retry-After` and `X-RateLimit-Reset`
  within one second.
- Task 5.5 (corvis-parity) is left to the maintainer's session, as agreed in the brief.

## Security review follow-up (2026-11-09)

The security review found that the global hit budget (`with_max_hits`, Decision 3) let an attacker evict their own
round-robin keys before they reached the limit (about 2,000 keys got 98% of requests through at 1,000 per hour, and a
blocked key could be unblocked by flooding other keys), that a limit at or above the budget made a key prune its own hits
and never block, and that `VecDeque` capacity is kept after pruning, so real memory was far above what the budget
suggested. The maintainer chose to replace exact per-hit storage with fixed buckets. This section supersedes Decision 3,
review note 3 and the hit-budget notes of "Implementation notes".

- **Buckets.** A key keeps at most 64 buckets `(expiry, count)`. A hit joins the previous bucket when its expiry falls in
  the same `ttl / 64` slice (counted from the key's first hit); the bucket then expires with its newest hit, so a merged
  hit never expires earlier than exactly (fail closed). When 64 buckets are live, a new hit merges into the nearest
  later bucket. Memory per key is at most 64 x 24 bytes plus a fixed overhead, whatever the limit, so memory is bounded
  by `max_entries` alone (about 1.8 KB per key, 180 MB at the default 100,000 keys in the worst case).
- **Removed:** `with_max_hits`, `DEFAULT_THROTTLER_MAX_HITS`, `hits_len`, and the global hit budget.
- **Accuracy.** Exact while a key's hits are more than `ttl / 64` apart (the storage replay compares exactly for those
  fixture cases); otherwise `X-RateLimit-Remaining` and `Reset` can differ from the original by up to `ttl / 64`. Block
  semantics are unchanged. Documented as a known difference. The fixture was re-recorded with spaced hits and gained a
  `dense_hits` scenario compared within `ttl / 64`.
- **Eviction.** Blocked keys sit in a separate idle-time index and are evicted only when no unblocked key is left.
- **Clock saturation.** `after()` saturates to a far instant (ten years, then shorter spans if the platform cannot
  represent it) and falls back to `now` only if even one hour overflows, so an unrepresentable block still lasts.
- **Duplicate names.** `ThrottlerStorage::new` logs a warning when two throttlers share a name (they share counters and
  block durations).
- **Regression tests** reproduce the PoC modes: `round_robin_keys_get_at_most_the_limit_each` (churn),
  `a_blocked_key_stays_blocked_while_others_flood` (unblock, with a full table), `a_large_limit_still_blocks`, and
  `a_key_uses_bounded_memory_whatever_its_hits` (cap).

### Finding 4: blocked keys pinned the counters table (re-review, 2026-11-09)

Evicting blocked keys last (above) let an attacker fill the table with blocked keys (200,000 requests to
`default:<IPv6>:Auth:login` with limit 1 pinned 100,000 entries in 56 ms): `evict_one` then emptied the unblocked
keys, so every new counter evicted the previous idle one and no counter reached its limit. Fix, as suggested by the
reviewer: blocked keys live in a separate compact table (`key hash -> { until, window_end }`, no buckets; the hits are
cleared when the key is blocked, because they are not used during the block and are discarded when it ends) with its own
cap, `with_max_blocked` (default the value of `max_entries`, about 120 bytes per key). The counters table keeps its
full `max_entries`, so blocked keys cannot displace it. When the blocked table is full the block that ends soonest is
dropped (that frees only keys the attacker pinned). A key is looked up in the blocked table first; when its block has
ended it resumes as a fresh counter that keeps the reporting window (`window_end`), so `Retry-After` and `Reset` are
unchanged. The "blocked keys are evicted last" rule above is replaced by this one.

- `total_hits` of a response for a blocked key is now `limit + 1` instead of the live hit count (the guard ignores it);
  the storage replay no longer compares `total_hits` for blocked responses.
- Worst case at the defaults: counters about 180 MB plus blocked keys about 12 MB.
- Regression tests: `pinning_the_blocked_table_does_not_disable_other_counters` (PoC mode `pin`),
  `a_short_block_keeps_the_reporting_window`, `the_blocked_table_has_its_own_cap_and_drops_the_soonest_block`.
- IPv6 /64 normalization stays out of scope (tracked separately).

### Finding 5: eviction by end time let short blocks evict each other (re-review, 2026-11-09)

The blocked table evicted the block that ends soonest. An attacker who filled it with long blocks (a route with a 1 h
block duration) made short blocks on normal routes evict each other, and an evicted key restarts from zero (91% of
requests passed with two keys alternating bursts of `limit + 1`; the control with equal block lengths let 0.2% through).
Now the table evicts in FIFO order by when the block started (a strict sequence number kept in a separate index,
`blocked_fifo`); `blocked_idle` is used only to purge expired entries. Evicting the block that ends latest was rejected
because a brute-force attacker could then free their own long block with one short block. With FIFO, freeing another
client's block takes `max_blocked` new blocks after it, the same cost as rotating keys. A block that is refreshed while
it lasts keeps its sequence number. The measured heap is about 178 bytes per blocked key; the FIFO index adds about 50,
so the docs and the `with_max_blocked` rustdoc say about 230 bytes (about 23 MB at the default, 200 MB with the counters).
Tests: `short_blocks_cannot_evict_each_other_after_long_blocks_pin_the_table` and
`the_oldest_block_is_evicted_first_whatever_its_length`.
