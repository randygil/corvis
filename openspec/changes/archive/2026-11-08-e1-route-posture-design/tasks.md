# Tasks

This spike's own work is group 1. Groups 2–5 are the plan for the follow-up issues: when an issue starts, create its
change with `/opsx:propose` from its group (copying the requirements it implements from
`specs/security/route-posture/spec.md` into the new change's delta) and track the boxes there, not here.

## 1. Spike sign-off (#25)

- [x] 1.1 Maintainer reviews `design.md` and answers its open questions (metadata key, `corvis new` default, gateway handshake rule, report-mode variable, stale exemptions, archive mode); record the answers in `design.md` and adjust the decisions; verify no open question is left unanswered
- [x] 1.2 Update the bodies of issues #26–#29 with a link to this change and to their group below (maintainer session); verify each issue links back

## 2. `#[public]` metadata and startup check — issue #26 (change `e1-public-metadata-and-startup-check`)

- [ ] 2.1 Add the provided methods `controls_access()` (default `true`) and `guard_name()` (default type name) to `CanActivate` in `crates/corvis/src/common/guard.rs`, with doc comments; verify existing guards in `crates/corvis/tests/` and `examples/` compile unchanged with `cargo build --workspace --all-targets`
- [ ] 2.2 Override `controls_access()` to `false` in `ThrottlerGuard` (`crates/corvis/src/modules/throttler.rs`) and `ZodGuard` (`crates/corvis/src/z/pipe.rs`); verify with a unit test that both return `false` and a user guard returns `true`
- [ ] 2.3 Export `pub const PUBLIC_METADATA: &str = "isPublic";` from `crates/corvis/src/lib.rs` (next to `HTTP_CODE_METADATA`) and the prelude; verify `Reflector::get_all_and_override::<bool>(PUBLIC_METADATA, ...)` reads a value set with `#[set_metadata(PUBLIC_METADATA, true)]` in a test
- [ ] 2.4 Handle `#[public]` in `expand_controller` and `expand_routes` (`crates/corvis-macros/src/controller.rs`): no arguments allowed, emits `.set_metadata(::corvis::PUBLIC_METADATA, true)`; add `public` to the `misplaced!` list in `crates/corvis-macros/src/lib.rs`; verify with tests that `ControllerDef::metadata()` / `route_metas()` carry the entry, and that `#[public(x)]` fails to compile (doc test with `compile_fail` or a trybuild-style test if one exists)
- [ ] 2.5 Add `crates/corvis/src/posture.rs` with the report model (`RoutePostureReport`, `PostureEntry`, `PostureKind`, `PostureStatus`, `GuardInfo`, `UncheckedSurface`, `RoutePostureError`), `Display` (stable sorted table) and `Serialize`; verify with unit tests on hand-built entries (sorting, grouping of path aliases, text of the error message from `design.md` decision 6)
- [ ] 2.6 Add `RoutePostureMode`, `RoutePostureOptions` (`enforce()`, `warn()`, `exempt(method, pattern, reason)`) and the builder methods `require_route_posture()` / `set_route_posture()` on `CorvisApplication` (`crates/corvis/src/application.rs`, stored in `AppConfig`); verify a blank reason makes `init()` fail with the exemption named
- [ ] 2.7 Compute the HTTP part of the report from `RouterTable::routes` (guards via `controls_access`/`guard_name`, public via handler-then-class `PUBLIC_METADATA`, exemptions via `path_matches` on the full path template) and call the check in `init()` right after `build_gateways`, before `AppShared` and the init hooks; log stale exemptions as warnings; verify with tests in a new `crates/corvis/tests/route_posture.rs`: off mode starts with unguarded routes, enforce fails listing every unprotected route, `listen_in_background` returns the error without binding, no `OnModuleInit` hook ran, warn mode logs and starts
- [ ] 2.8 Cover each level in `crates/corvis/tests/route_posture.rs`: route guard, controller guard, `use_global_guards`, `Provider::app_guard`, only `ThrottlerGuard` (unprotected), `#[public]` on route, on controller, controller public overridden by `#[set_metadata(PUBLIC_METADATA, false)]`, exemption by method and wildcard, URI versioning and path aliases reported per handler; verify `cargo test -p corvis --test route_posture`
- [ ] 2.9 Expose `app.route_posture() -> Option<RoutePostureReport>` (available after `init()` in every mode); verify a test reads statuses protected/public/exempt/unprotected from a mixed app

## 3. Route posture for WebSockets and SSE — issue #27 (change `e1-route-posture-for-websockets-and-sse`)

- [ ] 3.1 Carry `RouteMeta::sse` into `CompiledRoute` (`crates/corvis/src/router/mod.rs`) and report SSE routes with kind `Sse`; verify a test with an unguarded `#[sse("stream")]` route fails in enforce mode with an `SSE` line, and passes with `#[public]`
- [ ] 3.2 Accept `#[public]` below `#[websocket_gateway]` (`crates/corvis-macros/src/gateway.rs`) and on `#[subscribe_message]` methods in `#[handlers]` (`crates/corvis-macros/src/handlers.rs`, still rejected on `#[on_event]`/`#[cron]`/...), writing `PUBLIC_METADATA`; verify with macro-level tests that the metadata reaches `WsEnhancers`
- [ ] 3.3 Keep, per compiled gateway handler, what posture needs (guard list is already there; gateway path, event, gateway and method names) and add `WsMessage` entries to the report, with exemptions by `exempt_message(path, event, reason)`; global guards must not count; verify in `crates/corvis/tests/route_posture.rs` (feature `websockets`): gateway guard covers handlers, handler guard, `#[public]` on gateway and on handler, global `app_guard` alone leaves a handler unprotected
- [ ] 3.4 Add the not-checked surfaces to the report: each gateway handshake with `verify_client` set or not, `ServeStaticModule` mounts, and the raw `upgrade` listener count when the report is read after the init hooks; verify a test with a `ServeStaticModule` mount and an `on_upgrade` listener in enforce mode starts and lists both as not checked

## 4. Posture report in the CLI — issue #28 (change `e1-posture-report-in-the-cli`)

- [ ] 4.1 In `init()`, when `CORVIS_ROUTE_POSTURE_REPORT` is `text` or `json`, compute the report whatever the mode, print it to stdout, flush, and exit with 0 (nothing unprotected) or 1, before the init hooks and before binding; verify with an integration test that spawns a small example binary with the variable set and checks the output, the exit code and that no port was opened
- [ ] 4.2 Add `corvis posture [--json] [-- args]` to `crates/corvis-cli/src/main.rs` (help text included): runs `cargo run` like `start` with the variable set and forwards the exit code; verify by running it against `examples/tertulia` (text and JSON) and a copy with one guard removed (exit code 1)
- [ ] 4.3 Make the `corvis new` templates (`crates/corvis-cli/src/templates.rs`) call `app.require_route_posture()` in `main` and mark the generated `get_hello` route `#[public]` (subject to open question 2); verify the generated project builds, starts and passes `corvis posture`

## 5. Route posture docs and example — issue #29 (change `e1-route-posture-docs-and-example`)

- [ ] 5.1 Write `docs/security/route-posture.md`: the rule, `require_route_posture()` / warn mode / exemptions, `#[public]` and `PUBLIC_METADATA` with existing guards, `controls_access()`, WebSockets and SSE, what is not checked (static files, raw `upgrade` listeners, Swagger/Scalar, middleware, handshakes), the error message, `route_posture()` and `corvis posture`; verify every code sample against the implementation
- [ ] 5.2 Translate it to `docs/es/security/route-posture.md` and add the page to `docs/.vitepress/sidebar.mts` (Security section, EN and ES titles); verify with `npm run docs:check` in `docs/`
- [ ] 5.3 Cross-link from `docs/guards.md` (and its warning at "What global guards do not cover"), `docs/websockets/guards.md`, `docs/custom-decorators.md` (public routes), `docs/security/authentication.md` and the CLI page that lists commands, in both languages; verify links with `npm run docs:check`
- [ ] 5.4 Enable posture in `examples/tertulia` (`create_app` in `src/lib.rs` calls `require_route_posture()`); mark `#[public]` on `AuthController::login`, the public `RoomsController` reads (`list`, `messages`, `members`) and `StatsController` (`get` and the SSE `stream`), leaving behavior unchanged; add a tertulia test that `create_app()` + `init()` succeeds and that `route_posture()` lists those routes as public and the guarded ones as protected; verify `cargo test -p tertulia`

## 6. Definition of done

- [ ] 6.1 `cargo build --workspace --all-targets` with no warnings (each follow-up change)
- [ ] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 6.3 `cargo test --workspace` passes, including the regression tests of groups 2–5
- [ ] 6.4 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes (group 5; groups 2–4 update any page they make inaccurate)
- [ ] 6.5 Framework runtime code changes in groups 2–4: run the `corvis-parity` check and verify the three port apps (consi, facten, houseplan) keep 0 apidiff differences and equivalent databases
- [ ] 6.6 This spike: no code changed, so no build, test or parity run applies; confirm the change only adds files under `openspec/changes/e1-route-posture-design/`

## Workflow follow-up

- Archive this change with `/opsx:archive` once group 1 is done (with `--skip-specs`, per design decision 13, unless
  the maintainer decides otherwise); each follow-up change is archived separately when its issue is done.
- Close issue #25, tick it in epic #24 and move its card to Done (see the `corvis-task` skill).
