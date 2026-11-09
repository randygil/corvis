# Tasks

Prerequisite: #120 (`e13-rate-limit-headers-in-the-throttler`) is implemented and archived, so
`openspec/specs/security/rate-limiting/spec.md` holds the requirements this change modifies, the guard already sets
the headers through one helper, and `crates/corvis/tests/fixtures/throttler/` exists. If it is not, stop and implement
#120 first.

## 1. Reference behavior and failing tests

- [x] 1.1 Check the `@nestjs/throttler` version pinned by #120's `crates/corvis/tests/fixtures/throttler/package.json`;
  if it is not 6.7.1, re-read its `dist/throttler.service.js` and `dist/throttler.guard.js` against design.md
  (Context, steps 1-7) and record any difference there before going on; verify by quoting the version in design.md
- [x] 1.2 Add `crates/corvis/tests/fixtures/throttler/storage.mjs`: import `ThrottlerStorageService`, replace
  `Date.now` with a controlled clock, run the sequences of design.md, Decision 9 (storage), write `storage.json` with
  the inputs and the returned record of every call, and call `onApplicationShutdown` at the end; verify by running
  `node storage.mjs` once (it starts no server) and committing the JSON (not `node_modules`)
- [x] 1.3 Extend #120's `generate.mjs` with the HTTP sequence of design.md, Decision 9 (a 2-second throttler that
  blocks, a retry during the block, a request after it; offsets at least 300 ms from any expiry) and regenerate
  `throttler.json`; verify the new entries show `Retry-After` values and the request after the block succeeds
- [x] 1.4 Add failing unit tests in `crates/corvis/src/modules/throttler.rs`: the `storage.json` replay through
  `record_hit_at`, and one clock-driven test per storage scenario of the spec (double burst, rejected requests not
  counted, block end, block longer than ttl, block duration 0, limit 0, round-robin keys and blocked key under flood, idle keys removed, unknown name
  panics); verify with `cargo test -p corvis --lib throttler` that they fail to compile or fail for the expected reason
- [x] 1.5 Add failing integration tests next to #120's throttler tests, one per guard scenario of the spec that needs
  the HTTP stack (`Retry-After: 60` on the first block, retry during the block, default following `THROTTLE_TTL`,
  metadata override, configured from longest to shortest); keep real waits to the 1-2 second throttlers the scenarios
  allow; verify they fail for the expected reason

## 2. Storage

- [x] 2.1 Replace `Counters` with per-key records `{ hits: VecDeque<_>, window_end, blocked_until }` and an index
  ordered by idle time; add `ThrottlerRecord`, `record_hit` and the private `record_hit_at` following design.md,
  Decisions 1, 2, 5 and 7 (same `now` for every time computed in one call); re-implement `increment` on top of it;
  verify the replay test and the storage unit tests of 1.4 pass, and the existing `storage_memory_is_bounded` and
  `counts_per_key_and_resets_after_ttl` still pass
- [x] 2.2 (revised by the security review follow-up: fixed buckets of at most 64 per key replace `with_max_hits`) Eviction by idle time, blocked keys last, when the cap is
  reached, and buckets in `memory_usage`; verify the round-robin, flood and idle-key tests pass
- [x] 2.3 Add `with_block_duration(name, ms)` (panics on an unknown name) and the stable sort by `ttl` in
  `ThrottlerStorage::new`; verify the unknown-name test passes and, if #119 has landed, that its
  `From<Vec<ThrottlerOptions>>` still goes through `new`

## 3. Guard

- [x] 3.1 Add `THROTTLE_BLOCK_DURATION` (`"THROTTLER:BLOCK_DURATION"`) and export it with `ThrottlerRecord` in
  `crates/corvis/src/lib.rs` (the `modules::throttler` re-export and the prelude); verify
  `use corvis::prelude::*;` reaches both in an integration test
- [x] 3.2 In `ThrottlerGuard::can_activate`, compute the effective block duration (metadata, storage value, effective
  `ttl`), call `record_hit`, reject when `is_blocked`, and feed #120's header helper with `total_hits`,
  `time_to_expire` and `time_to_block_expire`; verify the integration tests of 1.5 pass, and
  `crates/corvis/tests/modules.rs` and `crates/corvis/tests/input_hardening.rs` still pass unchanged
- [x] 3.3 Make #120's replay compare `Retry-After` by value within one second and replay the new sequence of 1.3;
  verify it passes

## 4. Docs

- [x] 4.1 `docs/security/rate-limiting.md`: rewrite "How counting works" (sliding window per key, block from the moment
  the limit is exceeded, rejected requests not counted, throttlers checked by ascending `ttl`); add a "Blocking"
  section (default block duration, `with_block_duration`, `THROTTLE_BLOCK_DURATION`, `0` for no block, `limit` 0 and
  its difference from the original release); add `THROTTLE_BLOCK_DURATION` to the metadata table; update "Storage"
  (the `max_entries` cap, the 64-bucket worst case per key, eviction under attack); update the "Response headers" section of
  #120 (`Retry-After` is the time left in the block); same in `docs/es/security/rate-limiting.md`
- [x] 4.2 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes, including the regression tests of 1.4 and 1.5 and both differential tests
  (storage replay and HTTP replay)
- [x] 5.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 5.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences (none uses the throttler)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review (after #120's change is archived).
- Close issue #135, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
