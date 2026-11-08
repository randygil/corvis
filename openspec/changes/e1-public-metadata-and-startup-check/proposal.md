# Proposal

Implements [randygil/corvis#26](https://github.com/randygil/corvis/issues/26) (epic
[#24](https://github.com/randygil/corvis/issues/24)). The design was settled in the route posture spike
([#25](https://github.com/randygil/corvis/issues/25), `openspec/changes/archive/2026-11-08-e1-route-posture-design/`);
this change implements its task group 2 and copies the HTTP requirements of its spec delta.

## Why

Forgetting `#[use_guards(...)]` on a route is silent: the route works, its tests pass, and it is open to anyone.
Corvis has no way to say "this route is open on purpose", so a reviewer cannot tell a deliberate public route from a
forgotten guard, and nothing refuses to start when a route ends up unprotected. Apps already mark public routes by
hand with `#[set_metadata(IS_PUBLIC_KEY, true)]` (`examples/cats-app`, the facten port, `docs/guards.md`), but the
framework does not read that marker.

## What Changes

- **`#[public]`** below `#[controller]` and on route methods inside `#[routes]`. It stores the metadata entry
  `corvis::PUBLIC_METADATA` (`"isPublic"`, the key apps already use) with value `true`, so existing global guards
  that skip public routes through `Reflector` honor it unchanged. It takes no arguments; on any other item it is a
  compile error.
- **`CanActivate` gains two provided methods**, both non-breaking: `controls_access()` (default `true`) and
  `guard_name()` (default: the type name). `ThrottlerGuard` and `ZodGuard` return `false` from `controls_access()`:
  they limit or validate, they do not decide who may call a route.
- **Opt-in startup check:** `app.require_route_posture()` (enforce) and `app.set_route_posture(RoutePostureOptions)`
  (mode `Off`, `Warn` or `Enforce`, plus exemptions with a mandatory reason). Off by default: apps that do not call
  them behave exactly as before.
- **The rule:** in warn and enforce mode every compiled controller route (each method, path alias and URI version) is
  accepted only when a guard that controls access runs for it (global, provider-global, controller or route level:
  exactly the list the router executes), when it is public (handler value first, then controller), or when an
  exemption matches it.
- **Where and how it fails:** inside `init()`, right after the route table is compiled and before middleware
  `configure`, gateways, `OnModuleInit` and `OnApplicationBootstrap`, so `listen()` / `listen_in_background()` fail
  before binding. Enforce mode returns a typed `RoutePostureError` listing every unprotected route with the fix; warn
  mode logs the same list once and starts. An exemption that matches no route logs a warning.
- **Report API:** `app.route_posture()` returns, after `init()` and in every mode, the posture of every controller
  route (kind, method, full path, controller and handler, guards with `controls_access`, status). It implements
  `Display` and `serde::Serialize`.
- Docs (EN + ES): `#[public]`, `PUBLIC_METADATA` and a short "Require a guard on every route" section in the guards
  page; the full route posture page is #29.

## Capabilities

### New Capabilities

- `security/route-posture`: the `#[public]` marker, what counts as an access guard, the opt-in startup check for
  controller routes (modes, exemptions, error, warn mode) and the posture report API.

### Modified Capabilities

None (no spec covers guards yet).

## Impact

- `crates/corvis/src/common/guard.rs` (two provided methods), `crates/corvis/src/modules/throttler.rs`,
  `crates/corvis/src/z/pipe.rs` (override `controls_access`).
- `crates/corvis/src/lib.rs` (`PUBLIC_METADATA`, `public` re-export, prelude, `posture` module exports).
- New `crates/corvis/src/posture.rs` (options, report model, check, error).
- `crates/corvis/src/application.rs` (`AppConfig`, builder methods, `init()`, `route_posture()`).
- `crates/corvis-macros/src/controller.rs` (`#[public]` in `expand_controller` and `expand_routes`),
  `crates/corvis-macros/src/lib.rs` (`public` in the `misplaced!` list).
- New tests: `crates/corvis/tests/route_posture.rs`, plus unit tests in `posture.rs`.
- Docs: `docs/guards.md`, `docs/custom-decorators.md` and their Spanish translations.
- Port apps: no behavior change while the option is off (default). The parity check still runs because `init()` and
  the macros change.

## Out of scope

- SSE labelling, gateway message handlers, `#[public]` on gateways, gateway exemptions and the not-checked surfaces
  (static files, handshakes, raw `upgrade` listeners): #27.
- `corvis posture` in the CLI, report mode through `CORVIS_ROUTE_POSTURE_REPORT`, and the `corvis new` template: #28.
- The route posture docs page and enabling it in `examples/tertulia`: #29.
- Turning the check on by default: decided with the compatible vs strict mode spike (#8).
- Enabling posture in the three port apps.
- Attributes silently dropped on methods without a route attribute inside `#[routes]` (`#[use_guards]` on a helper
  method is discarded today, `crates/corvis-macros/src/controller.rs:245-246`): only `#[public]` gets an error here;
  possible new issue for the others.
- `init()` is not idempotent after a failure: calling it again folds `Provider::app_*` globals a second time
  (`crates/corvis/src/application.rs:365-398`). Pre-existing for any `init()` error; possible new issue.
- OpenAPI `security` derived from `#[public]` (possible new issue in the OpenAPI area).
