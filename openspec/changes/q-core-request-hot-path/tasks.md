# Tasks

Start after `q-core-lifecycle-and-cache-correctness` and `q-http-request-hot-path` are merged into `dev` (design.md
Review note 5 for the case where the HTTP change slips). Group 8 also waits for #35 (`e2-socket-io-gateway-adapter`) and
for task 5.4 of `e2-socket-io-hot-path-and-api-hygiene`. Shared Cargo target directory only (never a target directory
per task). No test compares timings or synchronizes with `sleep`; counting-allocator tests live in their own test binary.
Test servers and the benchmark bind to `127.0.0.1` only. Behavior must not change except where design.md "Behavior visible
to applications" says so.

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "The findings, confirmed against the code" against `dev` with the HTTP
  change merged (`router/mod.rs` and `http/request.rs` in particular: what the O(1) clone changed, whether `route_path`
  moved); write moved lines and items the HTTP change already resolved in a new "Implementation notes" section of design.md
  before any other task; verify `cargo test -p corvis` passes on the untouched tree

## 2. Benchmark (design.md Decision 1)

- [ ] 2.1 `bench/pipeline/` workspace member (`Cargo.toml` with `publish = false`, added to the workspace `members`):
  `corvis-pipeline-bench` with the counting allocator, the `allocs` mode and its nine scenarios (`plain`, `body-1k`,
  `body-64k`, `cors`, `middleware`, `middleware-miss`, `throttler-1`, `throttler-3`, `request-scope`) and the `contention`
  mode (1, 4, 8 workers, `plain` and `throttler-1`); verify `cargo build --release -p corvis-pipeline-bench` without
  warnings and one manual run of each mode
- [ ] 2.2 `bench/pipeline/run.sh` (build once, both modes, three runs, median with min and max, `SKIP_BUILD`, the
  missing-binary message) and `bench/pipeline/README.md` (what it measures and what it does not, variables, where the binary
  is built); `bench/pipeline/raw/` in `.gitignore`; link from `bench/README.md`; verify the scenarios of the spec
  requirements by hand
- [ ] 2.3 Before run on the untouched tree; copy the tables into Implementation notes ("Benchmark before"), with the check
  of design.md Decision 2 (`cors − plain` at most 4 and independent of the header count; a gap is recorded for the owner
  of `q-http-request-hot-path`, not fixed here); verify `git status` shows no `bench/pipeline/raw/` file

## 3. Router (Decision 4)

- [ ] 3.1 `lookup` with `Cow` normalization and borrowed candidates; `select_route` without collecting; `HashMap` grouping
  at startup; `AppShared::container` removed; unit test for a mixed-case path with a trailing slash; verify the routing
  suites (`enhancers_routing.rs`, `express_parity.rs`) and `cargo test -p corvis --lib router`

## 4. Middleware matcher (Decision 3)

- [ ] 4.1 `common/middleware.rs`: `PathPattern` (classification at construction, allocation-free `matches` over slices of
  the path); property test in `fuzz_tests.rs` comparing it with `path_matches` (10,000 cases, the grammar of Decision 3);
  verify the property test and the unit tests of `middleware.rs`
- [ ] 4.2 `build_middleware` compiles and deduplicates the patterns; `CompiledMiddleware::applies` uses them; `dispatch`
  reuses the global-only chain built at `init()`; test that an unmatched path runs a `for_routes("*")` middleware; verify
  the middleware tests of `enhancers_routing.rs` and `cargo test -p corvis --test app`

## 5. Reflector (Decision 5)

- [ ] 5.1 `common/reflector.rs`: `get` and `get_all_and_merge` deserialize from `&Value`; unit test comparing every JSON
  type with `from_value`; verify `cargo test -p corvis` and the examples' roles guards (`cargo test --workspace`)

## 6. Throttler (Decision 6)

- [ ] 6.1 Property test (run first, it must pass on the old code too): the digest of the parts equals `sha256` of the
  formatted key for generated names (multi-byte included), class and handler names, trackers and generator outputs, for
  the four key forms; verify it in `cargo test -p corvis --lib throttler`
- [ ] 6.2 `ThrottlerKeys` built with the storage (override keys, skip key, `HeaderName`s or `None`), re-built where the
  throttlers are re-sorted; `override_value`, `number_override`, `is_skipped`, `set_headers` use them; crate-private
  `HttpArgumentsHost::set_response_header_value` with the rules of `set_response_header`; verify `throttler_headers.rs`,
  `throttler_skip_log_ws.rs` and the unit tests of `throttler.rs`
- [ ] 6.3 `record_hit_parts` and `record_hit_hashed`; the guard hashes the key in parts with the length in a stack buffer and
  borrows the normalized address; verify every test of the table in design.md Decision 6 passes unchanged, including
  `replays_the_original_storage_fixture`
- [ ] 6.4 Run the `contention` mode; record the ratios in Implementation notes; if Decision 6's threshold is crossed, draft
  the follow-up issue text (sharding with the invariants of Decision 6) in Implementation notes for the maintainer

## 7. Request scope (Decision 7)

- [ ] 7.1 `di/provider.rs`: `Provider::name` as `Arc<str>`; `di/container.rs`: `RequestScope::get` looked up before the
  context and the future are built; `di/module_ref.rs` and `current_resolution_path` with static values; `common/param.rs`:
  name and logger only on error paths; verify `cargo test -p corvis --test di`, the unit tests of `di/request_scope.rs` and
  `--test scoped_error_log`

## 8. WebSocket gateways (Decision 8)

- [ ] 8.1 Re-read `websockets.rs` as merged after #35 and the `tokio-tungstenite` upgrade; record in Implementation notes
  the lines that replace the ones cited in design.md and the shared text type chosen
- [ ] 8.2 `crates/corvis/tests/ws_broadcast_memory.rs` (own binary, counting allocator, `127.0.0.1`): the three figures of
  Decision 8 and the value that fails to serialize; run it and record that it fails
- [ ] 8.3 One encoding per broadcast (`Frame` serialized from the caller's value), shared text per recipient, iteration
  under the read lock with `try_push` and closes after the lock; `WsClient::emit` without `to_value`; a serialization
  failure logged and sent to nobody; verify the gateway suites (`websockets.rs`, `ws_security.rs`, `ws_enhancers.rs`,
  `auth_ws.rs`, `send_timeout_ws.rs`)
- [ ] 8.4 Room index with the lock order of Decision 8, `removed` flag, empty rooms deleted; churn test (8 tasks, 1,000
  clients, 50 rooms); lock order in the module docs; verify the churn test 5 runs in a row and the gateway suites
- [ ] 8.5 Message parsing without copies (`remove` of `event`, `data`, `id`; borrowed UTF-8 for binary frames); verify
  `ws_broadcast_memory.rs` and the gateway suites

## 9. Benchmark after

- [ ] 9.1 After run on the same machine; "Benchmark after" in Implementation notes with, per decision, the figure it had to
  move (Decision 2, a check: `cors − plain` ≤ 4 and independent of header count; Decision 3: `middleware − plain` ≤ 3,
  `middleware-miss − plain` 0; Decision 4: `plain` down by at least 3; Decision 5: the roles read at 4; Decision 6:
  `throttler-1 − plain` ≤ 6, `throttler-3 − plain` ≤ 14; Decision 7: second resolution 0, `request-scope − plain` down by
  at least 3); any figure missed is explained there; publish `bench/pipeline/results.md`
- [ ] 9.2 `crates/corvis/tests/pipeline_allocations.rs` (own binary): one test per scenario pinning the after figure plus
  10 %, and the in-handler count of a second `Scoped<T>` resolution; verify it passes 5 runs in a row

## 10. Docs and review

- [ ] 10.1 Rustdoc of `PathPattern`, `ThrottlerStorage` (keys built once, digest format unchanged), the room index lock order
  in `websockets.rs`; docs: a broadcast value that fails to serialize (logged, sent to nobody) in
  `docs/websockets/gateways.md` and `docs/es/websockets/gateways.md`; verify `cargo doc -p corvis --no-deps` without
  warnings and `npm run docs:check` in `docs/`
- [ ] 10.2 Security review with the `security-reviewer` agent (the compiled middleware matcher against `path_matches`, the
  throttler's hashing and typed header setter, the request-store lookup, the room index and its lock order); repeat rounds
  until no open finding; record findings and fixes in design.md under "Security review"

## 11. Definition of done

- [ ] 11.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 11.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 11.3 `cargo test --workspace` passes, and the fuzz properties of `fuzz_tests.rs` pass with the CI case count
- [ ] 11.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for websockets, compression, tls, socket-io,
  auth-local, auth-api-key, auth-session and auth-oauth2, with no warnings, and `cargo check -p corvis --no-default-features`
- [ ] 11.5 Regression tests: one per spec scenario, as listed in groups 3-9
- [ ] 11.6 Docs pages updated in English and Spanish, then `npm run docs:check` in `docs/`
- [ ] 11.7 Framework runtime code changed (router, middleware, throttler, DI, `websockets.rs`): run the `corvis-parity` check
  and verify the three port apps keep 0 apidiff differences
- [ ] 11.8 `cargo deny check` and `node scripts/check-naming.mjs` pass
- [ ] 11.9 The security review of 10.2 is closed before archiving

## Workflow follow-up

- Archive with `/opsx:archive` (deltas of `http/routing`, `security/rate-limiting`, `core/injection-scopes`, `ws/gateways`
  applied; new `bench/request-pipeline`).
- Close the issue, tick it in epic #187 and move the card to Done (see the `corvis-task` skill).
- Open the follow-ups the maintainer accepts: throttler sharding if task 6.4 crossed the threshold; a cap on rooms per plain
  gateway client; a borrowed path for body parameters if the pipe API ever changes.
