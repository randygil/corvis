# Proposal

Implements [randygil/corvis#27](https://github.com/randygil/corvis/issues/27) (epic
[#24](https://github.com/randygil/corvis/issues/24)). The design was settled in the route posture spike
([#25](https://github.com/randygil/corvis/issues/25), `openspec/changes/archive/2026-11-08-e1-route-posture-design/`);
this change implements its task group 3 and copies the SSE and WebSocket requirements of its spec delta, adjusted for
[#116](https://github.com/randygil/corvis/issues/116).

**Depends on:** #26 (`e1-public-metadata-and-startup-check`: `#[public]`, the HTTP check, the report model) and #116
(`e13-exception-filters-and-global-guards-on-gateways`: global guards run on gateway handlers unless the application
keeps global enhancers HTTP-only). This change is written assuming both have landed; it must not start before them.

## Why

After #26, an unguarded controller route stops the application, but an unguarded WebSocket message handler does not:
the check does not look at gateways at all. SSE streams are checked as ordinary `GET` routes, so the error does not
say which line is a long-lived stream. Gateways are where "public by mistake" is easiest to miss: the handshake runs
no guard, a gateway with no `#[use_guards]` accepts every message, and until #116 global guards did not reach
gateways. Static file mounts, handshakes and raw `upgrade` listeners cannot be checked, but a reviewer reading the
report should at least see that they exist.

## What Changes

- **SSE:** routes declared with `#[sse]` are labelled `SSE` in the error and report (kind `PostureKind::Sse`). The
  rule is unchanged (they were already compiled routes checked by #26).
- **Gateway message handlers are checked:** in warn and enforce mode every `#[subscribe_message]` handler is accepted
  only when a guard that controls access runs for it, when it is public, or when a message exemption matches it.
  Unprotected handlers appear in the same error, as `WS  /chat  event "rooms"  ChatGateway::rooms`.
- **Guards counted are the ones that run:** with #116, global guards run on gateway handlers before the gateway's own
  guards, so they count; when the application keeps global enhancers HTTP-only (the #116 switch), they do not run
  there and do not count. The check reads the compiled handler guard list, so it follows whatever runs.
- **`#[public]` on gateways:** accepted below `#[websocket_gateway]` and on `#[subscribe_message]` methods, writing
  `PUBLIC_METADATA`; handler value first, then the gateway's.
- **Message exemptions:** `RoutePostureOptions::exempt_message(path, event, reason)`, same reason rules as #26.
- **Not-checked surfaces in the report:** each gateway handshake (with whether `verify_client` is set),
  `ServeStaticModule` mounts, and the number of raw `upgrade` listeners (when the report is read after the init
  hooks). They never make `init()` fail.
- **No user hook before a failing check:** gateways are compiled (guards resolved) before their `after_init` hooks
  run, and the check runs in between.
- Docs (EN + ES): WebSocket guards, gateways and SSE pages, and the posture section of the guards page.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `security/route-posture` (created by #26): SSE labelling, gateway message handlers, `#[public]` on gateways,
  message exemptions, not-checked surfaces, and the report and enforce requirements extended to gateways.

## Impact

- `crates/corvis/src/router/mod.rs` (`CompiledRoute` carries `sse`).
- `crates/corvis/src/websockets.rs` (`build_gateways` split into compile and init steps; `CompiledWsHandler` and
  `GatewayRuntime` keep what posture needs), `crates/corvis/src/application.rs` (`init()` order, `route_posture()`).
- `crates/corvis/src/posture.rs` (new kinds, gateway entries, `exempt_message`, `UncheckedSurface`).
- `crates/corvis-macros/src/gateway.rs`, `crates/corvis-macros/src/handlers.rs` (`#[public]`),
  `crates/corvis-macros/src/lib.rs` (`misplaced!` message for `public`).
- Tests: `crates/corvis/tests/route_posture.rs` (gateway and SSE cases, feature `websockets`).
- Docs: `docs/websockets/guards.md`, `docs/websockets/gateways.md`, `docs/http/server-sent-events.md`,
  `docs/guards.md` and their Spanish translations.
- Port apps: none of them declares a gateway or enables posture; no behavior change. Parity check still runs because
  `build_gateways` and `init()` change.

## Out of scope

- Enforcing the WebSocket handshake (requiring `verify_client` or `#[public]` per gateway): the spike's review notes
  keep it out until after this change; possible new issue for a stricter mode.
- Making global guards, filters or the HTTP-only switch work on gateways: #116.
- A per-transport access declaration on guards (`controls_access` for HTTP but not for WebSockets, see design Risks):
  possible new issue.
- Posture for raw `upgrade` listeners of `corvis::ws`, middleware-mounted `serve_static`, and Swagger/Scalar UIs
  (not detectable; possible new issue to give middleware and upgrade listeners a name and posture).
- `corvis posture` and report mode in the CLI (#28); the docs page and the tertulia example (#29).
- Duplicate `#[subscribe_message]` events on one gateway silently replace each other (`handlers.insert(event, ..)`,
  `crates/corvis/src/websockets.rs:580`): possible new issue.
