# Design

## Context

See proposal.md for motivation; the guarantees are in the `core/di` spec delta. Current state, verified in the code:

- **Circular message.** `DiError::Circular` (`crates/corvis/src/di/error.rs:15-18`) says: "Please, make sure that each
  side of a bidirectional relationship is decorated with \"forwardRef()\" (or refactor the dependency graph)." It is
  raised by `Container::resolve_provider` when the key is already on the resolution path
  (`crates/corvis/src/di/container.rs:343-347`), with the chain built from `Token::display_name`. The docs already
  apologize for it: `docs/fundamentals/circular-dependency.md:31` and `docs/faq/common-errors.md:47-48` (and the
  Spanish pages at the same lines). The remedies those pages list are: extract a third provider, resolve one side with
  `ModuleRef`, communicate with `EventEmitter`.
- **Unknown-dependency message.** `DiError::UnknownDependencies` (`error.rs:7-10`) ends with "If {token} is exported
  from a separate @Module, is that module imported within {module}?" followed by a `#[module(imports = [...])]` hint.
  `crates/corvis/tests/app.rs:381-394` (`missing_dependency_message`) asserts only the first two sentences, which do
  not change.
- **Scope rustdoc.** `crates/corvis/src/di/provider.rs:30` reads "Injection scope: `Default` (singleton), `Request` or
  `Transient`." while the enum (`provider.rs:31-38`) has only `Default` and `Transient`, and the macro rejects
  `scope = "request"` (`crates/corvis-macros/src/injectable.rs:170`). `docs/fundamentals/injection-scopes.md` is
  already correct.
- **`#[optional]`.** `injectable_impl` drains field attributes and matches `"optional" => {}`
  (`crates/corvis-macros/src/injectable.rs:29-43`, the arm at line 35), so the attribute is removed and ignored.
  Optionality comes only from the field type: `Option<Arc<T>>` implements `InjectField` through
  `ResolveCtx::get_optional` (`crates/corvis/src/di/injectable.rs:94-103`, `173-181`), and the `#[inject(token)]`
  path recognizes the same shape (`injectable.rs:56-57` of the macro crate). `injectable_impl` is shared by
  `#[injectable]` (`corvis-macros/src/lib.rs:32`), `#[controller]` (`controller.rs:130`), `#[module]`
  (`module.rs:69`) and `#[websocket_gateway]` (`gateway.rs:108`). The docs document the no-op:
  `docs/providers.md:97-99`. No `#[optional]` use exists in the workspace, the examples or the three port apps.
- **`use_class_as` and hooks.** `Provider::use_class` sets `p.hooks = ErasedHooks::from_typed(T::hooks())`
  (`provider.rs:148`); `use_class_as` (`provider.rs:152-169`) sets only the scope. Adding the same line would not be
  enough: every erased hook downcasts the stored instance to `Arc<T>` (`ErasedHooks::from_typed`, `provider.rs:64-95`)
  and silently returns `Ok(())` / an empty list when the downcast fails, but `use_class_as` stores
  `erase(cast(Arc<T>))`, an `Arc<U>`. The stored instance is read by three consumers of `ErasedHooks`:
  `Container::call_hook` (`container.rs:458-513`, via `Container::instance` at `container.rs:422`),
  `discovery::discover` (`crates/corvis/src/discovery.rs:91-108`, `#[handlers]`) and
  `websockets::build_gateways` (`crates/corvis/src/websockets.rs:550-562`, gateway metadata and WS handlers). The
  provider factory is called in exactly one place, `container.rs:357`. The docs document the limitation:
  `docs/fundamentals/custom-providers.md:137-139`.

## Goals / Non-Goals

**Goals:**
- No DI message or rustdoc names an API Corvis lacks.
- Every accepted field attribute has an effect; the one that has none fails at compile time with a fix.
- `use_class_as` is a drop-in for `use_class` with respect to hooks, handlers and gateways.

**Non-Goals:**
- Adding `forwardRef`, a request scope or a new optional-injection mechanism.
- Reworking DI error types or their wording beyond the two messages named here.

## Decisions

1. **Circular message text.** New text:
   `A circular dependency has been detected: {chain}. Break the cycle: move the shared logic into a third provider,
   resolve one side lazily with ModuleRef (not in its constructor), or communicate through EventEmitter. See
   "Circular dependencies" in the docs.` The chain keeps its current format (`A -> B -> A`), so the first sentence is
   unchanged and existing log searches still match.
   - Alternative: keep the message and only fix the docs (status quo). Rejected: the message is what users read first.
   - Alternative: drop all advice. Rejected: the three remedies are short and actionable.
2. **`@Module` → `#[module]` in `UnknownDependencies`.** One-word change in the same enum, same kind of defect; it
   keeps the asserted sentences intact. Included here rather than in a new issue because it is the same message family
   and touches the same file.
3. **Scope rustdoc.** "Injection scope: `Default` (singleton, the default) or `Transient`. There is no request scope:
   carry per-request data on the `Request` (see the injection scopes guide)." Doc-only.
4. **`#[optional]` becomes a compile error everywhere.** The `"optional"` arm returns
   `syn::Error::new_spanned(&attr, "#[optional] has no effect: declare the field as Option<Arc<T>> to make the
   dependency optional")`.
   - Alternative: give `#[optional] Arc<T>` a meaning. Impossible without a value to put in the field.
   - Alternative: accept it only on `Option<...>` fields as a readability marker. Rejected for now: it would still be
     an attribute with no effect, which is what the issue asks to remove. Listed under Open questions.
   - Alternative: a deprecation warning first. Proc macros on stable cannot emit warnings without tricks
     (`#[deprecated]` shims), and the crate is pre-1.0 with no known users of the attribute.
5. **Hooks of `use_class_as` run on the concrete instance via a "hook target".** The crate-private factory result
   becomes `Built { instance: Instance, hook_target: Option<Instance> }`. Every constructor returns
   `hook_target: None` except `use_class_as`, which returns `Some(erase(arc_t.clone()))` next to
   `instance: erase(cast(arc_t))` (same allocation, two views). `Container::resolve_provider` stores the hook target in
   a new `hook_targets: Mutex<HashMap<(ModuleId, Token), Instance>>` when the instance is cached (singletons only), and
   a new `Container::hook_instance(module, token)` returns the hook target if present, otherwise the instance.
   `call_hook`, `discover` and `build_gateways` switch from `instance()` to `hook_instance()`. `use_class_as` then sets
   `p.hooks = ErasedHooks::from_typed(T::hooks())` like `use_class`.
   - Alternative: let the `use_class_as` factory write the hook target through `ResolveCtx` (the key is the last
     element of `ctx.path`). Rejected: a hidden side channel that depends on path layout.
   - Alternative: require `U: Any` and downcast back. Rejected: forces a supertrait on every user trait.
   - Alternative: wrap the hooks at registration time with a closure that captures the cast. Not possible: the cast
     goes from `T` to `U`, and hooks need `T` from `U`.
   - Alternative: keep documenting the limitation. Rejected: the issue asks to fix it.
6. **Transient behavior unchanged.** Transient instances are not cached, so they have no hook target and, as with
   `use_class` today, no lifecycle hooks run for them.
7. **Testing overrides.** `Scanner::add_provider` replaces a provider by its override (`container.rs:93-109`), so an
   override built with `use_value` has no hook target and no hooks: same behavior as today for every override.

## Risks / Trade-offs

- [Breaking: code with `#[optional]` stops compiling] → the error says exactly what to write; the attribute never had
  an effect; no known user. Mentioned in the release notes of the change.
- [Hooks now run for existing `use_class_as` providers that implement them] → this is the fix, but it changes
  behavior for apps that relied on hooks *not* running. Only `facten-api` uses `use_class_as` among the ports, with a
  type that has no hooks; the parity check confirms it.
- [Double hooks if the same instance were reachable through two tokens] → not possible: `use_class_as` constructs its
  own `T`; `use_existing` aliases carry no hooks (`provider.rs:236-246`).
- [Message texts are asserted by users' tests] → the first sentence of both messages is unchanged.

## Open questions

- Should `#[optional]` be accepted on `Option<...>` fields as a migration-friendly marker instead of always failing?
  This design rejects it everywhere; a maintainer may prefer the softer rule.
- Should the circular message include a docs URL once the site address is final (#5)? This design names the page
  title only.

## Review notes (2026-11-08)

- `#[optional]` is rejected everywhere with a message pointing to `Option<Arc<T>>`; no marker exception (simpler rule, nothing uses it).
- No documentation URL in the circular-dependency message for now; it can be added once the docs URLs are stable.
- The corvis-swagger `@ApiExtraModels` message belongs to #22's inventory; #22 picks it up.

## Implementation notes

- The circular-dependency regression test declares `AService`/`BService` at module level: types declared inside a
  function body get a `{{closure}}` prefix in `std::any::type_name`, which would appear in the chain.
- `Container::construct_injectable` (guards, pipes, interceptors referenced by type) is unchanged: those classes are
  never registered through `use_class_as`, so they need no hook target.
- Task 5.6 (`corvis-parity`) is left unticked: it is run by the parity-verifier agent, not by the implementer.
- The two `cargo doc` warnings (`BACKTRACK_LIMIT`, `ErrorMap` link) predate this change and are unrelated.
