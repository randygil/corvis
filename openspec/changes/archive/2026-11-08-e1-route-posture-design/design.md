# Design

## Context

Epic #24 wants the application to refuse to start when a route has no guard and is not explicitly public. This spike
records how guards work today and settles the design the follow-up issues (#26–#29) implement. Everything below about
current behavior was read in the code at commit `7d2a83f`.

### How guards are declared and stored

- The guard trait is `CanActivate` with a single method, `can_activate(&self, &ExecutionContext) ->
  Result<bool, HttpException>` (`crates/corvis/src/common/guard.rs:23-26`). It carries no name and no kind.
- Declared guards are `GuardRef = EnhancerRef<dyn CanActivate>` (`crates/corvis/src/common/enhancer.rs:12-15`, `:68`):
  either `Instance(Arc<dyn CanActivate>)` (from an expression, `guard_instance`) or `Lazy { token, factory }` (from a
  type, `guard_of::<E>()`, resolved through DI in the controller's module, `:43-59`). The macros choose between them in
  `enhancer_tokens` (`crates/corvis-macros/src/utils.rs:95-109`): a plain type path becomes `guard_of`, anything else
  `guard_instance`.
- **Controller level:** `#[use_guards(...)]` below `#[controller]` is consumed by `expand_controller`
  (`crates/corvis-macros/src/controller.rs:84-121`) and becomes `.guard(...)` on `ControllerMetadata`
  (`crates/corvis/src/router/controller.rs:52-62`, `:80-83`). `#[set_metadata(k, v)]` and custom decorators
  (`#[apply_decorators]`, `#[api_*]`) end up in `ControllerMetadata::metadata` the same way (`controller.rs:107-118`).
- **Route level:** `expand_routes` (`crates/corvis-macros/src/controller.rs:184-245`) does the same for each method
  into `RouteMeta::guards` and `RouteMeta::metadata` (`crates/corvis/src/router/controller.rs:138-156`). Unknown
  attributes are kept on the method (`controller.rs:239-242`), so a new `#[public]` needs explicit handling.
- **Global, instance:** `app.use_global_guards(guard)` pushes an `Arc<dyn CanActivate>` into
  `RouterConfig::global_guards` (`crates/corvis/src/application.rs:164-168`, `crates/corvis/src/router/mod.rs:236-246`).
- **Global, provider:** `Provider::app_guard::<T>()` registers a provider with a `GlobalEnhancer::Guard` hook
  (`crates/corvis/src/di/provider.rs:98-106`, `:258-263`). These are folded into `global_guards` only inside
  `init()`, inserted ahead of the `use_global_guards` ones (`crates/corvis/src/application.rs:365-378`).

### When and how guards are merged

- `init()` (`crates/corvis/src/application.rs:361-423`) folds the provider globals, then calls `build_router`
  (`:399`), `build_middleware` (`:400`) and, with the `websockets` feature, `build_gateways` (`:401-402`), stores
  `AppShared` (`:403-417`), and only then discovers handlers and runs the `OnModuleInit` / `OnApplicationBootstrap`
  hooks (`:418-420`).
- `build_router` (`crates/corvis/src/router/mod.rs:277-394`) resolves controller guards once per controller
  (`:288`) and, for every route, builds the final list `global_guards + controller guards + route guards`
  (`:310-312`), then pushes one `CompiledRoute` per path alias and URI version (`:346-372`). `CompiledRoute`
  (`:38-57`) keeps `guards: Vec<Arc<dyn CanActivate>>` and two `Target`s (`class_target`, `handler_target`) whose
  `name` is the controller and handler name and whose `metadata` holds the `#[set_metadata]` entries
  (`:292-295`, `:326-329`; `Target` in `crates/corvis/src/common/context.rs:15-18`).
- At request time `run_pipeline` runs exactly `route.guards` in order (`crates/corvis/src/router/mod.rs:613-618`).
  So the compiled table is the single place where "which guards run for this route" is true by construction.
- `listen()` and `listen_in_background()` call `bind()`, which calls `init()` before binding the socket
  (`crates/corvis/src/application.rs:438-491`, `:476-477`). The in-memory test client goes through `handle()`, which
  also calls `init()` (`:426-430`; `TestingModule::init` at `crates/corvis/src/testing.rs:236-238`).
- Read-only introspection already exists without `init()`: `modules_info()` and `resolve_route_paths()`
  (`crates/corvis/src/application/introspection.rs:48-133`). It sees declared `GuardRef`s but not the provider
  globals (folded in `init()`) and could drift from the router.

### Public routes today

There is no framework notion of a public route. The convention apps use (`examples/cats-app`, the facten port, the
docs in `docs/guards.md:150-180`) is a constant `IS_PUBLIC_KEY = "isPublic"` set with `#[set_metadata(IS_PUBLIC_KEY,
true)]` (`examples/cats-app/src/common/decorators.rs:6-8`) and a global guard that reads it with
`Reflector::get_all_and_override` (`examples/cats-app/src/auth/auth_guard.rs:14-20`;
`crates/corvis/src/common/reflector.rs:19-23`, handler first, then class) and returns `Ok(true)`.

### SSE

`#[sse("path")]` is a controller route: the macro maps it to `RequestMethod::Get` and sets `RouteMeta::sse`
(`crates/corvis-macros/src/controller.rs:146-157`, `:246-249`; `crates/corvis/src/router/controller.rs:240-243`).
`build_router` does not treat it differently, so its guards are merged and run like any route.

### WebSockets

- `#[websocket_gateway(path = ...)]` collects `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]` and
  `#[set_metadata]` below it into `WsEnhancers` (`crates/corvis-macros/src/gateway.rs:42-73`;
  `crates/corvis/src/discovery.rs:25-58`). `#[handlers]` accepts the same enhancers only on `#[subscribe_message]`
  methods (`crates/corvis-macros/src/handlers.rs:86-123`).
- `build_gateways` (`crates/corvis/src/websockets.rs:544-598`) merges gateway guards and handler guards per handler
  (`:557`, `:565-566`) into `CompiledWsHandler::guards` (`:516-524`). It takes the `RouterConfig` but ignores it
  (`_cfg`, `:546`): **global guards do not run on gateways**, as the docs already warn (`docs/guards.md:111-113`,
  `docs/websockets/guards.md:30`). `run_handler` runs the handler's guards per message (`:711-715`).
- The handshake itself runs no guard. It can be filtered with `GatewayOptions::verify_client` (`:364-383`, checked in
  `upgrade` at `:665-670`); tertulia instead authenticates in `handle_connection` and disconnects with 4401
  (`examples/tertulia/src/chat/chat_gateway.rs:212-221`). Gateway options are runtime state set from
  `after_init`, which `build_gateways` calls (`:586-588`), so they are known once `build_gateways` returns.
- Raw WebSockets (`corvis::ws`) are `server.on_upgrade(...)` listeners (`crates/corvis/src/ws.rs:1-23`,
  dispatched in `crates/corvis/src/server.rs:322-331`). They are closures with no path or metadata, usually registered
  in `on_application_bootstrap` (`examples/tertulia/src/firehose.rs:43-77`), that is after the router is built.

### Other surfaces

- `corvis::serve_static(dir)` is a middleware instance (`crates/corvis/src/http/serve_static.rs:67`) stored as an
  opaque `Arc<dyn CorvisMiddleware>`; `ServeStaticModule` mounts are visible as `ServeStaticOptions` instances
  (`crates/corvis/src/application.rs:411`) and are served before routing (`crates/corvis/src/router/mod.rs:528-533`).
- Swagger and Scalar UIs are mounted with `app.use_fn(...)` (`crates/corvis-swagger/src/swagger_module.rs:152`,
  `crates/corvis-swagger/src/scalar.rs:244`): middleware, not routes.
- Middleware runs before guards and cannot be inspected (`crates/corvis/src/router/mod.rs:478-483`).
- The framework ships no controllers of its own (no `#[controller]` under `crates/*/src` outside CLI templates).
- Two framework guards do not control access: `ThrottlerGuard` (rate limiting,
  `crates/corvis/src/modules/throttler.rs:157-184`) and `ZodGuard` (payload validation,
  `crates/corvis/src/z/pipe.rs:250-264`). The docs' own example registers `ThrottlerGuard` as a global provider guard
  (`docs/guards.md:83-95`).
- The CLI is a separate binary that only wraps `cargo` for `start`/`build` (`crates/corvis-cli/src/main.rs:28-55`,
  `:332-349`); it cannot read an application's route table by itself.
- Errors are `anyhow::Error` (`crates/corvis/src/lib.rs:74`); metadata keys are exported constants such as
  `HTTP_CODE_METADATA` (`crates/corvis/src/lib.rs:122-124`).

### Port apps (`corvis-tests/corvis/*`, read-only survey)

- facten-api: 21 controllers, 74 routes, no `#[use_guards]`; global `Provider::app_guard::<JwtAuthGuard>()` and
  `RolesGuard` (`facten-api/src/app.module.rs:56-57`), public routes via `IS_PUBLIC_KEY = "isPublic"`.
- consi-backend: 17 controllers, 97 routes; controller-level guards on 13 controllers, route guards on 2 routes of
  `auth.controller.rs`; unguarded: 3 routes of `auth.controller.rs`, `checkout.controller.rs` (9), `vault.controller.rs`
  (1), `webhook.controller.rs` (3) = 16 routes. It also serves `/uploads` with `serve_static` (`src/main.rs:64`).
- houseplan-api: 4 controllers, 31 routes; guarded `bdv.controller.rs` and `api.controller.rs`; unguarded
  `telegram.controller.rs` (1) and `http/auth.controller.rs` (2) = 3 routes.

## Goals / Non-Goals

**Goals:**
- One precise rule for "protected" that matches what the router actually executes.
- An explicit, greppable, reviewable way to say "open on purpose" that also works with the existing
  `"isPublic"` convention.
- All unprotected routes reported at once, before anything listens or any hook has side effects.
- Zero behavior change for apps that do not opt in (the port apps keep 0 apidiff differences).
- A report usable in code review and as a CI gate.

**Non-Goals:**
- Proving that a guard authenticates; the check sees that a guard runs, not what it does.
- Inspecting middleware, raw `upgrade` listeners or documentation UIs.
- Changing which guards run where (for example making global guards apply to gateways).
- Turning the check on by default for existing applications (tied to #8).

## Decisions

1. **Check the compiled tables, inside `init()`, right after `build_gateways`.** The check walks
   `RouterTable::routes` and the compiled gateways, between `crates/corvis/src/application.rs:402` and the creation of
   `AppShared` at `:403`, so it runs before `OnModuleInit`/`OnApplicationBootstrap` hooks and before `bind()` opens a
   socket. It sees the provider globals (folded at `:365-398`) and the guard list the router really runs (`router/mod.rs:310-312`).
   - Alternative: check from introspection (`modules_info`) before `init()`. Rejected: it does not see
     `Provider::app_guard` globals and would duplicate the merge logic, so the check could drift from the router.
   - Alternative: a compile-time check in the macros. Rejected: global guards are registered at runtime, so a macro
     cannot know whether a route is covered.
   - Alternative: check in `listen()` only. Rejected: in-memory tests (`handle()`) and `listen_in_background` would
     skip it; `init()` is the one path all of them share.

2. **Opt-in application option, off by default.**
   `app.require_route_posture()` sets enforce mode; `app.set_route_posture(RoutePostureOptions)` sets the mode
   (`RoutePostureMode::{Off, Warn, Enforce}`) and exemptions. Both are builder methods on `CorvisApplication` like
   `enable_versioning`, stored in `AppConfig`, and must be called before `init()`/`listen()`.
   - Alternative: on by default. Rejected for now: consi (16) and houseplan (3) would stop starting, which breaks the
     parity requirement and every existing user; the default flip belongs to the compatible vs strict mode decision (#8).
   - Alternative: a field in `CorvisApplicationOptions`. Rejected: the other routing options (`enable_versioning`,
     `set_global_prefix`) are builder methods, and posture needs exemptions that read better as a builder.
   - New projects get it on: the `corvis new` template calls `require_route_posture()` (task group 4, #28).

3. **What counts: guards that control access, at any level the router runs them.** A route is protected when its
   compiled guard list contains at least one guard whose `controls_access()` is `true`. `CanActivate` gains two
   provided methods (non-breaking: existing impls compile unchanged):
   ```rust
   /// `true` (default) when this guard decides who may call the handler. Return `false` for guards that only
   /// limit or validate requests (rate limiting, payload validation): they do not count for route posture.
   fn controls_access(&self) -> bool { true }
   /// Name shown in posture errors and reports (default: the type name).
   fn guard_name(&self) -> &'static str { std::any::type_name::<Self>() }
   ```
   `ThrottlerGuard` and `ZodGuard` override `controls_access` to `false`. Names are shortened for display with
   `crate::di::token::short_type_name`.
   - Alternative: every guard counts. Rejected: the documented setup with a global `ThrottlerGuard` would make every
     route "protected" and the check meaningless.
   - Alternative: a separate marker trait (`AccessGuard`) required for counting. Rejected: every existing auth guard
     would have to opt in, and it cannot be queried through `dyn CanActivate` without extra registration.
   - Alternative: store names next to the `Arc`s (change `EnhancerRef::Instance` and `global_guards`). Rejected: it
     changes public enum variants and internal types for a display concern; the provided method covers instances,
     DI guards and test overrides alike (an `override_guard` double reports its own name, which is the truth).

4. **`#[public]` writes `PUBLIC_METADATA = "isPublic"`.** A new exported constant `corvis::PUBLIC_METADATA`
   (`"isPublic"`). `#[public]` below `#[controller]` emits `.set_metadata(::corvis::PUBLIC_METADATA, true)` on
   `ControllerMetadata`; on a method inside `#[routes]` it does the same on `RouteDef`. It takes no arguments (a
   compile error otherwise). Public is resolved like `get_all_and_override`: the handler's value if present, else the
   controller's. The router does not read it; only posture (and user guards) do.
   - Why that key: the global-guard-with-public-routes pattern from the original ecosystem uses `"isPublic"`
     everywhere (cats-app, facten, the docs). With the same key, `#[public]` works with existing guards unchanged, and
     existing `#[set_metadata(IS_PUBLIC_KEY, true)]` routes already count as public; composed decorators
     (`CustomDecorator::new().set_metadata(PUBLIC_METADATA, true)`) work too.
   - Alternative: a dedicated field `RouteMeta::public` / `ControllerMetadata::public`. Rejected: guards could not
     read it through `Reflector`, so apps would need both `#[public]` and `#[set_metadata(...)]`.
   - Alternative: a namespaced key (`"corvis:public"`). Rejected for the reason above; listed as an open question.
   - `#[public]` together with guards is allowed and normal (global auth guard that skips public routes); the
     route's status is "public" and the report lists the guards, so the real exposure is visible.
   - A misplaced `#[public]` (outside the four supported places) gets a clear compile error through the `misplaced!`
     list in `crates/corvis-macros/src/lib.rs:176-200`.

5. **Exemptions with a mandatory reason, for protection the framework cannot see.**
   `RoutePostureOptions::enforce().exempt(RequestMethod::Get, "/metrics", "scraped from the private network only")`
   and `.exempt_message("/chat", "ping", "health probe, no data")` for gateway events. `RequestMethod::All` matches
   any method. The path pattern is compared with the route's full path template (global prefix and URI version
   included, e.g. `/api/v1/cats/:id`) with the matcher middleware uses (`crates/corvis/src/common/middleware.rs:230`,
   case-insensitive, `*` wildcards). An empty or blank reason fails `init()`. This is the escape hatch for
   authentication done in middleware, at a reverse proxy, with mTLS or by network policy, without lying with `#[public]`.
   - Alternative: let middleware declare that it protects routes. Rejected for now: middleware is opaque
     (`Arc<dyn CorvisMiddleware>`) and its `for_routes` targets are already resolved paths; noted as possible future work.
   - Alternative: no exemptions. Rejected: users would mark protected routes `#[public]`, which makes the report wrong.

6. **Failure: one typed error listing everything.** `init()` returns `anyhow::Error` wrapping
   `corvis::RoutePostureError { unprotected: Vec<PostureEntry> }` (downcastable in tests), logged once with the
   `RoutePosture` logger context at error level. The message:
   ```text
   Route posture check failed: 3 routes have no guard and are not marked #[public].
     GET   /api/rooms                   RoomsController::list
     SSE   /api/stats/stream            StatsController::stream
     WS    /chat  event "rooms"         ChatGateway::rooms
   Add #[use_guards(...)] to the route or its controller (or register a global guard), mark it #[public] if it is
   meant to be open, or add an exemption with a reason. Guards that do not control access (ThrottlerGuard, ZodGuard)
   do not count.
   ```
   Routes are grouped per handler (path aliases and versions on one line each), sorted by path then method, so the
   output is stable for tests. Nothing request- or configuration-dependent is printed.
   - Alternative: fail on the first route. Rejected: fixing one at a time is slow and hides the size of the problem.
   - Alternative: `std::process::exit`. Rejected: `init()` already returns errors (DI errors), tests need to assert on
     the failure, and `main` returning `Err` already exits non-zero.

7. **Warn mode** logs the same text at warning level and continues. It exists for migrations: turn it on, read the
   list, fix, then switch to enforce.

8. **SSE is covered by the HTTP check; #27 only labels and tests it.** Because `#[sse]` routes are compiled routes
   with merged guards, #26's check already covers them. #27 adds the `SSE` label (from `RouteMeta::sse`, which must be
   carried into `CompiledRoute`) and regression tests.

9. **Gateways: check message handlers, report the handshake.** The unit is each `#[subscribe_message]` handler of each
   gateway: protected when `CompiledWsHandler::guards` has an access guard (gateway + handler guards; global guards
   do not count because they do not run there, `websockets.rs:546`), public when `#[public]` is on the handler or on
   the gateway (handler value first), or exempt by `(path, event)`. `#[public]` becomes valid below
   `#[websocket_gateway]` (`gateway.rs:45-72`) and on `#[subscribe_message]` methods (`handlers.rs:86-123`), writing
   the same metadata. The handshake is **not** enforced: connection-time authentication commonly lives in
   `handle_connection` (tertulia) where the framework cannot see it, and a gateway with every handler guarded leaks
   only what `handle_connection`/`after_init` push. The report shows, per gateway, whether `verify_client` is set.
   - Alternative: require `verify_client` or `#[public]` on every gateway. Rejected: tertulia's documented pattern
     (authenticate in `handle_connection`, close 4401) would fail; listed as an open question for a stricter mode.
   - Alternative: make global guards apply to gateways. Rejected: it changes behavior that the docs describe and the
     ports may rely on; out of scope.

10. **Surfaces outside the route table are reported, never enforced.** `ServeStaticModule` mounts ("static files are
    public by design"), the gateway handshakes, and the count of raw `upgrade` listeners appear in the report under
    "not checked". Middleware-mounted `serve_static` and Swagger/Scalar cannot be detected and are documented on the
    Security page instead. Raw `upgrade` listeners are usually registered in `on_application_bootstrap`, after the
    check runs: `route_posture()` called after a successful `init()` counts them (hooks have run by then), while the
    startup check and the CLI report mode (decision 12, no hooks) show them as "not counted".

11. **One report model shared by the check, the API and the CLI.** `corvis::posture::{RoutePostureReport,
    PostureEntry, PostureKind (Http | Sse | WsMessage), PostureStatus (Protected | Public | Exempt(reason) |
    Unprotected), GuardInfo { name, controls_access }, UncheckedSurface}`. The check computes the report and fails when
    `unprotected()` is non-empty. `app.route_posture() -> Option<RoutePostureReport>` returns it after `init()` (in
    every mode). It implements `Display` (the table) and `serde::Serialize` (JSON).

12. **CLI: `corvis posture [--json] [-- app args]` runs the app in report mode.** The CLI runs `cargo run` (same
    resolution as `corvis start`, `main.rs:332-349`) with the environment variable `CORVIS_ROUTE_POSTURE_REPORT=text`
    (or `json`). When `init()` sees that variable it computes the report regardless of the configured mode, prints it
    to stdout, and exits the process (code 0 if nothing is unprotected, 1 otherwise) right after the check, before any
    init hook and before binding. The CLI forwards the exit code.
    - Alternative: static analysis of the sources with `syn`. Rejected: it cannot see `use_global_guards` calls,
      provider globals in dynamic modules or guards chosen at runtime, so it would disagree with the startup check.
    - Alternative: a generated test calling `route_posture()`. Kept as the documented way for tests, but not for the
      CLI: it needs a test harness in every project.
    - Consequence: the app's bootstrap runs (DI factories, database connections), so it needs the same environment as
      `corvis start`. Documented. Raw `upgrade` listeners are not counted in this mode (no hooks run).
    - Exiting the process from inside `init()` only happens when that variable is set; it is documented as a
      development tool.

13. **This spike is archived without merging specs.** The spec delta here is the target for #26–#28; each follow-up
    change copies the requirements it implements into its own delta, so `openspec/specs/` only describes implemented
    behavior. The spike is archived with `--skip-specs` once the maintainer signs off.

## Migration impact

- **Existing apps and the port apps:** none while the option is off. The only always-on changes are two provided
  trait methods, a new constant, macro support for `#[public]`, and `controls_access() = false` on two guards, none of
  which affects request handling. #26–#28 still run the parity check because runtime code changes.
- **If a port opts in:** facten passes as is (global `JwtAuthGuard`); consi needs `#[public]` (or exemptions) on 16
  routes in 4 controllers; houseplan on 3 routes in 2 controllers. Opting them in is out of scope.
- **Apps with a global `ThrottlerGuard` and no auth guard:** every unguarded route becomes visible as unprotected,
  which is the purpose.
- **`corvis generate resource`** produces controllers without guards; in a project created by the new template the app
  will refuse to start until the routes are guarded or marked public. The error text says exactly that.

## Risks / Trade-offs

- [A guard that passes when its metadata is absent (a `RolesGuard` used alone) counts as protection] → the report
  lists guard names per route for review; docs advise overriding `controls_access` for pass-through guards.
- [`#[public]` shares the `"isPublic"` key with user code] → that is the point (existing guards honor it); an app using
  `"isPublic"` for something else would see routes counted as public. Documented; open question below.
- [Exemption patterns too broad (`*`) hide everything] → each exemption appears in the report with its reason and the
  routes it matched; an exemption that matches nothing logs a warning (stale configuration).
- [Report mode exits from `init()`] → only with an explicit environment variable; never in normal runs.
- [Gateways with unauthenticated handshakes pass] → reported per gateway; a stricter rule can come later.
- [Two new trait methods on `CanActivate` could clash with inherent methods of the same name on user guards] → no
  compile error (inherent methods win in method calls, posture calls through `dyn`), names chosen to be specific.

## Open questions

1. Metadata key for `#[public]`: `"isPublic"` (proposed, interoperates with existing guards) or a namespaced key?
2. Should the `corvis new` template enable `require_route_posture()` (proposed: yes, in #28), and should the default
   flip to on for all apps at 1.0 as part of #8?
3. Should a stricter option also require each gateway handshake to have `verify_client` or `#[public]`?
4. Environment variable name and contract for report mode (`CORVIS_ROUTE_POSTURE_REPORT=text|json`, exit code 1 when
   anything is unprotected): acceptable, or should report mode require an explicit call in `main` instead?
5. Should an exemption that matches no route fail in enforce mode instead of only warning?
6. Archive this spike with `--skip-specs` (proposed) or merge the delta now as the target spec?

## Review notes (2026-11-08)

- Keep `"isPublic"` as the metadata key: the port apps already use it, so existing global guards keep working.
- The `corvis new` template turns the check on (#28). Whether it is on by default for 1.0 is decided together with
  #8 (compatible vs strict mode), which is a maintainer decision.
- A stricter gateway-handshake mode is out of scope; revisit after #27.
- The report environment variable and its contract are accepted.
- An exemption that matches no route logs a warning instead of failing: a typo in an exemption already fails the
  check on the route it meant to exempt.
- Spikes are archived with `--skip-specs`; each follow-up change (#26-#29) carries its own requirements.
