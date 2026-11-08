# Tasks

## 1. Confirm the reference behavior

- [x] 1.1 Write a minimal Node app with the original framework (two global filters as providers, one bootstrap global filter, all catching everything) in the scratchpad, record which filter answers; record the result in design.md under Open questions and stop for a maintainer decision if it is not registration order; verify by attaching the recorded output to the change
- [x] 1.2 Add failing regression tests first, one per spec scenario group, in `crates/corvis/tests/enhancers_routing.rs` (new): provider filter order, `override_guard` on `app_guard`, overriding an `APP_*` provider, prefix exclusion per route, middleware under URI versioning, optional params; verify with `cargo test -p corvis --test enhancers_routing` that they fail for the expected reason on the current code

## 2. Global enhancers and overrides

- [x] 2.1 In `CorvisApplication::init()` (`crates/corvis/src/application.rs`), replace `global_filters.insert(0, x)` with an increasing `filter_at` index; verify the provider filter order tests pass and `crates/corvis/tests/enhancer_order.rs` still passes
- [x] 2.2 Record `enhancer_override_token::<T>()` on providers built by `Provider::app_guard/app_interceptor/app_pipe/app_filter` (`crates/corvis/src/di/provider.rs`), and in `init()` use the override value from the core module when present, keeping the position; verify the `override_guard` / `override_filter` on `app_*` tests pass and `crates/corvis/tests/app.rs` override test still passes
- [x] 2.3 In `Scanner::add_provider` (`crates/corvis/src/di/container.rs`), keep `enhancer` and the override token of the original provider when an override replaces it; make the `GlobalEnhancer` casts also accept the `Arc<dyn Trait>` form; make `init()` return an error naming the enhancer when the instance is missing or cannot be cast; verify the compatible and incompatible override tests pass

## 3. Routing

- [x] 3.1 In `build_router` (`crates/corvis/src/router/mod.rs`), decide the prefix per route from `join_paths(&[c_path, r_path])` instead of per controller; keep the resolver log line cosmetic; verify the three prefix exclusion scenarios and `crates/corvis/tests/express_parity.rs` pass
- [x] 3.2 In `build_middleware`, when URI versioning is enabled, compile each text pattern to the unversioned path plus one path per published version segment, for both `include` and `exclude`; verify the three middleware versioning scenarios and `crates/corvis/tests/app.rs` middleware tests pass
- [x] 3.3 Add a helper that expands `:name?` segments into all variants (startup error above 4 optional segments) and register every variant in `build_router`; keep `to_matchit` from silently dropping `?` and extend the unit tests near `router/mod.rs:789`; verify the optional parameter scenarios, the unit tests and `cargo test -p corvis --lib fuzz` pass
- [x] 3.4 Add a test showing that an expanded optional route conflicting with an explicit route is reported like any other conflict; verify it passes

## 4. Docs

- [x] 4.1 `docs/fundamentals/testing.md`: overrides now cover `Provider::app_*`; keep the warning only for expression enhancers and `use_global_*`; same in `docs/es/fundamentals/testing.md`; verify both pages read the same
- [x] 4.2 `docs/exception-filters.md` and `docs/faq/request-lifecycle.md`: provider filters in registration order; same in `docs/es/`
- [x] 4.3 `docs/faq/global-prefix.md`: replace the "applies per controller route" warning with the per-route rule and the `"name/*"` example, noting the behavior change; same in `docs/es/faq/global-prefix.md`
- [x] 4.4 `docs/middleware.md`: replace the URI versioning warning with the new rule; `docs/controllers.md` route patterns: document `:name?`; same in `docs/es/`
- [x] 4.5 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes, including the regression tests of group 1.2 (batch verification: 467 passed, 0 failed; the fuzz_filters NUL failure found here was filed and fixed as #141)
- [x] 5.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 5.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences (facten exercises `set_global_prefix_with_exclude` and two `app_guard`s) (maintainer batch run on batch/phase03-a 085adae: consi 832/832, facten 727/727, houseplan 598/598 quiet-hours count; routes identical, databases equivalent)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #17, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
