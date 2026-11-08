# Proposal

Implements [randygil/corvis#29](https://github.com/randygil/corvis/issues/29) (epic
[#24](https://github.com/randygil/corvis/issues/24)). The design was settled in the route posture spike (#25,
`openspec/changes/archive/2026-11-08-e1-route-posture-design/`, its design including the review notes, and task
group 5). This change implements that task group.

## Why

#26–#28 add the route posture check, the `#[public]` marker, gateway and SSE coverage, the report API and
`corvis posture`. Users learn about guards from `docs/guards.md` and the Security section, and none of those pages will
say that the framework can refuse to start with an unprotected route, how to mark a route as open on purpose, what the
check cannot see, or how to migrate an existing application. The docs also promise an example for every user-facing
feature, and `tertulia` is the example that already has every surface the check covers (guarded and open REST routes,
an SSE stream, a guarded gateway, a raw WebSocket and static files).

## What Changes

- **New page `docs/security/route-posture.md`** (and `docs/es/security/route-posture.md`), status `documented`,
  listed in the Security section of the sidebar. It covers:
  - the rule: guarded, `#[public]` or exempt;
  - enabling it: `require_route_posture()`, warn mode for migrations, exemptions with a reason;
  - `#[public]` and `PUBLIC_METADATA`, and how existing global guards that read `"isPublic"` keep working;
  - what counts as a guard and `controls_access()` (`ThrottlerGuard` and `ZodGuard` do not count);
  - WebSockets and SSE;
  - what is not checked: static files, raw `upgrade` listeners, Swagger/Scalar UIs, middleware, gateway handshakes;
  - the error message and how to read it;
  - `app.route_posture()` in tests, and `corvis posture` for review and CI;
  - a migration path for existing applications.
- **Cross-links**, in both languages, from: the guards page (including its "What global guards do not cover" warning
  and "Public routes with a global guard"), WebSocket guards, custom decorators (public routes), authentication
  ("Enabling authentication on all routes"), the CLI usage page section written by #28, and a new common-errors entry
  for the posture error.
- **`tertulia` enforces route posture.** `create_app()` calls `require_route_posture()`. `AuthController::login` and
  the open `RoomsController` reads (`list`, `messages`, `members`) get `#[public]` on the route, and `StatsController`
  gets `#[public]` at controller level (its JSON snapshot and SSE stream), which shows both placements. Request
  handling does not change. A test checks that the application starts and that the report lists those routes as
  public, the guarded routes and gateway handlers as protected, and the gateway handshake and the raw firehose
  listener as not checked. The example's README mentions it.

## Capabilities

### New Capabilities

- `docs/route-posture`: what the route posture page must document and where it is linked from.
- `examples/tertulia`: the example enforces route posture and its report matches its routes.

### Modified Capabilities

None.

## Impact

- Docs: `docs/security/route-posture.md`, `docs/es/security/route-posture.md`, `docs/.vitepress/sidebar.mts`, and
  edits (EN + ES) to `guards.md`, `websockets/guards.md`, `custom-decorators.md`, `security/authentication.md`,
  `cli/usages.md`, `faq/common-errors.md`.
- Example: `examples/tertulia/src/lib.rs`, `src/auth/auth_controller.rs`, `src/rooms/rooms_controller.rs`,
  `src/stats/mod.rs`, `tests/chat.rs`, `README.md`.
- No framework code changes, so the corvis-parity check does not apply (recorded in the definition of done).
- Depends on #26, #27 and #28 being merged: the page documents all three, and the tertulia test reads gateway entries
  (#27) and links to `corvis posture` (#28).

## Out of scope

- Switching `examples/cats-app` (and the docs samples built on it) from `#[set_metadata(IS_PUBLIC_KEY, true)]` to
  `#[public]`. Both work, since they write the same key. Possible follow-up for the examples (#21 area).
- Enabling posture in the three port apps (spike "Out of scope").
- An OpenAPI `security` section derived from `#[public]` (spike "Out of scope"; possible issue in the OpenAPI area).
- Naming middleware and raw `upgrade` listeners so they can be checked (spike "Out of scope").
- Authenticating the tertulia `/chat` handshake with `verify_client`. The example authenticates in
  `handle_connection` on purpose, and the page explains the difference.
