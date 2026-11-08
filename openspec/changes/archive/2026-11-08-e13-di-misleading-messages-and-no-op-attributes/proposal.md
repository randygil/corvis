# Proposal

Implements [randygil/corvis#14](https://github.com/randygil/corvis/issues/14) (epic [#13](https://github.com/randygil/corvis/issues/13)).

## Why

The docs review found four places where the dependency injection container says one thing and does another. The
documentation currently works around each of them with a warning box, which is the wrong fix: a user who reads only the
error message or the rustdoc is misled.

- The circular dependency error tells the user to decorate both sides with `forwardRef()`, a function Corvis does not
  have (`crates/corvis/src/di/error.rs:16`).
- The rustdoc of `Scope` lists a `Request` scope that does not exist (`crates/corvis/src/di/provider.rs:30`).
- `#[optional]` on an `#[injectable]` field is accepted and silently ignored
  (`crates/corvis-macros/src/injectable.rs:35`), so `#[optional] dep: Arc<T>` still fails at startup when `T` is
  missing.
- `Provider::use_class_as` never registers the hooks of `T` (`crates/corvis/src/di/provider.rs:152-169`), so
  `on_module_init`, the shutdown hooks and `#[handlers]` (`#[on_event]`, `#[cron]`...) of a class registered behind a
  trait never run.

## What Changes

- The circular dependency message names the remedies Corvis actually has (extract a third provider, resolve one side
  lazily with `ModuleRef`, communicate through `EventEmitter`) and no longer mentions `forwardRef()`. The same pass
  replaces the decorator spelling `@Module` in the unknown-dependency message with `#[module]`.
- The `Scope` rustdoc lists only `Default` and `Transient` and says there is no request scope.
- `#[optional]` on a field becomes a compile error that tells the user to declare the field as `Option<Arc<T>>`.
- `Provider::use_class_as` keeps every hook of `T`: the five lifecycle hooks, `#[handlers]` discovery and gateway
  metadata run against the concrete instance, exactly as with `Provider::use_class`.
- Docs (EN + ES) drop the warning boxes that described these defects and show the new messages.

## Capabilities

### New Capabilities

- `core/di`: dependency injection error messages, scopes, field attributes and custom providers behave as their
  messages and docs say.

### Modified Capabilities

None (no `core/*` spec exists yet).

## Impact

- `crates/corvis/src/di/error.rs`: two message texts (`Circular`, `UnknownDependencies`).
- `crates/corvis/src/di/provider.rs`: `Scope` rustdoc; `use_class_as` keeps hooks; the crate-private `Factory` result
  gains an optional hook target (no public API change).
- `crates/corvis/src/di/container.rs`, `crates/corvis/src/discovery.rs`, `crates/corvis/src/websockets.rs`: lifecycle
  hooks, handler discovery and gateway discovery use the hook target when a provider has one.
- `crates/corvis-macros/src/injectable.rs`: `#[optional]` is rejected. Breaking for code that wrote it, but it never
  had an effect, and no crate in the workspace, the examples or the three port apps uses it.
- Docs: `docs/fundamentals/circular-dependency.md`, `docs/faq/common-errors.md`, `docs/providers.md`,
  `docs/fundamentals/custom-providers.md` and their `docs/es/` translations.
- Port apps: `facten-api` registers `HttpImprentaProvider` with `use_class_as`
  (`facten-api/src/emissions/emissions.module.rs:44`); that type implements no hooks, so its behavior does not change.
  Startup messages are not part of the apidiff suites.

## Out of scope

- A deferred-reference mechanism (`forwardRef`-like) for provider cycles: a feature, not a message fix; candidate for
  the #23 spike.
- A request scope: explicitly unsupported (`docs/fundamentals/injection-scopes.md`); unchanged.
- The circular schema message of corvis-swagger (`crates/corvis-swagger/src/schema_factory.rs:662`), which mentions
  `@ApiExtraModels` and `type: () => ClassType`: the same kind of defect in another crate; candidate for #22 or a new
  issue.
- Broader DI test coverage (scopes, dynamic modules, `ModuleRef`): issue #15.
