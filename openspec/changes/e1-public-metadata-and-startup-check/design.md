# Design

## Context

The route posture spike (#25, `openspec/changes/archive/2026-11-08-e1-route-posture-design/design.md`, decisions 1–7
and 11, plus its Review notes) settled the design. This change implements its HTTP part (task group 2). Everything
below about current behavior was re-read in the code at commit `ccbef4b`.

### Guards

- `CanActivate` has one method, `can_activate(&self, &ExecutionContext) -> Result<bool, HttpException>`, under
  `#[async_trait]` (`crates/corvis/src/common/guard.rs:23-26`). No name, no kind.
- Declared guards are `GuardRef = EnhancerRef<dyn CanActivate>`: `Instance(Arc<..>)` or `Lazy { token, factory }`
  (`crates/corvis/src/common/enhancer.rs:12-15`, `:39-68`). `Lazy` resolves through DI and honors test overrides
  (`enhancer_override_token`, `:46-50`).
- `use_global_guards` pushes into `RouterConfig::global_guards` (`crates/corvis/src/application.rs:165-168`,
  `crates/corvis/src/router/mod.rs:238-246`). `Provider::app_guard` globals are folded in at the start of `init()`,
  ahead of the `use_global_guards` ones (`application.rs:365-398`).
- Two framework guards do not control access: `ThrottlerGuard` (`crates/corvis/src/modules/throttler.rs:157-185`,
  rate limiting per IP and handler) and `ZodGuard` (`crates/corvis/src/z/pipe.rs:250-264`, payload validation).

### Route compilation and `init()`

- `init()` (`application.rs:361-423`): returns early when `initialized`; folds the provider globals (`:365-398`); then
  `build_router` (`:399`), `build_middleware` (`:400`, which runs every module's `configure(consumer)`),
  `build_gateways` (`:401-402`, which calls each gateway's `after_init`, `crates/corvis/src/websockets.rs:586-588`),
  stores `AppShared` (`:403-417`), discovers handlers and runs `OnModuleInit` / `OnApplicationBootstrap`
  (`:418-420`), then sets `initialized` (`:421`).
- `build_router` (`router/mod.rs:277-394`) resolves controller guards once per controller (`:288`), builds
  `class_target` with the controller name and metadata (`:292-295`), and for every route merges
  `global_guards + controller guards + route guards` (`:310-312`) and builds `handler_target` (`:326-329`). It pushes
  one `CompiledRoute` per controller path alias, route path and URI version (`:300`, `:346-347`), with the full path
  (`join_paths(&[prefix, version, c_path, r_path])`, `:348`). `CompiledRoute` (`:38-57`) keeps `method`, `path`,
  `class_target`, `handler_target`, `guards` and `host`.
- At request time `run_pipeline` runs exactly `route.guards` in order (`router/mod.rs:613-618`). The compiled table
  is the single place where "which guards run for this route" is true by construction.
- `Target.metadata` is a `HashMap<String, Value>` (`crates/corvis/src/common/context.rs:15-18`) built from the
  `Vec<(String, Value)>` of `#[set_metadata]` entries (last one wins).
- `listen()` / `listen_in_background()` call `bind()`, which calls `init()` before `TcpListener::bind`
  (`application.rs:438-491`, `:476-477`). `handle()` (`:426-430`) and `TestingModule::init`
  (`crates/corvis/src/testing.rs:236-238`) also go through `init()`.
- Builder methods such as `enable_versioning` live on `CorvisApplication` and write `AppConfig`
  (`application.rs:71-78`, `:205-208`).

### Public routes today

- No framework notion. Apps use `IS_PUBLIC_KEY = "isPublic"` with `#[set_metadata(IS_PUBLIC_KEY, true)]`
  (`examples/cats-app/src/common/decorators.rs:8`, used in `cats_controller.rs`, `auth_controller.rs`,
  `app_controller.rs`) and a global guard that reads it with `Reflector::get_all_and_override`
  (`examples/cats-app/src/auth/auth_guard.rs:14-20`; `crates/corvis/src/common/reflector.rs:19-23`: the first target
  whose value deserializes into the requested type wins, handler first). Same pattern in `docs/guards.md:150-180` and
  `docs/custom-decorators.md:80-100`.
- `ControllerMetadata::set_metadata` (`crates/corvis/src/router/controller.rs:96`) and `RouteDef::set_metadata`
  (`:227`) store entries.

### Macros

- `expand_controller` (`crates/corvis-macros/src/controller.rs:55`) drains the struct attributes (`:85-121`):
  `use_guards`, `use_interceptors`, `use_pipes`, `use_filters`, `set_metadata`, `version`, decorators; anything else is
  kept (`:117`).
- `expand_routes` (`controller.rs:174`) does the same per method; unknown attributes are kept (`:241`). Attributes are
  drained before checking that the method has a route attribute, and a method without one is skipped
  (`:245-246`), so enhancers on a helper method are silently dropped.
- `attr_name` uses the last path segment (`crates/corvis-macros/src/utils.rs:7-9`); `attr_args` returns an empty list
  for `#[x]` and `#[x()]` (`:12-21`).
- The `misplaced!` list (`crates/corvis-macros/src/lib.rs:176-200`) defines attribute macros that only emit a
  `compile_error!`. They help only when the name resolves: the `corvis` crate re-exports `controller`, `routes`,
  `handlers`, ... (`crates/corvis/src/lib.rs:133-135`) but none of the `misplaced!` ones, so a misplaced
  `#[use_guards]` in an app is reported as "cannot find attribute".
- No `trybuild` or `compile_fail` test exists in the workspace.

### Middleware matching and exports

- `path_matches(pattern, path)` (`crates/corvis/src/common/middleware.rs:230-272`): trims slashes, `*` / `(.*)` /
  `{*name}` / `*name` wildcards, `:param` and `{param}` pattern segments match any segment, literal segments compare
  ignoring ASCII case.
- Metadata key constants are exported from the crate root (`HTTP_CODE_METADATA`, `crates/corvis/src/lib.rs:122-124`);
  the prelude lists exports explicitly (`lib.rs:151-176`).
- `short_type_name` exists in `crates/corvis/src/di/token.rs:102`.

## Goals / Non-Goals

**Goals:**
- `#[public]` that existing `"isPublic"` guards honor unchanged.
- An opt-in check that sees exactly the guards the router runs and reports every unprotected route at once, before any
  socket is bound or any hook runs.
- No observable change for apps that do not opt in.
- A report API that later changes (#27 gateways and SSE, #28 CLI) extend without breaking it.

**Non-Goals:**
- SSE labels, gateways, `#[public]` on gateways, not-checked surfaces (#27); CLI and report mode (#28); docs page and
  example (#29).
- Proving that a guard authenticates.
- Changing which guards run where.

## Decisions

1. **Check the compiled route table right after `build_router`.** A new `posture::check_routes(&RouterTable, &options)`
   runs between `build_router` (`application.rs:399`) and `build_middleware` (`:400`). That is later than every guard
   source (provider globals are folded at `:365-398`) and earlier than any user code with side effects: module
   `configure`, gateway `after_init` (inside `build_gateways`), `OnModuleInit`, `OnApplicationBootstrap`, and the
   socket bind.
   - Deviation from the spike (decision 1 said "right after `build_gateways`"): `build_gateways` calls user
     `after_init` hooks and registers gateway servers globally (`websockets.rs:585-588`), so a failing check placed
     after it would already have run user code. #27 adds gateway handlers by compiling them before their hooks run
     (see that change).
   - Alternative: check from introspection before `init()`. Rejected in the spike (no provider globals, logic drift).

2. **Trait extension with provided methods.** In `guard.rs`:
   ```rust
   /// `true` (default) when this guard decides who may call the handler. Return `false` for guards that only
   /// limit or validate requests (rate limiting, payload validation): they do not count for route posture.
   fn controls_access(&self) -> bool { true }
   /// Name shown in route posture errors and reports (default: the type name).
   fn guard_name(&self) -> &'static str { std::any::type_name::<Self>() }
   ```
   Both are synchronous, so `#[async_trait]` leaves them as is; existing impls compile unchanged. Calls go through
   `dyn CanActivate`, so a provider-global guard (cast from `Arc<T>`) and an `override_guard` double report their own
   type. Names are shortened for display with `short_type_name`. `ThrottlerGuard` and `ZodGuard` override
   `controls_access` to `false`.
   - Alternative: a marker trait or names stored next to the `Arc`s. Rejected in the spike (decision 3).

3. **`PUBLIC_METADATA` and `#[public]`.** `pub const PUBLIC_METADATA: &str = "isPublic";` next to
   `HTTP_CODE_METADATA`, also in the prelude. `#[public]` is handled in the attribute loops of `expand_controller` and
   `expand_routes`, emitting `.set_metadata(::corvis::PUBLIC_METADATA, true)`. Errors:
   - arguments (`#[public(..)]`, `#[public = ..]`): "`#[public]` takes no arguments";
   - on a method of a `#[routes]` block without a route attribute: "`#[public]` must be on a route method (with
     `#[get]`, `#[post]`, ...)"; checked before the `continue` at `controller.rs:246`, only for `#[public]`;
   - anywhere else: `public` joins the `misplaced!` list ("#[public] must be placed below #[controller] or on a route
     method inside `#[routes]`", wording updated by #27 for gateways) and is re-exported as `corvis::public` and from
     the prelude, so the misplaced macro is the one that resolves and its message is shown. Inside `#[controller]` and
     `#[routes]` the attribute is consumed by name before resolution, so no import is needed there.
   - Alternative: do not re-export (keep the "cannot find attribute" error). Rejected: the spec asks for a message
     that says where the attribute goes, and it costs one re-export.

4. **Public resolution reuses `Reflector`.** Status uses
   `Reflector.get_all_and_override::<bool>(PUBLIC_METADATA, &[&route.handler_target, &route.class_target])
   == Some(true)`, the same call user guards make, so posture and guards cannot disagree. A non-boolean value is
   ignored like in guards.

5. **Options and modes.** In `crates/corvis/src/posture.rs`:
   ```rust
   #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
   pub enum RoutePostureMode { #[default] Off, Warn, Enforce }
   #[derive(Clone, Debug, Default)]
   pub struct RoutePostureOptions { /* mode, exemptions */ }
   impl RoutePostureOptions {
       pub fn off() -> Self; pub fn warn() -> Self; pub fn enforce() -> Self;
       pub fn exempt(self, method: RequestMethod, pattern: impl Into<String>, reason: impl Into<String>) -> Self;
   }
   ```
   `CorvisApplication::require_route_posture()` = `set_route_posture(RoutePostureOptions::enforce())`, stored in a new
   `AppConfig::route_posture` field. Fields are private with accessors, so #27 can add `exempt_message` without a
   breaking change.
   - Exemption method matching: `RequestMethod::All` in the exemption matches any route; otherwise the methods must be
     equal (a route declared with `#[all]` is matched only by an `All` exemption). Path: `path_matches(pattern,
     route.path)` on the full template (`/api/v1/cats/:id`).
   - Validation of blank reasons runs at the very start of `init()` when the mode is not `Off`, before the provider
     globals are folded, so a configuration error leaves nothing half-done. In `Off` mode options are not read at all
     (spec: off changes nothing).

6. **One entry per compiled route.** The report has one `PostureEntry` per `CompiledRoute` (method, full path), not
   one per handler: an exemption can match one alias or version and not another, so status is per path. Sorting is by
   path, then method, then `Controller::handler`, so output is stable. The spike's "grouped per handler" (decision 6)
   is read as "each alias or version on its own line", which this satisfies.

7. **Report model, extensible.** Public types in `corvis::posture`, re-exported from the crate root:
   `RoutePostureReport` (private fields; `entries()`, `unprotected()`, `stale_exemptions()`), `PostureEntry`
   (`#[non_exhaustive]`, public fields `kind`, `path`, `host`, `owner`, `handler`, `guards`, `status`), `PostureKind`
   (`#[non_exhaustive]`, only `Http { method: RequestMethod }` here; #27 adds `Sse` and `WsMessage { event }`, so the
   method or event lives in the kind and no field has to change type later), `PostureStatus` (`Protected`, `Public`,
   `Exempt { reason }`,
   `Unprotected`), `GuardInfo { name, controls_access }`, `RoutePostureError` (`std::error::Error`, holds the
   unprotected entries, `Display` = the message below). `RoutePostureReport` implements `Display` (table) and
   `serde::Serialize`. `UncheckedSurface` from the spike is left to #27, which is the change that fills it.
   - Alternative: define every variant now. Rejected: variants nothing produces would be dead API until #27, and
     `#[non_exhaustive]` makes adding them non-breaking.

8. **Error and logs.** `init()` returns `anyhow::Error::new(RoutePostureError { .. })`, logged once with
   `Logger::new("RoutePosture").error(..)`. Message (spike decision 6):
   ```text
   Route posture check failed: 2 routes have no guard and are not marked #[public].
     GET   /api/rooms                   RoomsController::list
     POST  /api/rooms/:room/messages    RoomsController::post
   Add #[use_guards(...)] to the route or its controller (or register a global guard), mark it #[public] if it is
   meant to be open, or add an exemption with a reason. Guards that do not control access (ThrottlerGuard, ZodGuard)
   do not count.
   ```
   Singular form for one route. Warn mode logs the same text with `warn`. Stale exemptions log one `warn` line each
   ("Route posture exemption GET /metrics matches no route"), in warn and enforce mode. Nothing is logged in off mode.

9. **`route_posture()` computes on demand.** `CorvisApplication::route_posture(&self) -> Option<RoutePostureReport>`
   returns `None` before `init()`; afterwards it builds the report from `AppShared::router` with the configured
   exemptions (none in off mode). The startup check and this method share one function, so they cannot disagree, and
   off mode does no extra work during `init()`.
   - Alternative: compute once in `init()` and store it. Rejected: work in off mode, and #27 wants a live count of
     `upgrade` listeners when the report is read after the hooks.

10. **Tests.** New `crates/corvis/tests/route_posture.rs` drives apps through `init()`, `listen_in_background(0)` and
    `route_posture()`, and asserts on `err.downcast_ref::<RoutePostureError>()`. "No socket bound" is asserted through
    `app.get_url() == None`; "no hook ran" with an `OnModuleInit` provider that sets a flag; logs with a capturing
    `LoggerService` installed through `use_logger`. The `#[public(x)]` compile error is a `compile_fail` doctest on
    `PUBLIC_METADATA` in `crates/corvis/src/lib.rs` (no trybuild in the workspace; a doctest needs no new
    dependency).

## Risks / Trade-offs

- [A pass-through guard (`RolesGuard` alone, which returns `Ok(true)` when no roles are set) counts as protection] →
  the report lists guard names per route; the `controls_access` doc comment and the guards page say to override it for
  such guards.
- [`#[public]` shares `"isPublic"` with user code] → intended (existing guards honor it); an app using that key for
  something else would see routes counted as public. Documented.
- [Broad exemption patterns hide routes] → each exempt route shows its reason in the report; stale exemptions warn.
- [New trait methods could clash with inherent methods of the same name on a user guard] → no compile error; posture
  calls them through `dyn CanActivate`, which always uses the trait method.
- [Logging uses the global logger, which tests share] → log assertions use a capturing logger and serialize those
  tests (or check the returned report/error instead where possible).
- [`#[public]` re-exported in the prelude can shadow nothing but could surprise a user with their own `public`
  attribute] → glob imports are shadowed by explicit ones, so no conflict error.

## Open questions

1. Should `require_route_posture()` / `set_route_posture()` called after `init()` log a warning (they have no effect
   then), like other late builder calls? Proposed: no, consistent with `enable_versioning` today; docs say "before
   `init()`".
2. Display of `guard_name()` for closures or generic guards (`type_name` includes generic parameters): shortened with
   `short_type_name`, generics kept. Acceptable?
3. `PostureEntry::host`: included so two host-specific controllers on the same path are distinguishable in the report.
   Keep it, or leave hosts out until someone needs them?

## Review notes (2026-11-08)

- No warning when `require_route_posture()` is called after `init()`; `short_type_name` for guard names; keep `host`.
- The check placement before `build_middleware` (deviation from the spike, so no user `after_init` has run) is
  accepted.
- Archive #26 before #27 (#27 modifies the capability #26 creates).
