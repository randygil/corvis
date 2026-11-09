# Proposal

Implements [randygil/corvis#145](https://github.com/randygil/corvis/issues/145) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

Inside one `#[use_filters(A, B)]` on a route, a controller, a gateway or a `#[subscribe_message]` method, Corvis tries
`A` first. The original framework tries `B` first: it builds the list global + class + method and reverses it before
searching it (`RouterExceptionFilters.create`, `exceptionHandler.setCustomFilters(filters.reverse())`, and the same
`filters.reverse()` in the gateway filter context). The gateway differential fixture of #116 recorded that order
(`crates/corvis/tests/fixtures/ws_gateways/ws_gateways.json`, scenario `filters_in_one_use_filters`), and #116 kept
declaration order on purpose so gateways and HTTP routes would agree, documenting the difference. Global filters
already follow the original's reverse order (#17, spec `core/enhancers`). A migrated application that declares two
catching filters in one decorator gets a different filter answering in Corvis; this change removes that difference.

## What Changes

- **Behavior change:** filters declared in one `#[use_filters(...)]` are tried last listed first, on HTTP routes,
  controllers, gateways and `#[subscribe_message]` methods. `#[use_filters([A, B])]` (array form) counts as one
  attribute.
- Several `#[use_filters]` attributes stacked on the same item keep today's relative order (the upper attribute's
  filters first), because that is what the original produces for stacked decorators (they apply bottom to top and the
  combined list is then reversed). Each attribute's own list is reversed.
- The level order is unchanged: method (or handler) filters, then controller (or gateway) filters, then global filters
  on HTTP routes (gateways never use global filters).
- The reversal happens in the macros (`#[controller]`, `#[routes]`, `#[websocket_gateway]`, `#[handlers]`), so the
  runtime lists in `build_router` and `compile_gateways` keep meaning "try in this order". The `filter(...)` builders
  of `ControllerMetadata`, `RouteDef` and `WsEnhancers` keep their behavior and their rustdoc says so.
- A new HTTP differential fixture records the original's order (one decorator with two filters on a method and on a
  class, stacked decorators, method and class together); the gateway fixture gains the class-level and stacked cases.
  Rust tests replay both, and the special case in the gateway replay test that asserted the difference is removed.
- Parity risk check of the three port apps (none has two filters in one decorator; recorded in design.md).
- Docs (EN + ES): exception filters, request lifecycle FAQ, WebSocket exception filters (the noted difference is
  removed), migration guide (behavior change bullet).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `core/enhancers`: adds the order of route and controller filters declared with `#[use_filters]`.
- `ws/exception-filters`: the order within one level is now stated explicitly (last listed first inside one
  attribute).

## Impact

- `crates/corvis-macros/src/utils.rs` (a helper that returns one attribute's filter tokens in try order),
  `crates/corvis-macros/src/controller.rs` (class and method `use_filters`), `crates/corvis-macros/src/gateway.rs`,
  `crates/corvis-macros/src/handlers.rs`.
- `crates/corvis/src/router/controller.rs` and `crates/corvis/src/discovery.rs`: rustdoc of the `filter` builders
  only.
- Tests: `crates/corvis/tests/ws_enhancers.rs` (the declaration-order test and the replay special case),
  `crates/corvis/tests/enhancers_routing.rs` (new HTTP order tests), a new fixture
  `crates/corvis/tests/fixtures/http_filter_order/`, and new scenarios in
  `crates/corvis/tests/fixtures/ws_gateways/generate.mjs` and `ws_gateways.json`.
- Docs: `docs/exception-filters.md`, `docs/faq/request-lifecycle.md`, `docs/websockets/exception-filters.md`,
  `docs/migration-guide.md` and their translations under `docs/es/`.
- Users: an app with two filters in one `#[use_filters]` whose `catches` both return `true` for the same exception
  gets the other filter answering after the upgrade. Apps with one filter per attribute, or filters that catch
  disjoint exceptions, see no change.
- Port apps: none of consi, facten or houseplan declares route, controller or gateway filters (design.md, Context),
  so apidiff should not move; the corvis-parity check confirms it because generated code changes.

## Out of scope

- The order of global guards, interceptors and pipes, and of global filters: settled by #17.
- Global exception filters on gateways: the original does not apply them (#116).
- Request-scoped global filters (`getGlobalRequestFilters` in the original, which appends request-scoped `APP_FILTER`
  instances after the static ones): Corvis has no request-scoped enhancers; possible new issue if they are added.
- Filter matching by exception type (`@Catch(...)` metadata) versus Corvis's `catches`: unchanged.
