# Tasks

## 1. Failing tests

- [x] 1.1 In `crates/corvis/tests/trust_proxy.rs`, add a gateway (`#[websocket_gateway(path = "/who")]`)
  with a `who` message that replies `{ ip, ips, hostname }` from `client.handshake()`, a helper that connects with
  `tokio_tungstenite::connect_async` and a request built with `IntoClientRequest` plus extra headers, and tests for
  the scenarios "Gateway behind one trusted proxy", "Default setting ignores forwarded headers" and "Forwarded host";
  verify with `cargo test -p corvis --test trust_proxy` that the trusted-proxy and forwarded-host tests fail and the
  default-setting test passes
- [x] 1.2 Add the throttler scenarios: a module with `ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 2)])`
  and a global `ThrottlerGuard` (or `#[use_guards(ThrottlerGuard)]` on the gateway), two clients with different
  `X-Forwarded-For`, with and without `set_trust_proxy(1)`; and the `verify_client` scenario (accepts only
  `203.0.113.7`); verify the trusted cases fail and the untrusted case passes
- [x] 1.3 In `crates/corvis/tests/http_adapter.rs`, add an `upgrade` listener that responds with the resolved
  `req.request().ip()` in a header, with `set_trust_proxy(1)` and `X-Forwarded-For`; verify it fails

## 2. Implementation

- [x] 2.1 Add `Request::for_upgrade(parts, remote, trust) -> Result<Request, HttpException>` in
  `crates/corvis/src/http/request.rs` (design.md, Decision 1), with a unit test that `ip()` honors the setting;
  verify `cargo test -p corvis --lib request`
- [x] 2.2 Pass `&shared.http_settings.body_parser.trust_proxy` from `handle` (`crates/corvis/src/server.rs`) to
  `websockets::upgrade` and `http_adapter::dispatch_upgrade`, and build both requests with `Request::for_upgrade`
  (design.md, Decision 2); verify all tests of section 1 pass, and `cargo test -p corvis --test websockets --test
  ws_enhancers --test ws_security --test http_adapter` still passes
- [x] 2.3 Check the build without default features (`websockets` off, the case fixed by #153):
  `cargo check -p corvis --no-default-features` has no unused-variable warning for the trust setting in `handle`

## 3. Docs

- [x] 3.1 `docs/websockets/gateways.md`: in the `client.handshake()` row and the throttler limitation, state that the
  IP follows `set_trust_proxy` like HTTP requests and that `remote_addr()` keeps the socket address; add the note on
  the difference with a Node gateway behind Express (design.md, Decision 3); same in `docs/es/websockets/gateways.md`
- [x] 3.2 `docs/websockets/adapter.md` (`req.request()` and `ws.upgrade_request()` resolve the IP with
  `set_trust_proxy`) and `docs/security/rate-limiting.md` ("Behind a proxy": the setting also applies to gateway
  messages); same in `docs/es/`
- [x] 3.3 Run `npm run docs:check` in `docs/`; verify it passes

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes, including the regression tests of section 1
- [x] 4.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 4.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #158, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
