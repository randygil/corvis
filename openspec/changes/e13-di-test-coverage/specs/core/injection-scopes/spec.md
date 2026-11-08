# Spec Delta

## Purpose

How many instances of a provider the container creates and when, for the two supported scopes: the default
(singleton) and transient.

## ADDED Requirements

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
