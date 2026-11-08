# Tasks

Refines task group 3 of the route posture spike (`openspec/changes/archive/2026-11-08-e1-route-posture-design/tasks.md`).
Do not start before #26 (`e1-public-metadata-and-startup-check`) and #116
(`e13-exception-filters-and-global-guards-on-gateways`) are merged.

## 1. Preconditions

- [ ] 1.1 Re-read `build_gateways`, `run_handler` and `init()` as #116 left them; record in `design.md` (Context) the name of the HTTP-only switch and where global guards enter the handler guard list, and update the line references; verify the record answers open question 1 or flags it for the maintainer
- [ ] 1.2 If #116 applies global guards at dispatch time instead of in `CompiledWsHandler::guards`, move them into the compiled list so compile and dispatch share one `Vec` (design decision 2); verify `cargo test -p corvis --test websockets` and #116's gateway tests still pass

## 2. SSE

- [ ] 2.1 Add `sse: bool` to `CompiledRoute` (`crates/corvis/src/router/mod.rs`), set from `meta.sse` in `build_router`; add `PostureKind::Sse` and report SSE routes with it (label `SSE`, JSON `"sse"`), with `Get`/`All` exemptions matching them; verify in `crates/corvis/tests/route_posture.rs` that an unguarded `#[sse("stream")]` route fails enforce mode with an `SSE` line and passes with `#[public]`

## 3. `#[public]` on gateways

- [ ] 3.1 Accept `#[public]` below `#[websocket_gateway]` (`crates/corvis-macros/src/gateway.rs`), no arguments, emitting `.set_metadata(::corvis::PUBLIC_METADATA, true)`; verify with a test that the gateway's `GatewayInfo::enhancers.metadata` contains `("isPublic", true)`
- [ ] 3.2 Accept `#[public]` on `#[subscribe_message]` methods in `#[handlers]` (`crates/corvis-macros/src/handlers.rs`) as an enhancer, so it is rejected on `#[on_event]` / `#[cron]` / `#[interval]` / `#[timeout]`, and reject it on methods with no handler attribute; update the `misplaced!` message for `public` in `crates/corvis-macros/src/lib.rs`; verify with a test that the handler's `WsEnhancers::metadata` carries the entry and with `compile_fail` doctests for `#[public]` on `#[cron]` and with arguments

## 4. Gateways in the check

- [ ] 4.1 Split `build_gateways` (`crates/corvis/src/websockets.rs`) into `compile_gateways` (enhancers and handler map, no `SERVERS`, no `after_init`) and `start_gateways` (server registration and `after_init`); in `init()` order the steps as in design decision 1; verify `cargo test -p corvis --test websockets --test ws_security` pass unchanged
- [ ] 4.2 Add `PostureKind::WsMessage { event }` and build gateway entries from compiled gateways (guards from `CompiledWsHandler::guards`, public from handler then gateway metadata); run one check over routes and handlers between `compile_gateways` and `build_middleware`; adjust the error count wording (design decision 9); verify in `route_posture.rs`: gateway guard covers handlers, handler guard covers only its handler, `#[public]` on gateway and on handler, unguarded handler listed with path, event and `Gateway::method`, and an `after_init` that sets a flag did not run when the check fails
- [ ] 4.3 Cover global guards on gateways: global `Provider::app_guard::<AuthGuard>()` alone accepts a handler; with #116's HTTP-only switch the same handler is unprotected; a global `ThrottlerGuard` alone leaves it unprotected; verify `cargo test -p corvis --test route_posture`
- [ ] 4.4 Add `RoutePostureOptions::exempt_message(path, event, reason)` with exact event match and `*`, blank-reason validation and stale warnings; verify in `route_posture.rs`: exemption for `/chat` `ping` accepts the handler with its reason, blank reason fails `init()` naming path and event, an HTTP exemption does not match a handler, a stale message exemption warns

## 5. Report

- [ ] 5.1 Add `UncheckedSurface` (`StaticFiles`, `WsHandshake { verify_client }`, `UpgradeListeners { count }`) and `RoutePostureReport::unchecked()`, filled by `route_posture()` from `AppShared::static_dirs`, `AppShared::gateways` (`server.options()`) and `get_http_server().upgrade_listener_count()`; print them under "Not checked" and serialize them; verify a test in enforce mode with a `ServeStaticModule` mount, a gateway setting `verify_client` in `after_init` and an `on_upgrade` listener registered in `on_application_bootstrap` starts and lists all three
- [ ] 5.2 Verify `route_posture()` on an app with a guarded gateway and an `#[sse]` route lists the handler (kind, event, guards), the SSE route and the handshake, in off mode as well; check the JSON shape with `serde_json::to_value`

## 6. Docs

- [ ] 6.1 `docs/websockets/guards.md`: how route posture checks message handlers, `#[public]` on a gateway or handler, `exempt_message`, that global guards count only when they run on gateways, the recommendation to mark open gateways `#[public]` rather than skipping `ContextType::Ws` in a global guard, and that the handshake is reported but not enforced; same in `docs/es/websockets/guards.md`; verify the code samples against the implementation
- [ ] 6.2 `docs/websockets/gateways.md` (handshake section near `verify_client`): one paragraph on the not-checked handshake in the posture report; `docs/http/server-sent-events.md` ("Guards and interceptors"): SSE routes are checked like any route and labelled `SSE`; `docs/guards.md` posture section from #26: replace the sentence saying gateway handlers are not checked; same three pages in `docs/es/`; verify with `npm run docs:check` in `docs/`

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 7.3 `cargo test --workspace` passes, including the regression tests of groups 2–5, the existing WebSocket suites and the doctests of 3.2
- [ ] 7.4 Docs updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes
- [ ] 7.5 Framework runtime code changed (`init()` order, `build_gateways`, `CompiledRoute`): run the `corvis-parity` check and verify the three port apps (consi, facten, houseplan) keep 0 apidiff differences and equivalent databases
- [ ] 7.6 Confirm no public API, doc text or error message added by this change uses the words "Nest", "NestJS" or "Prisma"

## Workflow follow-up

- Archive the change with `/opsx:archive` after review (it updates `openspec/specs/security/route-posture/spec.md`,
  which #26 created; archive #26 first).
- Close issue #27, tick it in epic #24 and move its card to Done (see the `corvis-task` skill).
