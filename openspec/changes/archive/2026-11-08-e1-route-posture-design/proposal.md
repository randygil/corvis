# Proposal

Implements [randygil/corvis#25](https://github.com/randygil/corvis/issues/25) (epic
[#24](https://github.com/randygil/corvis/issues/24)). This is a design spike: its deliverable is `design.md`, the
`security/route-posture` requirements and the work plan for the follow-up issues
[#26](https://github.com/randygil/corvis/issues/26), [#27](https://github.com/randygil/corvis/issues/27),
[#28](https://github.com/randygil/corvis/issues/28) and [#29](https://github.com/randygil/corvis/issues/29). No code
changes in this change.

## Why

Forgetting `#[use_guards(...)]` on a route is silent: the route works, the tests that call it pass, and it is open to
anyone. Nothing in Corvis distinguishes "public on purpose" from "public by mistake". Today a guard can sit at four
levels (global instance, global provider, controller, route), the framework merges them only at `init()`, gateways
ignore the global ones, and static files, raw `upgrade` listeners and documentation UIs are outside the route table
altogether. Before the epic's tasks can add a startup check, these questions need one answer: what counts as
protected, how a route says "open on purpose", what the check can and cannot see, and how existing apps (including
the three port apps that must keep parity) are affected.

## What Changes

The spike settles the design (details and alternatives in `design.md`):

- **`#[public]` attribute** on a route method, below `#[controller]`, below `#[websocket_gateway]` or on a
  `#[subscribe_message]` handler. It sets the metadata entry `PUBLIC_METADATA` (`"isPublic"`, the key apps already use
  for this) to `true`, so existing global guards that skip public routes through `Reflector` honor it unchanged.
- **Opt-in enforcement:** `app.require_route_posture()` (and `set_route_posture(RoutePostureOptions)` with `Warn`
  mode and exemptions). Off by default, so existing apps and the port apps keep their behavior.
- **The rule:** a route is accepted when at least one guard that controls access runs for it (global, controller or
  route level, exactly the guards the router runs), when it is marked `#[public]`, or when an exemption with a written
  reason matches it. Guards that only rate-limit or validate (`ThrottlerGuard`, `ZodGuard`) do not count; any guard can
  declare this through a new provided method `CanActivate::controls_access()` (default `true`).
- **Where it runs:** at the end of route and gateway compilation inside `init()` (so before `listen` binds and before
  any lifecycle hook), on the compiled route table, not on a parallel reconstruction.
- **Failure:** `init()` returns a typed `RoutePostureError` that lists every unprotected route at once (method, full
  path, `Controller::handler`) with the fix; `Warn` mode logs the same list and starts.
- **SSE** routes are controller routes and are checked like any other; **gateway message handlers** are checked with
  the gateway's and handler's guards only (global guards do not run there); the WebSocket handshake, static files, raw
  `upgrade` listeners and middleware-mounted documentation UIs are reported but not enforced.
- **Report:** `app.route_posture()` returns every route with its guards and status; `corvis posture` prints it (text or
  JSON) for code review and CI.

## Capabilities

### New Capabilities

- `security/route-posture`: the startup check, the `#[public]` marker, what counts as a guard, exemptions, the error,
  WebSocket and SSE coverage, and the posture report (API and CLI).

### Modified Capabilities

None (no spec covers guards yet).

## Impact

- This change: `openspec/changes/e1-route-posture-design/` only.
- Follow-up changes (planned in `tasks.md`):
  - #26: `crates/corvis` (guard trait, application options, posture module, `init()`), `crates/corvis-macros`
    (`#[public]` in `#[controller]` and `#[routes]`), `ThrottlerGuard` and `ZodGuard`.
  - #27: `crates/corvis/src/websockets.rs`, `crates/corvis-macros` (`#[public]` in `#[websocket_gateway]` and
    `#[handlers]`).
  - #28: `crates/corvis-cli` (`corvis posture`, the `corvis new` template), report printing in `crates/corvis`.
  - #29: `docs/security/route-posture.md` and `docs/es/security/route-posture.md`, cross-links, `examples/tertulia`.
- Port apps: no change while the option is off (default). If they opt in later: consi has 16 unguarded routes,
  houseplan 3, facten 0 (it has a global guard).

## Out of scope

- Implementing anything: each follow-up issue becomes its own change from its group in `tasks.md`.
- Turning the check on by default for existing apps: decided with the compatible vs strict mode spike (#8).
- Checking that a guard really authenticates (the check only sees that one runs): the report exists for review.
- Seeing inside middleware (authentication done in middleware is handled with exemptions, not detected).
- Posture for raw `upgrade` listeners of `corvis::ws` and for Swagger/Scalar UIs (they are not routes; possible new
  issue: give middleware and upgrade listeners a name and posture so they can be checked).
- OpenAPI `security` derived from `#[public]` (possible new issue in the OpenAPI area).
- Enabling the check in the three port apps.
