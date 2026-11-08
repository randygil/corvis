# Tasks

## 1. Test target and helpers

- [x] 1.1 Create `crates/corvis/tests/di/main.rs` with the `mod` declarations of design.md Decision 1 and shared
  helpers: `compile_err(builder) -> DiError` (compiles, expects `Err`, returns `downcast_ref::<DiError>()` cloned, and
  fails the test with the full error otherwise), `assert_cycle(err, &[names])` (variant `Circular`, every name present
  in the chain, chain starts and ends with the same name) and a `next_id()` counter helper; verify
  `cargo test -p corvis --test di` builds and runs with one smoke test that compiles an empty testing module

## 2. Transient scope (`core/injection-scopes`)

- [x] 2.1 Add `tests/di/transient.rs` with the singleton scenario and the three "each consumer gets its own instance"
  scenarios (attribute form, `.scope(Scope::Transient)` form, two `module_ref.resolve` calls); verify
  `cargo test -p corvis --test di transient`
- [x] 2.2 Add the "neither created at startup nor stored" scenarios (constructor not run with no consumer, `app.get`
  returns `NotFound`, `on_module_init` not called after `init()`) and the "scope does not propagate" scenario, each
  count on a static owned by its test; verify the same command

## 3. ModuleRef (`core/module-ref`)

- [x] 3.1 Add `tests/di/module_ref.rs` with a fixture graph (root importing `A` and `CatsModule`, `A` importing
  `CatsModule`, an unrelated `B` with private `Secret` and `Counter`, a global module) and the location scenario
  (`module_name()` of a `ModuleRef` field in an exported provider); verify `cargo test -p corvis --test di module_ref`
- [x] 3.2 Add the `get`/`get_token` scenarios (module first, application-wide fallback, `NotFound`, `TypeMismatch`),
  `get_token_in_context` and `get_strict` scenarios; verify the same command
- [x] 3.3 Add the `resolve` and `create` scenarios (same singleton, two creations not stored, missing dependency yields
  `UnknownDependencies` naming the module); verify the same command

## 4. Custom providers (`core/custom-providers`)

- [x] 4.1 Add `tests/di/custom_providers.rs` with the `use_existing` scenarios: same `Arc`, chain of aliases, alias of a
  transient provider (pending open question 1), target exported by an import, target not visible (`compile` fails
  with `NotFound`), exported alias; verify `cargo test -p corvis --test di custom_providers`
- [x] 4.2 Add the `use_class_as` scenarios: trait object injected, implementation's own fields injected, concrete type
  yields `TypeMismatch`, runtime selection with `get_token_in_context::<dyn Exporter>` including an unknown format,
  transient implementation gives different instances (compare `id()` through the trait); do not assert lifecycle hooks
  (issue #14); verify the same command

## 5. Cycle detection (`core/circular-dependencies`)

- [x] 5.1 Add `tests/di/cycles.rs` with the provider-cycle scenarios (two singletons, self-dependency, three providers,
  across modules, factories with `DiError::Circular` and not `Factory`, aliases, transient self-dependency), each
  `compile()` wrapped in `tokio::time::timeout` (design.md Decision 5); verify
  `cargo test -p corvis --test di cycles`
- [x] 5.2 Add the module-cycle scenarios (two modules importing each other start and inject each other's exports; a
  re-export cycle with a missing token ends in `UnknownDependencies`); verify the same command

## 6. Global modules (`core/global-modules`)

- [x] 6.1 Add `tests/di/global_modules.rs` with the three declaration forms (`global`, `global = true`, `#[global]`
  below `#[module]`), visibility without import, not in the graph, not exported, and re-export from a global module;
  verify `cargo test -p corvis --test di global_modules`
- [x] 6.2 Add the precedence scenario (own provider shadows the global export, other modules still get the global
  one), the shared-instance scenario, and `DynamicModule::global(true)` / `global(false)` overriding the attribute;
  verify the same command

## 7. User-defined dynamic modules (`core/dynamic-modules`)

- [x] 7.1 Add `tests/di/dynamic_modules.rs` with a `MailerModule` written like the docs example (static part with
  `MailerService`, `for_root(options)` adding the options provider) and the combination scenarios, including
  `.controller(...)` served through `corvis::testing::request` after `init()` and absent when not added; verify
  `cargo test -p corvis --test di dynamic_modules`
- [x] 7.2 Add the privacy scenarios (private options, `.export("MAILER_OPTIONS")`, invalid export naming
  `MailerModule`), the two-importers scenario and the factory-context scenarios (importer's provider not visible,
  `.import(ModuleImport::of::<SettingsModule>())`, global module); verify the same command

## 8. Findings and documentation

- [x] 8.1 For every failing scenario, apply design.md Decision 6 (small local fix with its regression test, or
  `#[ignore = "see #<issue>"]` plus a new issue in epic #13); list each outcome in this file under the task; verify
  `cargo test -p corvis --test di` passes with only documented ignores
- [x] 8.2 Compare the tests with `docs/fundamentals/injection-scopes.md`, `module-ref.md`, `custom-providers.md`,
  `circular-dependency.md`, `dynamic-modules.md` and the "Global modules" and "Encapsulation" sections of
  `docs/modules.md`; correct any statement a test contradicts (and, if open question 3 is accepted, document the
  dynamic-over-static provider rule) in English and in the Spanish pages under `docs/es/`; if nothing changes, record
  that here; verify with `npm run docs:check` in `docs/`

## 9. Definition of done

- [x] 9.1 `cargo build --workspace --all-targets` with no warnings
- [x] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 9.3 `cargo test --workspace` passes
- [x] 9.4 Regression tests: every scenario in the six spec deltas maps to a test in `crates/corvis/tests/di/` (list
  the mapping in the PR description)
- [x] 9.5 Docs: pages updated in `docs/` and `docs/es/` per task 8.2 (or recorded as unchanged); `npm run docs:check`
  in `docs/` passes
- [ ] 9.6 If task 8.1 changed framework runtime code, run the `corvis-parity` check and verify the three port apps keep
  0 apidiff differences; otherwise record that only tests changed (PENDING: task 8.1 changed `di/container.rs` and
  `di/provider.rs` for aliases of transient providers; the parity check was not run by the implementer)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #15, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
