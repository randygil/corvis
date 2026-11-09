# Tasks

Prerequisite: #158 (`e13-websocket-handshake-honors-trust-proxy`) is archived on `main` (it is, at `eb8bed1`). Line
numbers in design.md refer to that commit.

## 1. Failing tests

- [x] 1.1 In `crates/corvis/tests/trust_proxy.rs`, add a gateway handler `who_ip(&self, #[ip] ip: String) -> String`
  (next to `WhoGateway`) and tests for "Behind one trusted proxy", "Default setting" and "Full IPv6 address" with the
  existing `ws_connect` / `ws_call` helpers; verify with `cargo test -p corvis --test trust_proxy` that they fail to
  compile ("cannot find attribute `ip`")
- [x] 1.2 In `crates/corvis/tests/websockets.rs`, add the "Mixed with the body and the socket" handler, a gateway with
  `#[use_pipes(CountingPipe)]` and an `#[ip]`-only handler ("Gateway pipe and an IP parameter"), and a handler with
  `#[ip] ip: u32` ("Conversion error"); verify they fail to compile for the same reason

## 2. Runtime helper

- [x] 2.1 Add `ws_ip_arg<T: DeserializeOwned>(client: &Arc<dyn Any + Send + Sync>) -> Result<T, HttpException>` to
  `crates/corvis/src/__private.rs` behind `#[cfg(feature = "websockets")]` (design.md, Decision 2), with a unit test
  that converts `"127.0.0.1"` into `String` and fails into `u32` with a `400` whose message starts with `ip:`; verify
  `cargo test -p corvis --lib __private` and `cargo check -p corvis --no-default-features` (no unused import)

## 3. Macro

- [x] 3.1 In `crates/corvis-macros/src/handlers.rs`, recognize `ip` in the parameter loop as role `ip`; reject
  arguments with the Decision 3 message; in the `#[subscribe_message]` expansion bind it with `ws_ip_arg` without
  registering a `ParamDef` and without consuming a body index; verify the tests of section 1 pass and `cargo test -p
  corvis --test websockets --test ws_enhancers --test ws_security` still passes
- [x] 3.2 Report `#[ip]` on `#[on_event]`, `#[cron]`, `#[interval]` and `#[timeout]` methods (Decision 4), combined
  with the block's other discarded-attribute errors; add `compile_fail` doctests with passing twins in
  `crates/corvis/src/doctests.rs` for an `#[on_event]` method and for `#[ip(ParseIntPipe)]`; verify with `cargo test
  -p corvis --doc doctests`
- [x] 3.3 List `#[ip]` in the rustdoc of `#[handlers]` in `crates/corvis-macros/src/lib.rs`; verify `cargo doc -p
  corvis-macros --no-deps` builds without warnings

## 4. Docs

- [x] 4.1 `docs/websockets/gateways.md`: add `#[ip] ip: String` to the parameters table (the handshake's client
  address, follows `set_trust_proxy`, no arguments or pipes) with the note on the original's `null` (design.md,
  Decision 5); same in `docs/es/websockets/gateways.md`
- [x] 4.2 `docs/websockets/pipes.md`: state that pipes run on `#[message_body]` only, not on `#[connected_socket]` or
  `#[ip]`; same in `docs/es/websockets/pipes.md`
- [x] 4.3 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes, including the regression tests of sections 1 to 3 and the doctests of 3.2
- [x] 5.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 5.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences

## 6. Security round

- [x] 6.1 Resolve the client address once per connection (`WsClient::ip()`) and read it in `ws_ip_arg`
- [x] 6.2 Make `Request::ip()` lazy (`proxy_addr::client_addr`) with a differential test against `all_addrs`, long
  headers and the edge cases
- [x] 6.3 Test that the address is the same value for every message of a connection
- [x] 6.4 `ThrottlerGuard` uses `WsClient::ip()` in WebSocket contexts, with a test that the tracker equals `client.ip()`

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #160, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
