# Tasks

Start after `e3-authentication-hot-path-and-consolidation` and `q-http-response-consistency` are merged into `dev`, and
preferably after `e2-socket-io-hot-path-and-api-hygiene` (design.md "Order with the other changes"). Shared Cargo target
directory only. The benchmark and test servers bind to `127.0.0.1`. No test sleeps to synchronize or compares timings;
benchmarks are not tests. Behavior must not change except where design.md "Behavior visible to applications" says so.

## 1. Reconcile

- [ ] 1.1 Re-check every line of design.md "Findings as checked" against `dev` (e3 and `q-http-response-consistency`
  merged, e2 if merged); record moved lines, and how e2's `Request::for_engine_poll` and e3's `typed_user` sit in
  `http/request.rs`, in a new "Implementation notes" section of design.md before any other task

## 2. Benchmark (design.md Decision 1)

- [ ] 2.1 `bench/http/` workspace member `corvis-http-bench` (`publish = false`, added to the workspace `members`): one
  route per scenario of Decision 1, a counting global allocator, a request counter, `GET /__bench/stats`, the
  `compress-step` subcommand; verify `cargo build --release -p corvis-http-bench` without warnings and a manual request to
  each route
- [ ] 2.2 `bench/http/run.sh` (status and body check per scenario, warmup, stats around a closed-loop `oha` run, the
  one-worker repetition, `--label`, `SKIP_BUILD`, the missing-binary message, `results/<label>.json`, the printed table) and
  `bench/http/README.md`; `bench/http/results/` in `.gitignore`; link from `bench/README.md`; verify the scenarios of the
  spec requirements of `bench/http-hot-path` by hand
- [ ] 2.3 Before run: `bash bench/http/run.sh --label before` and `compress-step`; copy the table into Implementation notes
  ("Benchmark before"); verify `git status` shows no results file

## 3. A request that clones in constant time (Decision 2)

- [ ] 3.1 `http/request.rs`: `Arc` fields for headers, body and query (the route path is left to
  `q-core-request-hot-path`); `headers_mut` / `set_body` copy on write; rustdoc of `Request`; adapt the
  constructors (and e2's `for_engine_poll` if merged); verify `cargo test -p corvis` and the socket-io suites with
  `--features socket-io`
- [ ] 3.2 Check that the router's clones (`req_for_cors`, `req_for_errors`) now share the parts (no change to CORS logic,
  which `q-core-request-hot-path` owns); verify the CORS tests of `tests/app.rs` and `tests/cors_startup_warning.rs`
- [ ] 3.3 `body_as` and `user_as` from references; unit test comparing the error messages of `from_value(v.clone())` and
  `T::deserialize(&v)` for a table of mismatches; verify the validation suites (`tests/zod_pipe.rs`,
  `tests/validation_transform_option.rs`)
- [ ] 3.4 `tests/request_clone.rs`: the scenarios of the spec requirement "A request shares its parsed parts with its
  clones" and the counting-allocator test of Decision 2; verify they pass

## 4. JSON on UTF-8 (Decision 3)

- [ ] 4.1 `http/json_parse.rs`: the `Source` trait with `Utf8` and `Utf16`, shared grammar, depth bound, duplicate-key
  rule and surrogate handling; literals without allocation; `JsObject` list with an index past 8 keys; verify
  `cargo test -p corvis --test json_parse` (every recorded case) and `--lib json_parse`
- [ ] 4.2 `http/body_parser.rs`: UTF-8 path after an optional BOM with `from_utf8`, fallback to today's UTF-16 decode and
  parse on any failure; `json_parse::parse` with the same rule; verify `--lib fuzz` and `--test input_hardening --test
  transport_security`
- [ ] 4.3 Property test `json_paths_agree` in `fuzz_tests.rs` (inputs of Decision 3); run it with `PROPTEST_CASES=100000`
  once and record the run in Implementation notes; verify it passes with the default case count in CI

## 5. Finishing without copies (Decision 4)

- [ ] 5.1 `http/express.rs` `finalize` moving the body, storing an empty type for sent bodies; `into_hyper` with static
  header values for its fixed types; `AppShared` keeps the prepared `X-Powered-By` built at `init`; unit test of the
  pointer of a 16 KiB text body; verify `--test express_parity --test typed_responses --test empty_results --test res` and
  `--features compression --test compression`

## 6. Small per-request costs (Decision 5)

- [ ] 6.1 `Request::cookie(name)` with an early-exit scan; unit tests of Decision 5; verify `cookies_first_occurrence_wins`
  and the auth cookie suites with `--features auth-session,auth-oauth2`
- [ ] 6.2 `TrustProxy::trusts_ip` used by `hostname()` and the proxy walk; verify `--test trust_proxy` and
  `trust_proxy_is_total_and_bounded`
- [ ] 6.3 `file_stream` with a reused buffer (`tokio-util` `io` feature or `read_buf`), `UnexpectedEof` kept; test for a
  file truncated while streamed; verify `streamable_file_from_path_streams`, `serve_static_ranges_are_exact`,
  `bodies_are_streamed_from_disk`

## 7. Static root (Decision 6)

- [ ] 7.1 `ServeStatic` caches the canonical root, re-resolving on a miss or a mismatch; test that replaces the served
  folder while the application runs; verify `--test static_response_security --test fuzz_static`

## 8. After run and docs

- [ ] 8.1 After run on the same machine: `bash bench/http/run.sh --label after` and `compress-step`; "Benchmark after"
  table in Implementation notes with, per decision, the number it had to move (design.md Decision 1 table); revert
  Decision 6 if its gate failed; explain any other figure missed; record the compression step figures and open the
  follow-up if the p99 exceeds 1 ms
- [ ] 8.2 `docs/http/performance.md`: the ETag cost and `EtagMode::Disabled`, `Request` clones share their parts, the
  benchmark and where it lives; translate in `docs/es/http/performance.md`; verify `npm run docs:check` in `docs/`
- [ ] 8.3 Security review with the `security-reviewer` agent: the two JSON paths (totality, depth, agreement on hostile
  input, invalid UTF-8, BOM handling), copy on write in `Request` (no request sees another's mutation), the cached root of
  static files (no file outside the folder after a swap, symlink rules), `cookie(name)`; repeat rounds until no open
  finding; record findings and fixes under "Security review" in design.md

## 9. Definition of done

- [ ] 9.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 9.2 `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo clippy -p corvis --no-default-features
  -- -D warnings`
- [ ] 9.3 `cargo test --workspace` passes, including `json_parse_matches_v8` unchanged
- [ ] 9.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for websockets, compression, tls,
  auth-local, auth-api-key, auth-session, auth-oauth2 and socket-io, with no warnings
- [ ] 9.5 Regression tests: one per spec scenario, as listed in groups 3-7
- [ ] 9.6 Docs pages updated in English and Spanish, then `npm run docs:check` in `docs/`
- [ ] 9.7 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences
- [ ] 9.8 `cargo deny check` and `node scripts/check-naming.mjs` pass
- [ ] 9.9 The security review of 8.3 is closed; a "Verification" table is added to design.md

## Workflow follow-up

- Archive the change with `/opsx:archive` (the new `bench/http-hot-path` capability and the delta of
  `http/request-parsing` applied).
- Close the issue, tick it in epic #187 and move the card to Done (see the `corvis-task` skill).
- Open the follow-ups the numbers ask for: compression off the worker (if a step exceeds 1 ms), the watchdog's single
  timer (if a profile shows it).
