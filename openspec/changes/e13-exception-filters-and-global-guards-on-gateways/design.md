# Design

## Context

See proposal.md for motivation; the behavior is in the two spec deltas. Current state, verified in the code:

**Gateway enhancers today**

- `WsEnhancers` (`crates/corvis/src/discovery.rs:27-35`) holds `guards`, `interceptors`, `pipes`, `metadata` and
  `params`. There is no `filters` field and no `filter()` builder (builders at `discovery.rs:37-58`).
- `#[websocket_gateway]` (`crates/corvis-macros/src/gateway.rs:42-73`) turns `#[use_guards]`, `#[use_interceptors]`,
  `#[use_pipes]` and `#[set_metadata]` below it into `WsEnhancers` builder calls, and returns
  `"#[use_filters] is not supported on WebSocket gateways"` for `#[use_filters]` (`gateway.rs:60-62`).
- `#[handlers]` (`crates/corvis-macros/src/handlers.rs:89-117`) maps `use_guards`, `use_interceptors` and `use_pipes`
  on a method (`handlers.rs:92-94`); `use_filters` is not in that match, so it is kept on the method and expands the
  `misplaced!` stub, whose message only mentions controllers (`crates/corvis-macros/src/lib.rs:199`). Enhancers on a
  method that is not `#[subscribe_message]` are rejected at `handlers.rs:119-123`.
- `enhancer_tokens("filter", ..)` (`crates/corvis-macros/src/utils.rs:95-108`) already produces
  `::corvis::filter_of::<T>()` / `::corvis::filter_instance(e)`, the same refs controllers use
  (`crates/corvis-macros/src/controller.rs:102-106`).

**Gateway runtime**

- `CompiledWsHandler` (`crates/corvis/src/websockets.rs:516-524`) stores `guards`, `interceptors`, `pipes`, `params`
  and `target`; `GatewayRuntime` (`websockets.rs:527-534`) stores the gateway's `class_target` and `module_ref`.
- `build_gateways(container, _cfg: &RouterConfig)` (`websockets.rs:544-599`) resolves the gateway's enhancers, then
  each handler's, appending handler after gateway. The router configuration is ignored (`_cfg`).
- `run_handler` (`websockets.rs:697-730`) builds the `ExecutionContext::for_ws` context, runs the guards (a `false`
  becomes `WsException::new("Forbidden resource")`, line 713), then interceptors (`call_interceptors`, 732-759), then
  pipes and the handler (`invoke_handler`, 761-776).
- `dispatch` (`websockets.rs:780-822`) catches panics, echoes the incoming `id`, logs internal errors through
  `WsExceptionsHandler` and sends `{ event: "exception", data: exception_payload(&e) }` (line 819).
  `exception_payload` (`websockets.rs:127-147`) is the built-in mapping.
- The context is created inside `run_handler`, so `dispatch` has no context to give a filter.

**Global enhancers**

- `RouterConfig` (`crates/corvis/src/router/mod.rs:237-246`) holds `global_guards`, `global_interceptors`,
  `global_pipes` and `global_filters`. `use_global_*` push into it (`crates/corvis/src/application.rs:159-181`);
  `init()` inserts the provider-registered ones ahead of them (`application.rs:361-397`) and only then calls
  `build_router` and `build_gateways` (`application.rs:400-402`), so `cfg` already has every global enhancer when
  gateways are built.
- HTTP routes order enhancers global → controller → method for guards, interceptors and pipes, and method →
  controller → global for filters (`build_router`, `router/mod.rs:310-321`); `handle_exception`
  (`router/mod.rs:745-761`) uses the first filter whose `catches` is true and fills the status if the filter left it
  empty.
- `ExceptionFilter::catch(exception, host: &ArgumentsHost)` (`crates/corvis/src/common/filter.rs:31-38`), where
  `ArgumentsHost` is `ExecutionContext` (`crates/corvis/src/common/context.rs:63-64`), whose `switch_to_ws()` gives the
  client and data (`context.rs:165-178`).
- `Body` (`crates/corvis/src/http/response.rs:45-52`): `Empty`, `Json`, `Text`, `Html`, `Bytes`, `Stream`.

**Original framework (source read from its repository, master branch, on 2026-11-08; the differential fixture re-checks the pinned
version)**

- `packages/websockets/socket-module.ts` builds `WsContextCreator` with `new GuardsContextCreator(container, config)`,
  `new PipesContextCreator(container, config)` and `new InterceptorsContextCreator(container, config)`: global guards,
  pipes and interceptors apply to gateways. The `ExceptionFiltersContext` is built as `new
  ExceptionFiltersContext(container)` and its `getGlobalMetadata()` returns `[]`: global filters do not.
- `ExceptionFiltersContext.create` reverses the combined class + method filter list (`filters.reverse()`), so method
  filters are tried before class filters.
- `ws-context-creator.ts`: guards → interceptors → pipes → handler; a guard returning `false` throws
  `new WsException(FORBIDDEN_MESSAGE)`. Corvis already matches this (`websockets.rs:711-714`).

**Tests and example**

- `crates/corvis/tests/websockets.rs`: `gateway_guards_run_with_a_ws_execution_context` (line 305) and
  `interceptors_exceptions_and_acknowledgements` (line 359) cover the current guard, interceptor and frame behavior,
  including `id` echo (line 381-388).
- `examples/cats-app/src/app_module.rs:31-33` registers `ThrottlerGuard`, `AuthGuard` and `RolesGuard` with
  `Provider::app_guard`; `examples/cats-app/src/auth/auth_guard.rs:15-27` skips targets with `IS_PUBLIC_KEY` metadata
  and otherwise requires a bearer token; `RolesGuard` passes when no roles are required
  (`examples/cats-app/src/common/guards.rs:13-16`). The `/chat` gateway (`examples/cats-app/src/chat.rs:7`) has no
  metadata and its doc comment tells users to connect with `websocat` and no token.

## Goals / Non-Goals

**Goals:**
- A global guard protects gateway messages by default, as in the original framework.
- Exception filters work on gateways and handlers with the HTTP trait, so one filter type can serve both transports.
- Existing applications can keep the old behavior with one call.

**Non-Goals:**
- Global filters on gateways (the original does not apply them), the `cause` field of the original's frames, and the
  mapping of plain `HttpException`s (see proposal, Out of scope).
- Route posture checks for gateways (#27).

## Decisions

1. **`WsEnhancers` gains `filters`, and both macros accept `#[use_filters]`.** Add `pub filters:
   Vec<FilterRef>` and `.filter(f)` to `WsEnhancers`. In `gateway.rs`, map `"use_filters" => "filter"` like the other
   enhancers; in `handlers.rs`, add `"use_filters" => Some("filter")`, so the existing "only on
   `#[subscribe_message]`" check (`handlers.rs:119-123`) also covers filters. Update the `misplaced!` message for
   `use_filters` to mention `#[websocket_gateway]` and `#[subscribe_message]`.
   Alternative considered: a separate `#[use_ws_filters]`. Rejected: one attribute, one trait, as for guards.
2. **Filters run in `dispatch`, with the context built there.** Move the `ExecutionContext::for_ws` construction from
   `run_handler` to `dispatch` and pass it in. On `Err(e)`, `dispatch` tries `handler.filters` (handler then gateway,
   resolved and concatenated in `build_gateways`) with the same first-match rule as `handle_exception`. The filter call
   is wrapped in `catch_unwind`. The ordering inside one level reuses whatever helper #17 settles for HTTP, so both
   transports agree; this change lands after #17 or rebases on it.
   Alternative considered: run filters inside `run_handler`. Rejected: panics are caught in `dispatch`, and a
   panicking handler must reach the filters too.
3. **Translate the filter's `Response` instead of a new trait.** `Json(v)` → `data: v`; `Text(s)` / `Html(s)` →
   `data: "s"`; `Empty` → send nothing; `Bytes` / `Stream` → log an error naming the filter and send
   `exception_payload` of the original exception (cloned before the filter takes it; `HttpException` is `Clone`).
   Status and headers are ignored. Settled in the spike review.
   Alternative considered: let the filter's JSON body be the whole frame when it has an `event` key. Rejected: it
   makes `data` shapes ambiguous; a filter that wants another event emits it and returns an empty response.
4. **Global guards, interceptors and pipes apply; global filters do not.** `build_gateways` starts each handler's
   guard, interceptor and pipe lists from `cfg.global_*` (when the switch is on), then appends the gateway's and the
   handler's, the same order as `build_router`. Filters stay handler → gateway → built-in. This follows the original
   source (Context above). The issue text and the spike said "global guards and global filters"; the review notes of
   the spike say to follow the original model, verified by a differential test, and to include global pipes and
   interceptors only if the original applies them the same way. It does for pipes and interceptors and does not for
   filters, so the spec follows it (parity first, as in the #17 review notes). The differential fixture (task 1.1)
   confirms it before the runtime work starts.
   Alternative considered: also apply global filters, as the issue says. Rejected for parity, and because HTTP-shaped
   global filters (for example `OrmExceptionFilter` answering with a status and a JSON error body) would change every
   gateway's frames on upgrade. See Open questions.
5. **The switch is an application method stored in `RouterConfig`.** `apply_global_enhancers_to_gateways(bool)`
   writes `RouterConfig::gateway_global_enhancers` (default `true`); `build_gateways` reads it. Called after `init()`
   it logs a warning and does nothing (gateways are built once).
   Alternatives considered: a `CorvisApplicationOptions` field (would force every caller of the options struct to
   learn a gateway setting), or a per-gateway attribute (`#[websocket_gateway(global_enhancers = false)]`). The
   per-gateway form is useful but not asked for; metadata checked by the guard already covers per-gateway opt-outs.
6. **cats-app marks the gateway public.** Add `#[set_metadata(IS_PUBLIC_KEY, true)]` below `#[websocket_gateway]` in
   `examples/cats-app/src/chat.rs`, and say in its doc comment that the global `ThrottlerGuard` now also counts chat
   messages (100 per minute per IP and handler, `app_module.rs:19`).
   Alternative considered: make `AuthGuard` skip `ContextType::Ws`. Rejected: the example would teach that WebSocket
   traffic is unauthenticated by default; marking one gateway public is explicit.
7. **Differential fixture like `crates/corvis-orm/tests/decimal_js.rs`.** A Node script under
   `crates/corvis/tests/fixtures/ws_gateways/` (with a `package.json` pinning `@nestjs/core`, `@nestjs/common`,
   `@nestjs/websockets`, `@nestjs/platform-ws` and `ws`) runs a gateway over the native `ws` adapter, whose frames
   have the same `{ event, data }` shape as Corvis, and records the frames for: a global guard (`useGlobalGuards` and
   `APP_GUARD`) refusing a message; a global pipe and a global interceptor on a message; `APP_FILTER` and
   `useGlobalFilters` filters with a failing handler; class and method `@UseFilters` both catching. The recorded JSON
   is committed; the Rust test replays the same gateway and compares frames, ignoring the `cause` field (Out of scope)
   and the `id` echo (a Corvis extension).

## Risks / Trade-offs

- [Apps with a global guard and a gateway start rejecting messages on upgrade] → documented as a behavior change in
  the docs and the change summary, with three remedies (metadata, `ctx.get_type()`, the switch); cats-app shows the
  metadata one.
- [A global `ThrottlerGuard` now counts WebSocket messages per handshake IP] → documented in the rate-limiting page;
  #120 makes the guard send no headers in a WebSocket context.
- [Global interceptors written for HTTP (reading `req.method()`, setting headers) now see WebSocket contexts] → same
  as in the original; the docs tell interceptor authors to check `ctx.get_type()`.
- [Global pipes such as `ZodValidationPipe::auto()` now run on message bodies] → that is what the docs already
  recommend repeating on gateways (`docs/websockets/gateways.md:287`); the duplicated gateway pipe now runs twice,
  which is harmless for validation pipes but documented.
- [The issue asked for global filters and this change does not apply them] → recorded in Open questions for the
  maintainer.

## Open questions

1. **Global filters.** The original does not apply global filters to gateways, and this change follows it. Confirm, or
   decide to diverge (apply them, or add an opt-in such as `apply_global_filters_to_gateways(true)`), which would
   contradict the issue's parity goal less than it seems because filters only shape errors.
2. **Order within one `#[use_filters(A, B)]`.** The original reverses the list, so `B` is tried before `A`. HTTP routes
   in Corvis try `A` first today; #17 decides the HTTP order, and gateways reuse it. If #17 lands without settling it,
   the maintainer chooses before task 3.2.
3. **Switch name.** The original's documentation says `useGlobalGuards()` does not set up guards for gateways in
   hybrid applications; Corvis has no hybrid mode, so the switch stands in for that case. Confirm the name
   `apply_global_enhancers_to_gateways`.

## Review notes (2026-11-08)

1. Follow the original: global guards, pipes and interceptors apply to gateways, global filters do not (the
   differential fixture of task 1.1 confirms it before runtime changes). This refines the issue text and the #23
   review note; no opt-in for global filters on gateways in this change.
2. Filter order inside one `#[use_filters(A, B)]` follows whatever #17 settles for HTTP (which follows the original).
3. `apply_global_enhancers_to_gateways(false)` is accepted as the switch name; #27 uses it.
