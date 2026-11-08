# Design

## Context

The route posture spike (#25, `openspec/changes/archive/2026-11-08-e1-route-posture-design/design.md`, decisions 8–11
and its Review notes) settled the design for SSE and gateways. #26 (`e1-public-metadata-and-startup-check`) adds
`#[public]`, `PUBLIC_METADATA`, `controls_access()` / `guard_name()`, the HTTP check between `build_router` and
`build_middleware`, and the report model in `crates/corvis/src/posture.rs`. #116
(`e13-exception-filters-and-global-guards-on-gateways`) makes global guards and filters apply to gateway handlers,
before the gateway's own guards, with an application switch to keep global enhancers HTTP-only. **This change assumes
both have landed.** The current-code references below were read at commit `ccbef4b`, before either of them; task 1.1
re-reads the touched functions after they land.

### SSE today

- `#[sse("path")]` maps to `RequestMethod::Get` with `is_sse = true` (`crates/corvis-macros/src/controller.rs:156`,
  `:188`) and emits `.sse()` (`:247-249`), which sets `RouteMeta::sse` (`crates/corvis/src/router/controller.rs:153`,
  `:240-243`).
- `build_router` does not read `meta.sse`; `CompiledRoute` has no SSE flag (`crates/corvis/src/router/mod.rs:38-57`).
  SSE routes get merged guards like any route (`:310-312`), so #26 already checks them, labelled as `GET`.

### Gateways today

- `#[websocket_gateway(path = ...)]` normalizes the path (leading `/`, no trailing `/`) and drains `use_guards`,
  `use_interceptors`, `use_pipes` and `set_metadata` below it into `WsEnhancers`; `use_filters` is an error; anything
  else is kept (`crates/corvis-macros/src/gateway.rs:35-73`, in `expand` at `:10`). #116 allows `use_filters` here.
- `#[handlers]` drains `use_guards`, `use_interceptors`, `use_pipes` and `set_metadata` on every method
  (`crates/corvis-macros/src/handlers.rs:89-116`); enhancers on a method whose kind is not `subscribe_message` are an
  error (`:119-123`); a method with no handler attribute is skipped and its enhancers dropped (`:117-118`).
- `WsEnhancers` (`crates/corvis/src/discovery.rs:25-58`) and `GatewayInfo { path, on_init, ..., enhancers }`
  (`discovery.rs:114-123`).
- `build_gateways` (`crates/corvis/src/websockets.rs:544-598`), per gateway provider: resolves gateway guards,
  interceptors and pipes (`:557-559`); per `WsMessage` handler merges gateway + handler guards (`:565-566`) into
  `CompiledWsHandler { call, guards, interceptors, pipes, params, target }` (`:516-524`), with `target.name` = method
  name and `target.metadata` = handler metadata (`:576-577`); stores them in a `HashMap` keyed by event (`:580`);
  then registers the gateway server in the global `SERVERS` map (`:585`) and **calls the user's `after_init`**
  (`:586-588`); builds `class_target` with the gateway name and metadata (`:589`) and the `GatewayRuntime { name,
  server, handlers, info, class_target, module_ref }` (`:527-534`, `:591-594`). It ignores the `RouterConfig` it
  receives (`_cfg`, `:546`); #116 changes that.
- `run_handler` runs `handler.guards` in order for each message (`websockets.rs:697-715`).
- The handshake runs no guard. `GatewayOptions::verify_client` (`websockets.rs:364-383`, setter `:423-426`) is
  checked in `upgrade` (`:665-670`). Options live in the gateway's `WsServer` (`server.options()`, `:462-464`) and are
  usually set in `after_init` (doc example at `:353-362`), so they are known only after `after_init` runs.
- Gateways are not under the global prefix: `match_upgrade` looks the request path up directly in
  `AppShared::gateways` (`websockets.rs:601-608`, `crates/corvis/src/router/mod.rs:452`).

### Other surfaces

- `ServeStaticModule` mounts are `ServeStaticOptions` instances (`serve_root`, `root_path`, ...,
  `crates/corvis/src/modules/serve_static.rs:17-29`) collected into `AppShared::static_dirs`
  (`crates/corvis/src/application.rs:411`).
- Raw `upgrade` listeners: `HttpServer::on_upgrade` and `upgrade_listener_count()`
  (`crates/corvis/src/http_adapter.rs:110-117`); `app.get_http_server()` (`application.rs:330`). Typically registered
  in `on_application_bootstrap` (`examples/tertulia/src/firehose.rs:43-77`), after `init()`'s compilation steps.

### Examples

- `examples/tertulia/src/chat/chat_gateway.rs:37-38`: `/chat` with `#[use_guards(AuthGuard)]` at gateway level and
  handler guards on `leave`, `message`, `typing`, `clear`; `after_init` only logs (`:203-205`).
- `examples/cats-app/src/chat.rs:7`: a public `/chat` gateway with no guards; with #116 its global `AuthGuard`
  reaches it, and #116 updates that example.

## Goals / Non-Goals

**Goals:**
- Unguarded gateway message handlers stop the application in enforce mode, in the same error as HTTP routes.
- The guards counted for a handler are exactly the ones that run, including global guards after #116.
- SSE lines are recognizable in the error and report.
- No user code (gateway `after_init`) runs before a failing check.
- Reviewers see the surfaces the check cannot enforce.

**Non-Goals:**
- Enforcing the handshake; enforcing static files, raw `upgrade` listeners, Swagger/Scalar or middleware.
- Changing which guards run on gateways (that is #116).
- CLI, docs page, example (#28, #29).

## Decisions

1. **Split `build_gateways` into compile and start steps.** `compile_gateways(container, cfg)` does everything up to
   and including the handler map (`websockets.rs:550-583`, plus #116's global guards and filters) and returns
   per-gateway compiled data without touching `SERVERS` or calling `after_init`. `start_gateways(compiled)` registers
   the servers and calls `after_init` (`:584-594`). `init()` becomes: fold globals → `build_router` →
   `compile_gateways` → posture check (routes and handlers) → `build_middleware` → `start_gateways` → `AppShared` →
   discovery and hooks.
   - Deviation from the spike (decision 1, "check right after `build_gateways`"): with the split, the check sees every
     handler's final guard list and still runs before any user hook. The spec's "Enforce mode refuses to start"
     requirement is extended to name `after_init`.
   - Alternative: keep `build_gateways` whole and check after it. Rejected: a failing check would already have run
     `after_init` and registered servers globally (`SERVERS`), which `WsServer::for_path` callers can observe.
   - Alternative: check routes before `build_middleware` (as #26) and gateways separately after `build_gateways`.
     Rejected: two failure points, two error lists, and user hooks before the second.

2. **Read the guard list that runs.** Posture reads `CompiledWsHandler::guards`, the list `run_handler` iterates. After
   #116 that list should contain global guards (unless the HTTP-only switch is on), then gateway, then handler guards.
   If #116 instead applies global guards at dispatch time from shared state, this change makes the compile step
   produce the full list so that compile and dispatch use the same `Vec` (task 1.2). Either way the rule is "count what
   runs", and the spike's requirement "Global guards do not count for gateways" is replaced by "Gateway guards count
   exactly as they run".
   - Alternative: compute "global + gateway + handler" in the posture module from `RouterConfig`. Rejected: it would
     duplicate the HTTP-only switch logic and could drift from dispatch.

3. **Keep what posture needs on the compiled handler.** `CompiledWsHandler` already has `guards` and `target` (method
   name and metadata). `GatewayRuntime` already has `name`, `info.path` and `class_target` (gateway metadata). No new
   fields are needed except the event, which is the map key. Entries are built from `AppShared::gateways` for
   `route_posture()` and from the compiled (not yet started) gateways for the startup check, through one function that
   takes `(path, gateway name, class target, [(event, handler target, guards)])`.

4. **Kinds and labels.** `PostureKind` gains `Sse` and `WsMessage { event: String }` (it is `#[non_exhaustive]` since
   #26). `CompiledRoute` gains `sse: bool` from `meta.sse`. Table labels: `SSE` for `Sse`, `WS` for `WsMessage` with
   `event "<name>"` after the path. Sorting stays path, then label, then owner/handler, so gateway lines sort among
   routes by path. JSON: `{"kind":"sse"}`, `{"kind":"ws_message","event":"ping"}`.

5. **Public resolution for handlers.** `Reflector.get_all_and_override::<bool>(PUBLIC_METADATA, &[&handler.target,
   &gateway.class_target]) == Some(true)`, the same rule as routes (#26 decision 4).

6. **`#[public]` in the gateway macros.** `gateway.rs`: `"public"` in the attribute loop emits
   `.set_metadata(::corvis::PUBLIC_METADATA, true)` on `WsEnhancers`; arguments are an error. `handlers.rs`: `"public"`
   is treated like `set_metadata` (it counts as an enhancer, so the existing "only supported on #[subscribe_message]
   methods" error covers `#[on_event]`, `#[cron]`, `#[interval]`, `#[timeout]`), and a `#[public]` on a method with no
   handler attribute is an error instead of being dropped (`handlers.rs:117-118`). The `misplaced!` message for
   `public` becomes "#[public] must be placed below #[controller] or #[websocket_gateway], or on a route or
   #[subscribe_message] method".

7. **Message exemptions.** `RoutePostureOptions::exempt_message(path, event, reason)`. Path matched with
   `path_matches` against the gateway path; event compared exactly (events are case-sensitive in the protocol), `*`
   matches every event of the gateway. Same blank-reason validation at the start of `init()` and the same stale
   warning ("Route posture exemption WS /chat event \"ping\" matches no handler"). HTTP exemptions never match gateway
   handlers and message exemptions never match routes. HTTP exemptions with `Get` or `All` match `Sse` entries.

8. **Not-checked surfaces.** `UncheckedSurface` (`#[non_exhaustive]` enum): `StaticFiles { serve_root }` per
   `AppShared::static_dirs`, `WsHandshake { path, gateway, verify_client: bool }` per gateway (read from
   `server.options().verify_client.is_some()`), `UpgradeListeners { count }` from
   `get_http_server().upgrade_listener_count()`. `RoutePostureReport::unchecked()` returns them; the table prints them
   under a "Not checked" heading. The startup check runs before `after_init`, so it never needs them; they are filled
   only by `route_posture()`, which reads live state when called (after `init()`, hooks have run, so
   `verify_client` and bootstrap-time listeners are visible). A listener count of zero is omitted.
   - Alternative: a fixed "not counted" marker for listeners as in the spike (decision 10). Not needed: `route_posture()`
     is only available after `init()`; the "not counted" case is the CLI report mode of #28, which can mark it there.

9. **Error text.** One list for routes and handlers, one count: "Route posture check failed: 3 routes or message
   handlers have no guard and are not marked #[public]." (singular forms for one entry; "routes" alone when there are
   no gateway entries, so #26's text and tests stay valid for HTTP-only apps). The fix paragraph adds "or the
   gateway" after "or its controller".

## Risks / Trade-offs

- [A global guard that returns `Ok(true)` for `ContextType::Ws` (one way #116 suggests to keep cats-app's gateway
  open) counts as protecting gateway handlers] → the report lists the guard per handler; the WebSocket guards page
  recommends `#[public]` on the gateway instead of skipping `ContextType::Ws`, so the intent is visible. A
  per-transport declaration is listed under Out of scope.
- [Reordering `init()` (gateways compiled before `build_middleware`, started after it) changes when gateway enhancers
  are resolved through DI] → resolution has no user side effects beyond provider construction, which bootstrap
  already did for singletons; the parity check and the existing `crates/corvis/tests/websockets.rs` and
  `ws_security.rs` suites cover it.
- [#116 may land with a different shape than assumed (switch name, where global guards are applied)] → task 1.1 reads
  it first; decision 2 says what to adapt; open question 1.
- [Handshakes stay unenforced] → reported per gateway with `verify_client`; a stricter mode is a separate issue.

## Open questions

1. The name of #116's switch that keeps global enhancers HTTP-only is not fixed yet; tests and docs here use whatever
   #116 ships. Confirm that "HTTP-only global enhancers ⇒ global guards do not count for gateways" is the intended
   posture semantics.
2. Should message exemptions accept event patterns beyond exact and `*` (for example `admin.*`)? Proposed: no until
   someone needs it.
3. Error count wording when gateway entries exist ("routes or message handlers"): acceptable, or always say "entries"?

## Review notes (2026-11-08)

- Depends on #116: use the HTTP-only switch name #116 settles; with the switch on, global guards do not count for
  gateways.
- Event patterns: exact and `*` only.
- A global guard that lets `ContextType::Ws` through counts as protection; the docs (#29) recommend `#[public]` on
  public gateways instead.
- The out-of-scope defects found (non-idempotent `init()` after a failure, enhancers on methods without a route
  attribute dropped silently, duplicate gateway events replaced silently) are filed as separate issues.

## Implementation record (task 1.1)

Read after #116 and #26 landed (base `80fb87c`):

- The #116 switch is `CorvisApplication::apply_global_enhancers_to_gateways(bool)` (default `true`), stored in
  `RouterConfig::gateway_global_enhancers` (`crates/corvis/src/router/mod.rs`). Open question 1 is answered: with it
  off, global guards are not in the handler guard list and do not count.
- Global guards enter the list in the compile step (`compile_gateways`, formerly `build_gateways`): they are the first
  elements of the gateway guard list, then the gateway's, then the handler's, in the same `Vec` that `run_handler`
  iterates. No move was needed (task 1.2).
- The posture check receives plain data (`GatewayPosture`, built by `websockets::posture_gateways`), so `posture.rs`
  compiles without the `websockets` feature.
- JSON tags follow the `type` key #26 already uses for kinds: `{"type":"sse"}`, `{"type":"ws_message","event":"ping"}`;
  not-checked surfaces serialize as `{"type":"static_files"|"ws_handshake"|"upgrade_listeners", ...}` under
  `unchecked`. A message exemption serializes with `"method":"WS"` and an extra `"event"` field.
- `PostureKind` is no longer `Copy` (`WsMessage` owns its event).

### Combination with the #26 security-review fixes

- Message exemptions match the gateway path with the same literal matcher as route exemptions (`template_matches`);
  event patterns stay exact or `*`.
- Redundant message exemptions (matching only guarded or public handlers) warn like route ones.
- `GuardScope` gains `Gateway { gateway }` and `Message { gateway, handler }`; `PublicSource` gains `Gateway`. The
  compiled handler records how many leading guards are global (0 with HTTP-only global enhancers) and how many come
  from the gateway, so scopes follow what runs.
- The access-guards summary counts gateway message handlers in the same line and in `access_guards()`.
