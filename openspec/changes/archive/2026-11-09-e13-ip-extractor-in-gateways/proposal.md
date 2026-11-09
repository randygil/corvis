# Proposal

Implements [randygil/corvis#160](https://github.com/randygil/corvis/issues/160) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Depends on #158 (`e13-websocket-handshake-honors-trust-proxy`,
archived on `main`), which made the handshake request resolve the client address with `set_trust_proxy`. The issue
body suggests the slug `e13-ip-parameter-in-gateway-handlers`; this change uses `e13-ip-extractor-in-gateways`, the
name it was assigned when drafted.

## Why

`#[ip] ip: String` works in controller handlers (`crates/corvis-macros/src/controller.rs:195-207`,
`crates/corvis/src/router/mod.rs:828`), but a gateway message handler only recognizes `#[connected_socket]` and
`#[message_body]` / `#[payload]` (`crates/corvis-macros/src/handlers.rs:165-177`). Any other attribute is left on the
parameter, so `#[ip]` in a `#[subscribe_message]` handler fails with rustc's generic "cannot find attribute `ip` in this
scope", and the handler has to take `#[connected_socket] client: WsClient` just to call `client.handshake().ip()`.
Code moved between a controller and a gateway (an audit log, a per-IP check) cannot keep its parameter list.

## What Changes

- `#[ip]` on a parameter of a `#[subscribe_message]` handler receives the client address of the connection's
  handshake, `client.handshake().ip()`: the socket address, or the forwarded client with `set_trust_proxy` (#158). It
  is the full address (not grouped by IPv6 prefix), converted to the parameter type the same way a controller's
  `#[ip]` is. It can be combined with `#[message_body]` and `#[connected_socket]` in any order.
- `#[ip]` in a gateway takes no arguments and no pipes; gateway, handler and global pipes do not run on it, as they do
  not run on `#[connected_socket]`.
- Clear compile errors where it cannot apply: `#[ip]` on a parameter of an `#[on_event]`, `#[cron]`, `#[interval]` or
  `#[timeout]` method in `#[handlers]` (there is no client), and `#[ip(..)]` with arguments in a gateway handler. Both
  messages name the method.
- **Divergence from the original, documented:** in the original framework `@Ip()` in a gateway handler resolves to
  `null`, because the WebSocket parameter factory only knows the socket, the payload and the ack (design.md,
  Context). Corvis gives the handshake address.
- Tests (integration and `compile_fail` doctests) and docs (EN + ES) on gateway parameters.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `ws/gateways`: adds the `#[ip]` gateway handler parameter and the compile errors where it cannot apply.

## Impact

- `crates/corvis-macros/src/handlers.rs`: the parameter loop recognizes `ip` (role `ip`, no arguments); the
  `#[subscribe_message]` expansion binds it without consuming a body index; other handler kinds report it.
- `crates/corvis/src/__private.rs`: a `#[cfg(feature = "websockets")]` helper that reads the handshake IP from the
  type-erased client and converts it like `extract_arg` does for `ParamKind::Ip`.
- `crates/corvis-macros/src/lib.rs`: the `#[handlers]` rustdoc lists `#[ip]`.
- Tests: `crates/corvis/tests/trust_proxy.rs` (gateway `#[ip]` with and without trust), `crates/corvis/tests/websockets.rs`
  (mixed parameters, pipes not applied, conversion error); `compile_fail` doctests in `crates/corvis/src/doctests.rs`.
- Docs: `docs/websockets/gateways.md` (parameters table), `docs/websockets/pipes.md` (pipes run on
  `#[message_body]` only), and their copies under `docs/es/`.
- No behavior change for existing code: today `#[ip]` on a gateway parameter does not compile. The port apps do not
  use gateways; apidiff should not move.

## Out of scope

- `#[ip]` in `handle_connection` / `handle_disconnect` (trait methods, not expanded by `#[handlers]`): they receive the
  client and read `client.handshake().ip()`.
- Clear compile errors for the other controller-only parameter attributes (`#[query]`, `#[headers]`, `#[req]`, ...) on
  gateway parameters; they keep rustc's "cannot find attribute" error. Possible new issue.
- Gateway equivalents of `#[query]` / `#[headers]` reading the handshake: possible new issue.
- Pipes on `#[connected_socket]` (the original runs them on every WebSocket parameter): unchanged.
