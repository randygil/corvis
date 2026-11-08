# core/injection-scopes Specification

## Purpose
How many instances of a provider the container creates and when, for the two supported scopes: the default
(singleton) and transient.

## Requirements

### Requirement: Singleton providers are shared within their module
A provider with the default scope SHALL be instantiated once per module that registers it, when the application
starts, and every consumer that resolves it from that module MUST receive the same instance.

#### Scenario: Two consumers of a singleton
- **WHEN** two providers of a module each declare an `Arc<Counter>` field and `Counter` has the default scope
- **THEN** both fields point to the same instance (`Arc::ptr_eq`), and `app.get::<Counter>()` returns that instance

### Requirement: Transient providers give each consumer its own instance
A provider with the transient scope SHALL be instantiated every time it is resolved: each consumer that injects it MUST
receive a distinct instance, and each `ModuleRef::resolve` call MUST return a new one. The transient scope SHALL be
honored whether it is declared with `#[injectable(scope = "transient")]` or with `Provider::scope(Scope::Transient)`.

#### Scenario: Two consumers of a transient provider
- **WHEN** two singleton providers each declare an `Arc<OperationTimer>` field and `OperationTimer` is declared with
  `#[injectable(scope = "transient")]`
- **THEN** the two fields hold different instances

#### Scenario: Transient scope set on the provider
- **WHEN** a module registers `Provider::use_class::<OperationTimer>("TIMER").scope(Scope::Transient)` for a class
  without a scope attribute, and two providers inject `#[inject("TIMER")] Arc<OperationTimer>`
- **THEN** the two fields hold different instances

#### Scenario: Two fields of the same transient type
- **WHEN** one singleton declares two `Arc<OperationTimer>` fields and `OperationTimer` is transient
- **THEN** the two fields hold different instances

#### Scenario: Resolving a transient provider on demand
- **WHEN** `module_ref.resolve::<OperationTimer>().await` is called twice
- **THEN** both calls succeed and return different instances

### Requirement: Transient instances are neither created at startup nor stored
The container MUST NOT instantiate a transient provider at startup unless a singleton consumer injects it, and it MUST
NOT store transient instances. Consequently, `ModuleRef::get` and `app.get` MUST return `DiError::NotFound` for a
transient provider, and lifecycle hooks MUST NOT be called on transient instances.

#### Scenario: No consumer at startup
- **WHEN** an application registers a transient provider that no singleton injects and starts
- **THEN** the provider's constructor has not run

#### Scenario: Looking up a transient instance
- **WHEN** `app.get::<OperationTimer>()` is called for a transient `OperationTimer`
- **THEN** it returns `Err(DiError::NotFound { .. })`

#### Scenario: Lifecycle hooks
- **WHEN** a transient provider implements `OnModuleInit`, a singleton injects it, and the application runs `init()`
- **THEN** the transient provider's `on_module_init` is not called

### Requirement: Scope does not propagate to consumers
A singleton that depends on a transient provider SHALL receive one transient instance when the singleton is created
and MUST keep it for its whole life; the singleton MUST NOT become transient.

#### Scenario: Singleton consumer resolved twice
- **WHEN** a singleton `Report` injects a transient `OperationTimer` and `Report` is obtained twice (`get` and
  `resolve`)
- **THEN** both calls return the same `Report`, holding the same `OperationTimer` instance

### Requirement: Transient providers' dependencies are validated at startup
When an application or testing module is created, the container MUST check the declared dependencies of every
transient provider before it instantiates any provider, and MUST NOT instantiate transient providers to do so. A
required dependency that is not visible from the provider's module MUST make creation fail with the same `DiError` its
first resolution would return: `UnknownDependencies` (provider, token, index, module) for an injected field, `NotFound`
for an alias target.

#### Scenario: Missing dependency of an unused transient provider
- **WHEN** a module registers a transient `OperationTimer` with an `Arc<Clock>` field, no module provides `Clock`, no
  provider injects `OperationTimer`, and the testing module is compiled
- **THEN** compilation fails with `DiError::UnknownDependencies` whose message names `OperationTimer`, `Clock`, index
  `[0]` and the module

#### Scenario: Not instantiated by the check
- **WHEN** a transient provider with all its dependencies available counts its constructions and no singleton injects
  it
- **THEN** compilation succeeds and the counter is still zero

#### Scenario: Alias with a missing target
- **WHEN** a module registers `Provider::use_existing("TIMER", "MISSING").scope(Scope::Transient)`
- **THEN** compilation fails with `DiError::NotFound` naming `"MISSING"`

### Requirement: The startup check follows transient dependencies only where it can know them
Dependencies that are transient providers MUST be checked in their own module, and a cycle made only of transient
providers MUST fail with `DiError::Circular`. Absent optional dependencies (`Option<Arc<T>>`) MUST NOT fail the check.
Dependencies that can only be known by running code (factory providers) SHALL NOT be checked.

#### Scenario: Transitive transient dependency
- **WHEN** transient `Report` injects transient `OperationTimer`, and `OperationTimer`'s `Arc<Clock>` dependency is
  missing
- **THEN** compilation fails with `DiError::UnknownDependencies` naming `OperationTimer` and `Clock`

#### Scenario: Optional dependency absent
- **WHEN** a transient provider has an `Option<Arc<Clock>>` field and no module provides `Clock`
- **THEN** compilation succeeds

#### Scenario: Cycle of transient providers
- **WHEN** transient `A` injects transient `B`, `B` injects `A`, and no singleton injects either
- **THEN** compilation fails with `DiError::Circular` whose chain contains `A` and `B`

#### Scenario: Transient factory provider
- **WHEN** a module registers `Provider::use_factory("CLIENT", ..).scope(Scope::Transient)` whose factory asks for a
  missing token
- **THEN** compilation succeeds and the error appears when `"CLIENT"` is first resolved, as before

#### Scenario: Transient provider replaced by a testing override
- **WHEN** a transient `OperationTimer` whose `Clock` dependency is missing is replaced in the testing module with
  `override_provider::<OperationTimer>().use_value(..)`
- **THEN** compilation succeeds, because only the providers actually registered after overrides are checked
