# Tasks

Start after #35 (`e2-socket-io-gateway-adapter`) and `q-core-lifecycle-and-cache-correctness` are merged into `dev`.
Shared Cargo target directory only (never a target directory per task). No test compares timings or synchronizes with
`sleep`. Every bug fix starts with its test, run once to see it fail (noted in Implementation notes). Behavior must not
change except where design.md "Behavior visible to applications" says so.

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "The findings, confirmed against the code" against `dev` with #35 and
  the lifecycle change merged (`gateway.rs`, `handlers.rs`, `websockets.rs`, `socket_io/gateway.rs`, `posture.rs`,
  `cron_job.rs`); confirm #35 made gateway arguments spanned errors; write moved lines in a new "Implementation notes"
  section of design.md before any other task; verify `cargo test --workspace` passes on the untouched tree

## 2. Results by type (Decision 1)

- [ ] 2.1 `crates/corvis/tests/return_types.rs` with the cases of Decision 1 (routes, an event handler, a gateway handler);
  run it and record that it does not compile and, for the alias case once the file compiles without it, that the route
  answers 200 `{"Err":..}`
- [ ] 2.2 `__private.rs`: `ResultKind` / `PlainKind`, `ResultTag::split`, `PlainTag::split`, the `Infallible` arm of the
  error probes; `controller.rs` and `handlers.rs` generate the split for routes, event and scheduled handlers and gateway
  handlers; `is_result` and the name test removed; verify `return_types.rs`, `cargo test -p corvis-macros` and
  `cargo test --workspace`

## 3. Attribute errors (Decision 2)

- [ ] 3.1 Macro unit tests for `#[http_code]` and `#[redirect]` with no and too many arguments, `#[set_metadata]` arity
  span, `#[module(global = CONST)]`, `#[init(a, b)]`; run and record the failures (panics and silent expansions)
- [ ] 3.2 The checks of Decision 2 in `controller.rs`, `module.rs`, `injectable.rs`; one `compile_fail` doctest per
  attribute in `crates/corvis/src/doctests.rs`; verify `cargo test -p corvis-macros` and `cargo test -p corvis --doc`

## 4. One implementation each (Decision 3)

- [ ] 4.1 `utils.rs` `enhancer_attr` used by `controller.rs`, `handlers.rs`, `gateway.rs`, each keeping its placement rules
  and messages; verify the macro unit tests and the `compile_fail` doctests
- [ ] 4.2 `common/enhancer.rs` `resolve_all` used by `router/mod.rs` and `websockets.rs`; verify `cargo test -p corvis`
- [ ] 4.3 Save the route posture report (`CORVIS_ROUTE_POSTURE_REPORT`) of the example apps before the refactor; one `entry`
  helper in `posture.rs` for routes and gateway handlers; verify the reports are byte-identical after, and `route_posture.rs`
  and the posture unit tests pass

## 5. Dead code, hygiene and typed replies (Decisions 4, 5)

- [ ] 5.1 Remove the silencing statements (`injectable.rs`, `module.rs`), fix `controller.rs:64`, the rustdoc of
  `__private::type_name_of`, `Span::mixed_site()` for `__ctx`; verify `cargo build --workspace --all-targets` without
  warnings
- [ ] 5.2 Test: a gateway handler whose reply fails to serialize answers `exception` `Internal server error` (run, record
  the failure); `WsHandlerReply` between generated code and both dispatchers; `encode_reply`, the `__corvis_same_event`
  key, the `SHAPE_*` constants and the `AtomicU8` removed from `websockets.rs` and `socket_io/gateway.rs`; serialization
  failures of replies and `WsException::object` as internal errors, `WsResponse::new` logging; verify the gateway suites
  of both adapters (`websockets.rs`, `ws_enhancers.rs`, `ws_security.rs`, `auth_ws.rs`, the Socket.IO gateway suites of #35)
  and the `ws_gateways` fixture replay

## 6. Small modules (Decisions 6-10)

- [ ] 6.1 Log literals: the no-double-space assertions in `router/cors.rs` tests and `throttler_skip_log_ws.rs` (run, record
  the failures), then `\` continuations at `router/cors.rs:194` and `throttler.rs:1105`, `:1110`
- [ ] 6.2 Unit test reporting the token count of a 20-route controller's `routes()`; record the figure in Implementation
  notes and, above 10,000 tokens, the follow-up issue draft
- [ ] 6.3 `health.rs`: configuration cached per crypto provider; unit test with `Arc::ptr_eq`; verify `--test health` and
  `--test health_panic_log`
- [ ] 6.4 `event_emitter.rs`: allocation-free wildcard matching with a property test against the old matcher; the
  mismatched-payload warning (generated code in `handlers.rs` calls a helper in `event_emitter.rs`), capped set,
  capturing-logger test; verify `cargo test -p corvis --test modules`
- [ ] 6.5 `cron_job.rs`: `try_new`, `TryFrom<CronJobParams>`, `#[doc(alias = "from")]`, `from` and its `allow` removed; every
  call in `crates/`, `docs/` and `examples/` updated; verify `--test schedule` and `cargo test --workspace --doc`

## 7. Docs

- [ ] 7.1 Scheduling page (`CronJob::try_new`, breaking change noted), the controllers or responses page (results recognized
  by type, with the alias example), and their `docs/es/` versions; verify `npm run docs:check` in `docs/`

## 8. Definition of done

- [ ] 8.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 8.2 `cargo clippy --workspace --all-targets -- -D warnings` passes (no `should_implement_trait` allow left in
  `cron_job.rs`)
- [ ] 8.3 `cargo test --workspace` passes, doctests included
- [ ] 8.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for websockets, compression, tls, socket-io,
  auth-local, auth-api-key, auth-session and auth-oauth2, with no warnings, and `cargo check -p corvis --no-default-features`
- [ ] 8.5 Regression tests: one per spec scenario, each seen failing before its fix
- [ ] 8.6 Docs pages updated in English and Spanish, then `npm run docs:check` in `docs/`
- [ ] 8.7 Framework runtime code and the macros changed: run the `corvis-parity` check and verify the three port apps build
  without warnings and keep 0 apidiff differences
- [ ] 8.8 `cargo deny check` and `node scripts/check-naming.mjs` pass
- [ ] 8.9 Security review with the `security-reviewer` agent, scoped to Decision 1 (an error must never be answered as a
  success), Decision 3 (posture and enhancer placement) and Decision 5 (gateway errors through the exception path); record
  findings in design.md under "Security review"

## Workflow follow-up

- Archive with `/opsx:archive` (deltas of `http/responses` and `core/enhancers` applied).
- Close the issue, tick it in epic #187 and move the card to Done (see the `corvis-task` skill).
- Open the follow-ups the maintainer accepts: one generated function per route if task 6.2 crossed the threshold.
