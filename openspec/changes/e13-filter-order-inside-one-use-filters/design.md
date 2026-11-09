# Design

## Context

See proposal.md for motivation; the behavior is in the two spec deltas. Current state, verified in the code on
2026-11-08:

**Where Corvis orders filters**

There are four places where a `#[use_filters]` list is turned into code, all through the same helper, and two places
where the lists are concatenated at startup.

- `enhancer_tokens(kind, exprs)` (`crates/corvis-macros/src/utils.rs:95-108`) flattens array arguments
  (`flatten_array`, `utils.rs:39-48`) and maps each expression, in order, to `::corvis::filter_of::<T>()` or
  `::corvis::filter_instance(e)`.
- Controllers: `#[controller]` maps `use_filters` below it to `.filter(..)` calls on `ControllerMetadata`, one per
  token in declaration order (`crates/corvis-macros/src/controller.rs:102-106`).
- Routes: `#[routes]` does the same per method (`crates/corvis-macros/src/controller.rs:269-273`), building
  `RouteDef::filter` calls.
- Gateways: `#[websocket_gateway]` maps `use_filters` to the builder name `filter` (`crates/corvis-macros/src/gateway.rs:60`)
  and emits `.filter(..)` per token in order (`gateway.rs:71-74`).
- `#[subscribe_message]` methods: `#[handlers]` maps `use_filters` to `filter` (`crates/corvis-macros/src/handlers.rs:98`)
  and emits the calls in order (`handlers.rs:103-107`).
- Attributes are processed in source order (top to bottom) in all four macros, so stacked `#[use_filters]` attributes
  append in that order.
- The builders push in call order: `ControllerMetadata::filter` (`crates/corvis/src/router/controller.rs:92-95`),
  `RouteDef::filter` (`controller.rs:210-213`), `WsEnhancers::filter` (`crates/corvis/src/discovery.rs:49-52`).
- HTTP: `build_router` builds each route's list as method filters, then controller filters, then
  `cfg.global_filters` (`crates/corvis/src/router/mod.rs:409-411`). `handle_exception` (`router/mod.rs:864-876`) walks
  the list and the first filter whose `catches` is `true` answers.
- Global filters: `init()` reverses the `use_global_filters` list and appends the provider filters reversed
  (`crates/corvis/src/application.rs:534-539`), the original's order (#17).
- Gateways: `compile_gateways` builds each handler's list as handler filters then gateway filters
  (`crates/corvis/src/websockets.rs:591-592`); `exception_data` picks the first filter whose `catches` is `true`
  (`websockets.rs:849`).
- `#[apply_decorators]` cannot add filters: `CustomDecorator` only holds metadata entries
  (`crates/corvis/src/common/decorator.rs:33-36`). Middleware has no filters. SSE routes go through `build_router`.

So within one attribute and across stacked attributes, Corvis tries filters in source order.

**Original framework (read from `@nestjs/core` 11.2.1 and `@nestjs/common` 11.2.1 in the facten original's
`node_modules`; the gateway fixture of #116 recorded the same order with 12.4.0)**

- `ContextCreator.createContext` (`@nestjs/core/helpers/context-creator.js`) returns
  `[...global, ...class, ...method]`.
- `RouterExceptionFilters.create` (`@nestjs/core/router/router-exception-filters.js`) calls
  `exceptionHandler.setCustomFilters(filters.reverse())`, and `ExceptionsHandler.invokeCustomFilters`
  (`@nestjs/core/exceptions/exceptions-handler.js`) uses the first filter whose `@Catch` types match.
- `UseFilters(...filters)` (`@nestjs/common/decorators/core/exception-filters.decorator.js`) calls
  `extendArrayMetadata`, which appends to the existing list (`@nestjs/common/utils/extend-metadata.util.js`).
  TypeScript applies stacked decorators bottom to top, so `@UseFilters(A, B)` above `@UseFilters(C)` stores
  `[C, A, B]`; reversed, the search order is `B, A, C`.
- Net effect: method level before class level before global; inside one decorator, last listed first; between
  stacked decorators, the upper one first.
- The gateway side (`ExceptionFiltersContext.create` in `@nestjs/websockets`) reverses the combined class + method
  list the same way; the #116 fixture recorded `Second` answering for `@UseFilters(First, Second)` on a handler
  (`crates/corvis/tests/fixtures/ws_gateways/ws_gateways.json:273-292`).

**Tests and docs that encode the current order**

- `filters_of_one_attribute_are_tried_in_declaration_order_like_http` (`crates/corvis/tests/ws_enhancers.rs:694-700`)
  asserts `First` answers for `#[use_filters(First, Second)]` (`ws_enhancers.rs:360-365`).
- The fixture replay special-cases `filters_in_one_use_filters` and asserts the difference
  (`ws_enhancers.rs:969-975`, gateway `FxOrder` at `ws_enhancers.rs:879-891`).
- No HTTP test covers two filters in one attribute; `enhancers_routing.rs:66-80` covers global filter order only.
- No committed HTTP fixture records filter order; #17 checked the global order by hand
  (`openspec/changes/archive/2026-11-08-e13-enhancers-and-routing-edge-cases/design.md`, Implementation notes).
- Docs: `docs/exception-filters.md:268-276` (route, controller, global; no word on order inside one attribute),
  `docs/faq/request-lifecycle.md:75-85` (same), `docs/websockets/exception-filters.md:39-41` (declaration order and
  the noted difference with the original).

**Port apps (parity risk check, done while drafting)**

- Originals (`C:/Users/Usuario/Devel/corvis-tests/nest/{consi-backend,facten-api,houseplan}`, excluding
  `node_modules`): no `@UseFilters` anywhere. The only filter is consi's `AllExceptionsFilter` (`@Catch()`), bound with
  `app.useGlobalFilters(...)` in `consi-backend/src/main.ts:21`.
- Ports (`C:/Users/Usuario/Devel/corvis-tests/corvis/{consi-backend,facten-api,houseplan-api}`): no `#[use_filters]`;
  consi's port calls `app.use_global_filters(AllExceptionsFilter::new())` (`consi-backend/src/main.rs:65`). None of the
  three has a gateway (recorded in #116).
- So no route of the ports has two catching filters in one decorator, and the change should not move apidiff. Task 5.1
  repeats the grep on the current sources before running the parity check.

## Goals / Non-Goals

**Goals:**
- Filters declared with `#[use_filters]` are tried in the original framework's order on HTTP routes, controllers,
  gateways and message handlers.
- One rule for both transports, so a filter list behaves the same on a route and on a gateway.
- The order is pinned by differential fixtures, not by reading the original's source.

**Non-Goals:**
- Changing level order (method, class, global) or global filter order.
- Matching filters by exception type like `@Catch(...)`; Corvis keeps `catches`.

## Decisions

1. **Reverse each attribute's list in the macros, not the combined list at runtime.** Add a helper in
   `crates/corvis-macros/src/utils.rs`, for example `filter_tokens_in_try_order(exprs) -> Vec<TokenStream>`, that
   calls `enhancer_tokens("filter", exprs)` and reverses the result. Use it at the four sites listed in Context
   (controller class, route method, gateway, handler). Stacked attributes keep appending in source order, which
   matches the original as shown in Context. The runtime (`build_router`, `compile_gateways`, `handle_exception`,
   `exception_data`) is unchanged and keeps reading the lists as "try in this order".
   Alternatives considered: (a) reverse the method and class lists in `build_router` and `compile_gateways`: simpler
   to read, but it also reverses stacked attributes (`C, B, A` instead of the original's `B, A, C`), and the builders
   would then mean "reverse declaration order" for hand-written metadata; rejected. (b) reverse inside
   `enhancer_tokens` for `kind == "filter"`: the helper is also the place that knows nothing about order for guards,
   interceptors and pipes; a dedicated helper makes the asymmetry visible at each call site; rejected as less clear.
2. **The `filter` builders keep push order.** `ControllerMetadata::filter`, `RouteDef::filter` and
   `WsEnhancers::filter` stay as they are; their rustdoc says that filters are tried in the order they are added and
   that `#[use_filters(A, B)]` adds `B` then `A`. Code that builds metadata by hand (tests, custom macros) keeps its
   behavior.
   Alternative considered: a new builder taking a group and reversing it (`filters_group(vec![..])`), which would also
   keep expression evaluation order (see Risks). Rejected: new public API for a corner case.
3. **Expression evaluation order follows the try order.** With the reversal in the macro,
   `#[use_filters(A::new(), B::new())]` evaluates `B::new()` before `A::new()` when the metadata function runs, and
   DI-resolved filters are resolved in try order by `resolve_all` (`router/mod.rs:350-359`). Filter constructors with
   observable side effects are not a supported pattern; the docs do not promise an evaluation order. Accepted.
4. **A new HTTP differential fixture, and new scenarios in the gateway fixture.** HTTP:
   `crates/corvis/tests/fixtures/http_filter_order/` (`package.json` pinning `@nestjs/common`, `@nestjs/core` and
   `@nestjs/platform-express` at 11.2.1 like `fixtures/empty_results`, plus `reflect-metadata` and `rxjs`;
   `generate.mjs` bound to `127.0.0.1`; output `http_filter_order.json`). It applies decorators by calling them as
   functions, as `ws_gateways/generate.mjs` does, so stacking is simulated by applying the lower decorator first.
   Scenarios, each recording which filter answered:
   - method `UseFilters(First, Second)`, both catch-all;
   - class `UseFilters(First, Second)`, both catch-all;
   - method `UseFilters(A, B)` stacked above `UseFilters(C)`, with overlapping `@Catch` sets so the full order is
     observable (for example `B` catches only `ForbiddenException`, `A` catches `ForbiddenException` and
     `ConflictException`, `C` catches everything; requests that raise each of the three exceptions);
   - class `UseFilters(C1, C2)` with method `UseFilters(M1, M2)`, with `@Catch` sets such that only `M1` and `C2`
     catch one exception;
   - one global filter plus class and method filters, to confirm the global one is last.
   Rust replays in `crates/corvis/tests/enhancers_routing.rs` use `catches` to mirror the `@Catch` sets.
   Gateway: `crates/corvis/tests/fixtures/ws_gateways/generate.mjs` gains `gatewayFilters: [First, Second]` and a
   stacked case (`makeGateway` gets a `methodFilterGroups` option applied last group first); the replay in
   `ws_enhancers.rs` drops the special case and compares like every other scenario.
   Alternative considered: extend `ws_gateways` with HTTP routes (it already depends on `@nestjs/platform-express`
   12.4.0). Rejected: one fixture per transport keeps each generator small; the version difference is noted in Open
   questions.
5. **Behavior change, no switch.** Unlike #116, there is no opt-out: it only changes which of two filters that both
   catch an exception answers, the fix is to swap the two names, and an opt-out would keep two orders alive in the
   docs. The migration guide and the change summary say it.
   Alternative considered: `app.legacy_filter_order(true)`. Rejected for the reasons above.

## Risks / Trade-offs

- [Apps with two catching filters in one attribute get the other filter answering] → documented as a behavior change
  (migration guide, change summary); the remedy is to swap the names in the attribute.
- [Constructor and DI resolution order of filters in one attribute is reversed] → Decision 3; no known code depends on
  it.
- [The HTTP fixture pins 11.2.1 while the gateway fixture pins 12.4.0] → both versions reverse the list (11.2.1 read
  directly, 12.4.0 recorded by the gateway fixture); Open question 1.
- [Hand-written metadata and macro-generated metadata now differ in how a "list" is written] → Decision 2 documents
  the builders; only the attribute syntax mirrors the original decorator.

## Open questions

1. **Fixture version.** The HTTP fixture pins 11.2.1 (the facten original's version, like `empty_results`); the gateway
   fixture pins 12.4.0. Confirm 11.2.1 for HTTP, or move it to 12.4.0 so both fixtures match.
2. **Migration guide wording.** This proposal adds a behavior-change bullet next to the #116 gateway bullet in
   `docs/migration-guide.md`. Confirm, or move release notes elsewhere if a changelog is introduced.

## Review notes (2026-11-08)

1. The new HTTP fixture pins the same version as the existing HTTP fixtures (11.2.1); unifying it with the gateway fixture (12.4.0) is not part of this change.
2. The behavior change is noted in `docs/migration-guide.md` (EN + ES), with no switch.

## Implementation notes

- Fixtures: `http_filter_order` pins 11.2.1 (review note 1) and records six scenarios (method attribute, class
  attribute, stacked method attributes, stacked class attributes, method before class, global last). The recorded order
  matches the spec delta (`Second`; `B` for the stacked case; `M1`; `C2`). `ws_gateways.json` only gained the two new
  keys (checked with a diff). Both fixtures were recorded with `npm ci` in the scratchpad, not in the repo; the new
  fixture folder has its `node_modules` ignored in `.gitignore`, like the others.
- Single-capital-letter names such as `A` or `B` count as constants for the macros (`is_const_like`), so
  `#[use_filters(A, B)]` would build instances (`filter_instance`) instead of resolving through DI. The test filters
  are therefore named `FltA`, `FltB`, ... in the Rust replays and `FilterA`, ... in the macro unit tests. Not a
  change of behavior.
- The fixture's `B` catches only a `RangeError` (HTTP: only `ForbiddenException`); in the gateway replay it is a
  filter whose `catches` is always `false` and that records nothing, so `calls` match the fixture.
- Corvis answers with the exception's status when a filter returns `Response::json(..)` without a status; the replay
  filters call `.with_status(200)` to mirror the fixture's `res.status(200).json(..)`. The array-form scenario is not
  in the fixture (the original has no array form); a Rust test covers it.
- The gateway attribute and stacked-handler tests reuse the `FxModule` gateways (`/fx_gateway_order`, `/fx_stacked`)
  and the fixture replay; no separate gateway was added to `AppModule`.
- The red-first step of 2.1 to 2.3 was not run as a separate step: the tests assert the new order only and the macro
  expansion showed `B, A, C` as soon as the helper existed.
- Port apps grep (task 5.1, 2026-11-08): `UseFilters` in `nest/{consi-backend,facten-api,houseplan}` and
  `use_filters` in `corvis/{consi-backend,facten-api,houseplan-api}` (sources only, excluding `node_modules`,
  `target`, `dist`): 0 files each. No route has two filters in one decorator.
- Task 6.5 (corvis-parity) is left to the maintainer's session, as instructed.
