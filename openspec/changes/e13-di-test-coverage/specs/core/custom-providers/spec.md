# Spec Delta

## Purpose

Aliases (`Provider::use_existing`) and implementations exposed as trait objects (`Provider::use_class_as`).

## ADDED Requirements

### Requirement: An alias hands over the same instance as its target
`Provider::use_existing(alias, existing)` SHALL register `alias` so that resolving it returns the very instance that
resolving `existing` returns. Aliases MAY point to other aliases, and an alias of a transient provider is transient.

#### Scenario: Alias of a class provider
- **WHEN** a module registers `CatsService` and `Provider::use_existing("CATS", Token::of::<CatsService>())`, and a
  provider injects `#[inject("CATS")] Arc<CatsService>`
- **THEN** that field and `app.get::<CatsService>()` are the same `Arc` (`Arc::ptr_eq`)

#### Scenario: Chain of aliases
- **WHEN** `"B"` is an alias of `CatsService` and `"A"` is an alias of `"B"`
- **THEN** resolving `"A"` returns the same instance as resolving `CatsService`

#### Scenario: Alias of a transient provider
- **WHEN** `"TIMER"` is an alias of a transient `OperationTimer` and two providers inject `"TIMER"`
- **THEN** each receives its own instance (an alias takes the scope of its target), and so do two providers
  injecting `OperationTimer` directly

### Requirement: An alias resolves its target with its module's visibility
The `existing` token of an alias SHALL be looked up from the module that registers the alias, with the usual
encapsulation rules. When the target is not visible, the application MUST fail to start with `DiError::NotFound`. An
alias MAY be exported like any provider.

#### Scenario: Target exported by an imported module
- **WHEN** module `A` imports `CatsModule`, which exports `CatsService`, and `A` registers
  `Provider::use_existing("CATS", Token::of::<CatsService>())`
- **THEN** resolving `"CATS"` in `A` returns `CatsModule`'s `CatsService` instance

#### Scenario: Target not visible
- **WHEN** module `A` registers an alias whose target is a private provider of a module `A` does not import
- **THEN** compiling the testing module fails with `DiError::NotFound` naming the target token

#### Scenario: Exported alias
- **WHEN** `CatsModule` registers the alias `"CATS"` and declares `exports = ["CATS"]`, and `AppModule` imports it
- **THEN** a provider of `AppModule` can inject `#[inject("CATS")] Arc<CatsService>`

### Requirement: use_class_as exposes a class as a trait object
`Provider::use_class_as::<T, dyn Trait>(token, cast)` SHALL construct `T` with its `#[injectable]` constructor,
injecting `T`'s fields from the registering module, and register the result under `token` as `Arc<dyn Trait>`.
Requesting the concrete type `Arc<T>` under that token MUST fail with `DiError::TypeMismatch`.

#### Scenario: Injecting the trait object
- **WHEN** a module registers `Provider::use_class_as::<MemoryUserRepo, dyn UserRepo>("USER_REPO", |r| r)` and
  `UsersService` injects `#[inject("USER_REPO")] Arc<dyn UserRepo>`
- **THEN** `UsersService` calls `MemoryUserRepo`'s implementation through the trait

#### Scenario: Dependencies of the implementation
- **WHEN** `MemoryUserRepo` declares an `Arc<Seed>` field and `Seed` is a provider of the registering module
- **THEN** the application starts and the repository uses that `Seed` instance

#### Scenario: Asking for the concrete type
- **WHEN** `module_ref.get_token::<MemoryUserRepo>("USER_REPO")` is called
- **THEN** it returns `Err(DiError::TypeMismatch { .. })`

#### Scenario: Choosing the implementation at runtime
- **WHEN** `"EXPORTER:csv"` and `"EXPORTER:json"` are registered with `use_class_as` for two classes implementing
  `dyn Exporter`, and a service calls `get_token_in_context::<dyn Exporter>(format!("EXPORTER:{format}"))`
- **THEN** each format returns its own implementation and an unknown format returns `Err(DiError::NotFound { .. })`

### Requirement: use_class_as inherits the class's scope
A provider built with `use_class_as` SHALL take the scope declared by `T`'s `#[injectable]` attribute, and
`Provider::scope(...)` SHALL override it.

#### Scenario: Transient implementation
- **WHEN** `T` is declared with `#[injectable(scope = "transient")]`, registered with `use_class_as`, and two providers
  inject its token
- **THEN** they receive different instances
