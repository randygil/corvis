# Proposal

Implements [randygil/corvis#116](https://github.com/randygil/corvis/issues/116) (epic [#13](https://github.com/randygil/corvis/issues/13)).
Design notes come from the #23 spike, feature 2
(`openspec/changes/archive/2026-11-08-e13-missing-core-features-users-expect/design.md`).

## Why

WebSocket gateways ignore every global enhancer. `build_gateways` receives the router configuration that holds the
global guards, interceptors, pipes and filters and never reads it, so a global `AuthGuard` registered with
`Provider::app_guard` protects every HTTP route and no WebSocket message. Nothing warns about it: the gap is only
documented. Error frames also cannot be shaped with an exception filter: `#[use_filters]` below
`#[websocket_gateway]` is a compile error, and the only options are `WsException::object` or an interceptor.

A user migrating an application from the original framework expects the opposite on both counts: there, global
guards, interceptors and pipes reach gateway handlers in a non-hybrid application, and `@UseFilters()` works on
gateways and their handlers.

## What Changes

- `#[use_filters(...)]` is accepted below `#[websocket_gateway]` and on `#[subscribe_message]` methods, using the same
  `ExceptionFilter` trait as HTTP. Filters are tried handler first, then gateway, then the built-in mapping.
- The `Response` a filter returns is turned into the `exception` frame: its JSON (or text) body becomes the frame's
  `data`, and the `id` of the incoming frame is still echoed. An empty body means the filter already answered (for
  example with `client.emit`), and nothing else is sent.
- **Behavior change:** global guards, interceptors and pipes (`use_global_guards`, `use_global_interceptors`,
  `use_global_pipes`, `Provider::app_guard`, `app_interceptor`, `app_pipe`) apply to gateway handlers, ahead of the
  gateway's and the handler's own enhancers, as they do in the original framework. Global exception filters stay
  HTTP-only, also as in the original (see design.md, Decision 4 and Open questions).
- An application switch, `app.apply_global_enhancers_to_gateways(false)`, keeps global enhancers HTTP-only for apps
  that migrate gradually.
- `examples/cats-app` marks its `/chat` gateway public so the global `AuthGuard` keeps letting `websocat` clients in.
- A differential fixture records the original framework's behavior (global guard on a gateway, filter order, frame
  shape) and a Rust test replays it.
- Docs (EN + ES): websockets exception filters, gateways, guards, pipes, interceptors; migration guide; deployment
  checklist; rate limiting limitations.

## Capabilities

### New Capabilities

- `ws/exception-filters`: exception filters on gateways and handlers, their order, and how a filter's response becomes
  the `exception` frame.
- `ws/gateways`: which global enhancers reach gateway handlers, in what order, and the switch to keep them HTTP-only.

### Modified Capabilities

None (no spec covers gateways yet).

## Impact

- `crates/corvis-macros/src/gateway.rs` (accept `#[use_filters]`), `crates/corvis-macros/src/handlers.rs` (accept
  `#[use_filters]` on `#[subscribe_message]`), `crates/corvis-macros/src/lib.rs` (misplaced-attribute message).
- `crates/corvis/src/discovery.rs` (`WsEnhancers::filters`), `crates/corvis/src/websockets.rs` (`build_gateways`,
  `run_handler`, `dispatch`), `crates/corvis/src/router/mod.rs` (`RouterConfig` flag),
  `crates/corvis/src/application.rs` (the switch).
- `examples/cats-app/src/chat.rs`.
- `crates/corvis/tests/websockets.rs` and a new fixture under `crates/corvis/tests/fixtures/ws_gateways/`.
- Docs: `websockets/exception-filters.md`, `websockets/gateways.md`, `websockets/guards.md`, `websockets/pipes.md`,
  `websockets/interceptors.md`, `migration-guide.md`, `deployment.md`, `security/rate-limiting.md` and their Spanish
  translations under `docs/es/`.
- Users: apps with a global guard, interceptor or pipe and a gateway see them run on every message after upgrading.
  The docs and the change summary say how to opt out per gateway (metadata the guard checks, or
  `ctx.get_type()`) or for the whole app (the switch).
- Port apps: none of consi, facten or houseplan has a gateway, so apidiff should not move; the parity check confirms
  it.
- Route posture (#25, #27) relies on the rule this change fixes: which guards protect a gateway handler.

## Out of scope

- Global exception filters on gateways: the original does not apply them (design.md, Decision 4). An opt-in can be a
  new issue if users ask for it.
- The `cause` field (`{ pattern, data }`) that the original's base WebSocket filter adds to string and unknown-error
  frames: Corvis frames have no `cause`; possible new issue (parity of the built-in frame).
- Mapping a plain `HttpException` (for example `BadRequestException` from a global `ValidationPipe`) like the original,
  which reports it as an unknown error; Corvis keeps today's `{ status, message, ...response }` (`exception_payload`).
  Possible new issue.
- Route posture for gateways (#27) and an IoAdapter / socket.io transport (#35): they reuse this pipeline.
- A separate `WsExceptionFilter` trait: rejected in the spike review.
- Throttler response headers in a WebSocket context: #120 decides that (no headers).
