# Tasks

Refines task group 2 of the route posture spike (`openspec/changes/archive/2026-11-08-e1-route-posture-design/tasks.md`).

## 1. Guard declarations

- [x] 1.1 Add the provided methods `controls_access()` (default `true`) and `guard_name()` (default `std::any::type_name::<Self>()`) to `CanActivate` in `crates/corvis/src/common/guard.rs`, with doc comments (design decision 2); verify `cargo build --workspace --all-targets` compiles every existing guard in `crates/corvis/tests/`, `examples/` and the CLI templates unchanged
- [x] 1.2 Override `controls_access()` to return `false` in `ThrottlerGuard` (`crates/corvis/src/modules/throttler.rs`) and `ZodGuard` (`crates/corvis/src/z/pipe.rs`); verify with a unit test that both return `false` through `&dyn CanActivate`, that a user guard returns `true`, and that `guard_name()` through `Arc<dyn CanActivate>` is the concrete type's name

## 2. `PUBLIC_METADATA` and `#[public]`

- [x] 2.1 Export `pub const PUBLIC_METADATA: &str = "isPublic";` from `crates/corvis/src/lib.rs` next to `HTTP_CODE_METADATA`, and add it to the prelude; verify with a test that `Reflector::get_all_and_override::<bool>(PUBLIC_METADATA, ..)` reads a value set with `#[set_metadata(PUBLIC_METADATA, true)]`
- [x] 2.2 Handle `#[public]` in `expand_controller` and `expand_routes` (`crates/corvis-macros/src/controller.rs`): no arguments (error "`#[public]` takes no arguments"), emits `.set_metadata(::corvis::PUBLIC_METADATA, true)`; error when it sits on a method without a route attribute; verify with tests that `ControllerDef::metadata().metadata` and `route_metas()[i].metadata` contain `("isPublic", true)`
- [x] 2.3 Add `public` to the `misplaced!` list in `crates/corvis-macros/src/lib.rs` and re-export it as `corvis::public` and from the prelude; verify with `compile_fail` doctests on `PUBLIC_METADATA` in `crates/corvis/src/lib.rs` for `#[public(true)]` on a route, `#[public]` on a helper method inside `#[routes]` and `#[public]` on a free function, plus one passing doctest with `#[public]` below `#[controller]`

## 3. Report model and options

- [x] 3.1 Add `crates/corvis/src/posture.rs` with `RoutePostureReport`, `PostureEntry`, `PostureKind` (`#[non_exhaustive]`, `Http { method }`), `PostureStatus`, `GuardInfo` and `RoutePostureError`, `Display` (sorted table, error text of design decision 8, singular/plural) and `serde::Serialize`; verify with unit tests on hand-built entries: sort order, error text for one and two routes, JSON field names
- [x] 3.2 Add `RoutePostureMode`, `RoutePostureOptions` (`off()`, `warn()`, `enforce()`, `exempt(method, pattern, reason)`) and the exemption matcher (method rule of design decision 5, `path_matches` on the full template); verify with unit tests: `All` matches every method, `Get` does not match an `#[all]` route, `cats/*` matches `/api/v1/cats/:id`, case-insensitive literals
- [x] 3.3 Add `require_route_posture()` and `set_route_posture()` to `CorvisApplication` (`crates/corvis/src/application.rs`), stored in a new `AppConfig::route_posture` field; validate blank reasons at the start of `init()` when the mode is not `Off`; verify a test where a blank reason makes `init()` fail with the exemption's method and pattern in the message, and an `Off` configuration with a blank reason starts

## 4. Startup check

- [x] 4.1 Implement `posture::report_for_routes(&RouterTable, &RoutePostureOptions)` (guards via `controls_access`/`guard_name`, public via `Reflector` handler-then-class, exemptions, stale exemptions) and call the check in `init()` between `build_router` and `build_middleware`: enforce returns `RoutePostureError` (logged once at error level), warn logs at warning level, off does nothing; verify in a new `crates/corvis/tests/route_posture.rs`: off mode starts with unguarded routes and logs nothing about posture, enforce fails listing every unprotected route, warn logs and serves requests
- [x] 4.2 Assert the failure point: an `OnModuleInit` provider and a module `configure` that set flags stay untouched, and `listen_in_background(0)` returns the error with `get_url()` still `None`; verify `cargo test -p corvis --test route_posture`
- [x] 4.3 Cover each guard level and marker in `route_posture.rs`: route guard, controller guard, `use_global_guards`, `Provider::app_guard`, only a global `ThrottlerGuard` (unprotected), only a `ZodGuard` (unprotected), `#[public]` on route, on controller, controller public overridden by `#[set_metadata(PUBLIC_METADATA, false)]`, the `"isPublic"` literal convention, an `override_guard` double keeps the route protected; verify `cargo test -p corvis --test route_posture`
- [x] 4.4 Cover exemptions and paths: exemption by method, by wildcard and with `All`; stale exemption logs a warning and starts; global prefix, URI versioning (`/v1`, `/v2`) and controller path aliases each reported on their own line, with an exemption matching only one of them; verify `cargo test -p corvis --test route_posture`

## 5. Report API

- [x] 5.1 Add `CorvisApplication::route_posture() -> Option<RoutePostureReport>` (`None` before `init()`, computed on demand from `AppShared::router` with the configured exemptions); verify a test reads protected, public, exempt (with reason) and unprotected from a mixed app, the same report in off mode, and the JSON output with `serde_json::to_value`

## 6. Docs

- [x] 6.1 `docs/guards.md`: replace the hand-made `IS_PUBLIC_KEY` constant in "Public routes with a global guard" with `#[public]` / `PUBLIC_METADATA` (keeping a note that the `"isPublic"` literal still works), add a short "Require a guard on every route" section (`require_route_posture()`, warn mode, exemptions with a reason, `controls_access()`, `route_posture()`), stating that `#[sse]` routes are checked like any route and that gateway message handlers are not checked (#27 updates that sentence); same in `docs/es/guards.md`; verify every code sample against the implementation
- [x] 6.2 `docs/custom-decorators.md`: use `PUBLIC_METADATA` / `#[public]` in the metadata example and mention that `CustomDecorator::new().set_metadata(PUBLIC_METADATA, true)` composes it; same in `docs/es/custom-decorators.md`; verify with `npm run docs:check` in `docs/`

## 7. Definition of done

- [x] 7.1 `cargo build --workspace --all-targets` with no warnings
- [x] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 7.3 `cargo test --workspace` passes, including the regression tests of groups 1–5 and the doctests of 2.3
- [x] 7.4 Docs updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes
- [x] 7.5 Framework runtime code changed (`init()`, `CanActivate`, macros): run the `corvis-parity` check and verify the three port apps (consi, facten, houseplan) keep 0 apidiff differences and equivalent databases (not run by the implementer: the maintainer runs it) (maintainer batch run on batch/phase05-h 66ff09a: consi 832/832, facten 727/727, houseplan 636/636; routes identical, databases equivalent)
- [x] 7.6 Confirm no public API, doc text or error message added by this change uses the words "Nest", "NestJS" or "Prisma"

## Workflow follow-up

- Archive the change with `/opsx:archive` after review (it creates `openspec/specs/security/route-posture/spec.md`).
- Close issue #26, tick it in epic #24 and move its card to Done (see the `corvis-task` skill).
