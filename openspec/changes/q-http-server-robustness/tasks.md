# Tasks

Land after nothing in particular: this change is independent of `e3-authentication-hot-path-and-consolidation` and
`e2-socket-io-hot-path-and-api-hygiene` (design.md "Order with the other changes"), and before the other three
`q-http-*` changes. Shared Cargo target directory only (never a target directory per task). Test servers bind to
`127.0.0.1`. Timing rule for every new test: the paused clock or a condition with a deadline that only fails the test;
never `sleep` to synchronize, never compare durations between runs. Behavior must not change except where design.md
"Behavior visible to applications" says so.

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "Findings as checked" against `dev`; record moved lines and any
  conflict with changes merged meanwhile (e3 in `http/res.rs`, e2 in `server.rs`) in a new "Implementation notes" section
  of design.md before any other task; verify `cargo test -p corvis` passes on the untouched tree

## 2. Accept errors (design.md Decision 1)

- [ ] 2.1 `crates/corvis/src/server/accept.rs`: crate-private `Acceptor` trait implemented for `TcpListener`,
  `is_connection_error`, the backoff (5 ms doubling to 1 s, reset on success, interruptible by the shutdown future, no
  permit held while waiting) and the once-a-minute warning with the count; `serve` uses it; verify
  `cargo test -p corvis --test transport_security --test http2_limits`
- [ ] 2.2 Unit tests in `server/accept.rs` on the paused clock for the three scenarios of the spec requirement "Accept
  errors do not spin" (schedule, one line per burst, immediate retry of a connection error, shutdown during a wait);
  verify they pass 5 runs in a row

## 3. Shutdown (Decision 2)

- [ ] 3.1 Failing test first: `crates/corvis/tests/server_lifecycle.rs`, scenario "Handler still running at the deadline"
  (order started, dropped, hook) with `listen_until_in_background`; verify it fails on the untouched tree and record the
  observed order in Implementation notes
- [ ] 3.2 `ServerOptions::shutdown_timeout(Option<Duration>)` (default 10 s, unrepresentable durations are `None`,
  rustdoc of what is and is not cancelled); `crates/corvis/Cargo.toml`: `tokio-util = { version = "0.7", features =
  ["rt"] }` (check `Cargo.lock` keeps 0.7.20 and `cargo deny check` passes)
- [ ] 3.3 `serve`: `TaskTracker` and `CancellationToken`, `spawn_tracked` for the plain connection task, the TLS
  handshake-and-connection task and the drains of `handle` (through `ConnCtx`), the sequence of Decision 2 and the
  warning line with the number of connections cut; verify 3.1 passes and `cargo test -p corvis --test tls --features tls`
- [ ] 3.4 The other scenarios of the spec requirement "The shutdown ends what the server started" in
  `tests/server_lifecycle.rs` (graceful finish within the deadline, `None` waits, drain in progress, TLS client that sends
  nothing); verify `cargo test -p corvis --test server_lifecycle` and `--features tls --test tls` 5 runs in a row

## 4. `#[res]` responses nobody can end (Decision 3)

- [ ] 4.1 Failing test first in `crates/corvis/tests/res.rs`: a handled handler that returns without ending the
  response gets a response within the test's deadline; verify it fails (times out) on the untouched tree
- [ ] 4.2 `http/res.rs`: `impl Drop for Res` (notify when a handle goes and the response is not ended),
  `wait_end_or_orphaned`, `finish` with one notification, `take_response` with `mem::take`; `router/mod.rs`: remove the
  extension after a handled pipeline, answer `Orphaned` with the generic `500` and one `ExceptionsHandler` line; rustdoc
  of `Res` and of `HttpArgumentsHost::get_response`; grep the repository and the three port apps for `get_response()`
  called after a handler returned and record the result; verify 4.1 passes
- [ ] 4.3 Tests for the scenarios of the spec requirement "An `@Res()` response nobody can end is answered" (task that
  ends it later, task that drops its handle, passthrough unaffected); verify `cargo test -p corvis --test res` and the
  replay of `fixtures/res_express.json` (`later` case) pass

## 5. `from_hyper` (Decision 4)

- [ ] 5.1 `from_hyper` returns `Request`; remove `Err(resp) => resp` and the `allow`; verify
  `cargo clippy -p corvis --all-targets -- -D warnings`

## 6. One request path (Decision 5)

- [ ] 6.1 `PendingBody.body` as a boxed body; `from_hyper` generic over the body type; `multer::BodySource::Stream` with
  the boxed body; verify `cargo test -p corvis --test transport_security --test multer`
- [ ] 6.2 `server::respond` extracted from `handle`; `handle` keeps the Engine.IO, gateway and upgrade routing; verify
  `cargo test -p corvis --test http2_limits --test response_memory --test send_timeout --test compression --features
  compression`
- [ ] 6.3 `TestClient` keeps a copy of the application's `ServerOptions`; `TestRequest::send` builds a
  `hyper::Request<Full<Bytes>>` and calls `respond`; tests for the scenarios of the spec requirement "The test client
  reads a request like the server" (413 by body, 413 by declared length, multipart as a stream, same answer as a listening
  server for the routes of `tests/express_parity.rs`); verify they pass
- [ ] 6.4 Run `cargo test --workspace` and every per-feature suite; fix each test that relied on the test client
  ignoring a limit by setting the limit it needs, or record it as a finding in Implementation notes; verify the full run
  passes

## 7. Split `server.rs` (Decision 6)

- [ ] 7.1 Record `cargo test -p corvis --features tls,compression,socket-io -- --list | wc -l` and the same for the
  default features in Implementation notes, before any move
- [ ] 7.2 Move to `server/options.rs`, `server/request.rs` and `server/response.rs`; verify `cargo build -p corvis
  --all-targets` without warnings and `--test transport_security --test express_parity`
- [ ] 7.3 Move to `server/budget.rs`, `server/rapid_reset.rs` and `server/guarded_body.rs`; verify `--test
  http2_limits --test response_memory` and `cargo test -p corvis --lib server`
- [ ] 7.4 Move to `server/timed_io.rs` and `server/watchdog.rs`; verify `--test http1_framing --test send_timeout` and
  `--features tls --test tls --test transport_security`
- [ ] 7.5 `ConnState` grouped into `Activity`, `EndFlags`, `CancelState`, `Clocks` and `ResponseBudget`, orderings and
  lock scopes unchanged; verify the suites of 7.2-7.4 again and that the test counts equal those of 7.1

## 8. Docs and review

- [ ] 8.1 Docs: `shutdown_timeout` and what it cancels in `docs/deployment.md` and `docs/fundamentals/lifecycle-events.md`;
  the orphaned `#[res]` response and `get_response()` after the handler in `docs/controllers.md`; translate in
  `docs/es/`; verify `npm run docs:check` in `docs/`
- [ ] 8.2 Security review with the `security-reviewer` agent: accept backoff (a client cannot trigger the wait: only
  non-connection errors), shutdown cancellation (no task of the server after `serve`, no permit or budget leaked by a
  cancelled connection), the boxed pending body (413 by length and while streaming, drain bounds, `client_aborted` over
  HTTP/2), the split read with `git diff --color-moved=zebra`; repeat rounds until no open finding; record findings and
  fixes under "Security review" in design.md; verify each finding has a regression test or a recorded reason

## 9. Definition of done

- [ ] 9.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 9.2 `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo clippy -p corvis --no-default-features
  -- -D warnings`
- [ ] 9.3 `cargo test --workspace` passes; `cargo test -p corvis --features tls,compression` passes
- [ ] 9.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for websockets, compression, tls,
  health-tls, auth-local, auth-api-key, auth-session, auth-oauth2 and socket-io, with no warnings
- [ ] 9.5 Regression tests: one per spec scenario, as listed in groups 2-6; the failing tests of 3.1 and 4.1 were seen
  failing before their fix
- [ ] 9.6 Docs pages updated in English and Spanish, then `npm run docs:check` in `docs/`
- [ ] 9.7 Framework runtime code changed (`server.rs`, `server/`, `router/mod.rs`, `http/res.rs`, `testing.rs`): run the
  `corvis-parity` check and verify the three port apps keep 0 apidiff differences
- [ ] 9.8 `cargo deny check` and `node scripts/check-naming.mjs` pass
- [ ] 9.9 The security review of 8.2 is closed; a "Verification" table (checks and results, as in the archived changes)
  is added to design.md

## Workflow follow-up

- Archive the change with `/opsx:archive` (the new `http/server-lifecycle` capability and the deltas of `http/responses`
  and `core/testing` applied).
- Close the issue, tick it in epic #187 and move the card to Done (see the `corvis-task` skill).
- Open the follow-ups the maintainer accepts (closing upgraded WebSocket sessions is already
  `q-core-lifecycle-and-cache-correctness`): rewriting the `sleep` calls of the server's integration suites; the watchdog's single timer if a profile shows it.
