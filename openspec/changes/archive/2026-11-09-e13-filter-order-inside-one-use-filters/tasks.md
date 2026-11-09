# Tasks

## 1. Reference behavior

- [x] 1.1 Add `crates/corvis/tests/fixtures/http_filter_order/` with a `package.json` pinning `@nestjs/common`,
  `@nestjs/core` and `@nestjs/platform-express` (11.2.1, unless Open question 1 says otherwise), `reflect-metadata` and
  `rxjs`, and a `generate.mjs` that starts an Express app on `127.0.0.1`, applies the decorators as functions (lower
  stacked decorator first) and records which filter answered each request of the scenarios in design.md, Decision 4,
  into `http_filter_order.json`; if the recorded order differs from the spec delta `core/enhancers`, stop and record
  it under Open questions in design.md; verify by running `node generate.mjs` once and committing the JSON (not
  `node_modules`)
- [x] 1.2 Extend `crates/corvis/tests/fixtures/ws_gateways/generate.mjs`: a `filters_in_one_gateway_use_filters`
  scenario (`gatewayFilters: [First, Second]`) and a `stacked_handler_use_filters` scenario (new `methodFilterGroups`
  option, groups applied last first, with `@Catch` sets that make the order observable); regenerate
  `ws_gateways.json` and check that the existing scenarios did not change; verify with `git diff` on the JSON (only
  the new keys are added)

## 2. Failing tests

- [x] 2.1 In `crates/corvis/tests/enhancers_routing.rs`, add tests for the `core/enhancers` scenarios (method
  attribute, controller attribute, stacked attributes, method before controller, array form) and a replay of
  `http_filter_order.json` that builds the same routes with `catches` mirroring the `@Catch` sets; verify with
  `cargo test -p corvis --test enhancers_routing` that the new tests fail because `First` (declaration order) answers
- [x] 2.2 In `crates/corvis/tests/ws_enhancers.rs`, rename `filters_of_one_attribute_are_tried_in_declaration_order_like_http`
  to `filters_of_one_attribute_are_tried_last_listed_first` and expect `Second`; add tests for the gateway-attribute
  and stacked-handler scenarios of the `ws/exception-filters` delta; remove the `filters_in_one_use_filters` special
  case in the fixture replay (`ws_enhancers.rs:969-975`) and add `FxModule` gateways for the two new fixture
  scenarios; verify with `cargo test -p corvis --test ws_enhancers` that these fail on the current code
- [x] 2.3 Add a unit test in `crates/corvis-macros` (next to the existing ones in `controller.rs` and `handlers.rs`)
  that expands `#[use_filters(A, B)] #[use_filters(C)]` on a route and on a `#[subscribe_message]` method and asserts
  the generated `.filter(..)` calls appear in the order `B`, `A`, `C`; verify with `cargo test -p corvis-macros`
  that it fails on the current code

## 3. Macros

- [x] 3.1 Add the helper of design.md, Decision 1, to `crates/corvis-macros/src/utils.rs` and use it in
  `controller.rs` (class-level `use_filters`, lines 102-106, and method-level, lines 269-273), `gateway.rs` (the
  `filter` builder branch, lines 60 and 71-74) and `handlers.rs` (lines 98 and 103-107), leaving guards, interceptors
  and pipes untouched; verify that the tests of 2.1, 2.2 and 2.3 pass and that
  `cargo test -p corvis --test app --test enhancer_order --test websockets` still passes
- [x] 3.2 Update the rustdoc of `ControllerMetadata::filter`, `RouteDef::filter` (`crates/corvis/src/router/controller.rs`)
  and `WsEnhancers::filter` (`crates/corvis/src/discovery.rs`): filters are tried in the order they are added, and
  `#[use_filters(A, B)]` adds `B` then `A`; verify with `cargo doc -p corvis --no-deps` without warnings

## 4. Docs

- [x] 4.1 `docs/exception-filters.md` ("Which filter responds"): inside one `#[use_filters(A, B)]` the last listed is
  tried first, stacked attributes keep the upper one first, with a short example; same in
  `docs/es/exception-filters.md`
- [x] 4.2 `docs/faq/request-lifecycle.md` ("Exception filters"): the same rule in one sentence; same in
  `docs/es/faq/request-lifecycle.md`
- [x] 4.3 `docs/websockets/exception-filters.md` (lines 39-41): replace "in the order they are written" and remove the
  paragraph about the difference with the original; same in `docs/es/websockets/exception-filters.md`
- [x] 4.4 `docs/migration-guide.md`: a behavior-change bullet (filters inside one `#[use_filters]` are now tried last
  listed first, as in the original; swap the names to keep the old behavior); same in `docs/es/migration-guide.md`
- [x] 4.5 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Port apps

- [x] 5.1 Repeat the parity risk grep on the current sources: `@UseFilters` in the three originals under
  `C:/Users/Usuario/Devel/corvis-tests/nest/` and `#[use_filters` in the three ports under
  `C:/Users/Usuario/Devel/corvis-tests/corvis/` (excluding `node_modules` and `target`); record the result in
  design.md, Implementation notes; verify that no decorator or attribute lists two filters (if one does, list the
  route and which filter answers before and after)

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 6.3 `cargo test --workspace` passes, including the regression tests of 2.1 to 2.3 and both fixture replays
- [x] 6.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 6.5 Generated framework code changed: run the `corvis-parity` check and verify the three port apps keep 0
  apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #145, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
