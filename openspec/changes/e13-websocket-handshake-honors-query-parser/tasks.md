# Tasks

Prerequisite: #158 (`e13-websocket-handshake-honors-trust-proxy`) is archived on `main` (it is, at `eb8bed1`). Line
numbers in design.md refer to that commit.

## 1. Failing tests

- [x] 1.1 In `crates/corvis/tests/websockets.rs`, add a gateway (`#[websocket_gateway(path = "/q")]`) with a `query`
  message that replies `{ "query": client.handshake().query(), "raw": client.handshake().uri().query() }`, a helper
  that starts the app with an optional `QueryParser`, and tests for the scenarios "Extended parser", "Default parser"
  and "Disabled parser"; verify with `cargo test -p corvis --test websockets` that the extended and disabled tests fail
  and the default one passes
- [x] 1.2 Add the "verify_client sees the parsed query" scenario (a gateway whose `verify_client` checks
  `req.query().get("auth").and_then(|a| a.get("token")).is_some_and(Value::is_string)`) and the "Trust setting is
  kept" scenario (`Extended` plus `set_trust_proxy(1)` and `X-Forwarded-For`); verify the `verify_client` test fails
  and the trust test passes
- [x] 1.3 In `crates/corvis/tests/http_adapter.rs`, on a small module of its own, add an `upgrade` listener that
  answers through its `Respond` decision with the JSON of `req.request().query()`, `req.query_param("token")` and
  `req.query_string()` in headers; tests for "Upgrade listener with the extended parser" and "Upgrade listener with
  the parser disabled"; verify both fail

## 2. Implementation

- [x] 2.1 Change `Request::for_upgrade` in `crates/corvis/src/http/request.rs` to take `&BodyParserOptions` and build
  the request with `with_parser` and an empty body, returning `Err` on `body_error` (design.md, Decision 1); update
  `upgrade_tests` to pass options (default, `Extended`, a trust setting) and check `query()` and `ip()`; verify
  `cargo test -p corvis --lib request`
- [x] 2.2 Change `websockets::upgrade` and `http_adapter::dispatch_upgrade` to take `&BodyParserOptions`, and pass
  `&shared.http_settings.body_parser` from `handle` in `crates/corvis/src/server.rs` (Decision 2); verify all tests of
  section 1 pass, and `cargo test -p corvis --test websockets --test ws_enhancers --test ws_security --test
  http_adapter --test trust_proxy` still passes (the #158 trust tests included)
- [x] 2.3 Correct the rustdoc of `UpgradeRequest::query_param` in `crates/corvis/src/http_adapter.rs` (Decision 5);
  verify `cargo doc -p corvis --no-deps` has no broken intra-doc links
- [x] 2.4 Check the build without default features (`websockets` off): `cargo check -p corvis --no-default-features`
  has no unused-variable warning in `handle`

## 3. Docs

- [x] 3.1 `docs/websockets/gateways.md`: in the `client.handshake()` row, state that the query string is parsed with
  `set_query_parser` like HTTP requests and that `uri().query()` keeps the raw string; add the note on the difference
  with a Node gateway (design.md, Decision 4) and on `Disabled` (read `?token=` from `uri().query()`) next to
  Tertulia's token example; same in `docs/es/websockets/gateways.md`
- [x] 3.2 `docs/websockets/adapter.md` (the `req.query_param("x")` row: parsed with the application's query parser;
  `req.query_string()` is raw) and `docs/http/performance.md` (the `req.query` parser row also covers WebSocket
  handshakes); same in `docs/es/`
- [x] 3.3 Run `npm run docs:check` in `docs/`; verify it passes

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes, including the regression tests of section 1 and the unit test of 2.1
- [x] 4.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 4.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences

## 5. Security round

- [x] 5.1 Share the handshake through `Arc<Request>` in `WsClient` and `ExecutionContext::for_ws`, and correct the
  `Request` clone comment
- [x] 5.2 Test that every message sees the same handshake `Request` (`messages_share_the_handshake_and_its_resolved_address`)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #159, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
