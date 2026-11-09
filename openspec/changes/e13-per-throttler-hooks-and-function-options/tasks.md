# Tasks

Prerequisite: #156 (`e13-throttle-overrides-per-throttler-name`) is archived on `main` (it is, at `eb8bed1`). Line
numbers in design.md refer to that commit.

## 1. Reference behavior and fixture

- [x] 1.1 Re-read the sort, the hook chain, `resolveValue` and `toNumber` in `dist/throttler.guard.js` of the version
  pinned by `crates/corvis/tests/fixtures/throttler/package.json` (`npm ci` in a scratch folder, no `node_modules` in
  the repository); if the version is not 6.7.1 or the behavior differs from design.md (Context, Original), record it
  in design.md before going on; verify by quoting the version in design.md
- [x] 1.2 Extend `generate.mjs` as design.md, Decision 7 says: the `Ptr` controller and the apps `throttler-tracker`,
  `throttler-key`, `dynamic-limit`, `dynamic-ttl-order` and `function-error`; regenerate `throttler.json` with
  `node generate.mjs`; verify the existing cases are byte-identical in the diff, `dynamic-ttl-order` records
  `Retry-After-b: 10` on the second response and `Retry-After-a: 60` on the third, and `function-error` records 500,
  403 and 200; commit the JSON (not `node_modules`)
- [x] 1.3 Add failing integration tests in `crates/corvis/tests/throttler_headers.rs`, one per spec scenario of the
  added and modified requirements that needs the HTTP stack (per-throttler hooks and their precedence, value
  functions, route override over a function, errors, ttl-function order, the `t:` source); verify they fail to compile
  (missing builders)

## 2. Storage API

- [x] 2.1 In `crates/corvis/src/modules/throttler.rs` add `ThrottlerValueFn<T>`, the maps and the five builders
  `with_tracker_for`, `with_key_generator_for`, `with_limit_fn`, `with_ttl_fn`, `with_block_duration_fn` (design.md,
  Decisions 1 and 3), each panicking on an unknown name; rustdoc each one with its place in the precedence chain and
  the client-controlled-input warning; verify with unit tests of the panics and that a `for_root_async` factory
  returning a storage keeps them
- [x] 2.2 Keep each throttler's configured index in the storage and re-sort in `with_ttl_fn` (Decision 4); verify with
  a unit test that static throttlers stay in ascending ttl order and ttl-function throttlers follow in configured order
- [x] 2.3 Export `ThrottlerValueFn` next to `ThrottlerTrackerFn` in `crates/corvis/src/lib.rs`; verify
  `corvis::ThrottlerValueFn` resolves in `tests/throttler_headers.rs`

## 3. Guard

- [x] 3.1 In `ThrottlerGuard::can_activate`, resolve `limit`, `ttl` and the block duration as route override, then
  function, then static value (block duration then `with_block_duration`, then effective ttl), calling a function only
  when no override applies and returning its `Err` before `record_hit` (Decisions 2 and 3); verify the value-function
  and error tests of 1.3 pass and the existing tests in `tests/throttler_headers.rs` and `tests/modules.rs` still pass
- [x] 3.2 Resolve tracker and key generator as route override, then per-throttler hook, then module hook, then
  built-in, and store per-throttler generator keys under the `t:` marker (Decisions 2 and 5); verify the hook tests of
  1.3, including "The throttler's generator and a registered generator do not meet", and that #156's
  `a_crafted_tracker_cannot_hit_another_routes_builtin_counter` still passes

## 4. Differential replay

- [x] 4.1 Declare the `Ptr` routes in `tests/throttler_headers.rs` with `throttle(..)` and named hooks, and build each
  new case's storage from the case table; verify `replays_the_original_library_fixture` passes for every case

## 5. Docs

- [x] 5.1 `docs/security/rate-limiting.md`: in "Trackers and key generators", add the per-throttler level
  (`with_tracker_for`, `with_key_generator_for`) and the four-level precedence; add a section on function-valued
  limit, ttl and block duration (signatures, precedence, not called under a route override, errors, check order of
  ttl functions, derive values from server-side state and not from client headers, keep functions cheap); same in
  `docs/es/security/rate-limiting.md`; run `npm run docs:check` in `docs/` and verify it passes

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 6.3 `cargo test --workspace` passes, including the regression tests of 1.3, 2.1 and 2.2 and the differential
  replay of 4.1
- [x] 6.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 6.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences (none uses the throttler)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #162, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

## 7. Security round (2026-11-09)

- [x] 7.1 Always consult the block table in `record_hit_at`; regression tests for `with_block_duration_fn` and `with_ttl_fn` returning 0 for a blocked key
- [x] 7.2 Warn once per throttler when a ttl function returns 0; rustdoc and docs EN + ES (stable per key, ttl 0, `Err` stops later throttlers)
