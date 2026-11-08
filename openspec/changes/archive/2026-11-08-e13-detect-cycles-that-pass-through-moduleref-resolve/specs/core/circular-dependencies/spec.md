# Spec Delta

## ADDED Requirements

### Requirement: Cycles through ModuleRef during construction are detected
When, while a provider is being constructed, its factory or constructor calls `ModuleRef::resolve` or
`ModuleRef::create` (directly or through code it calls on the same task), and that call needs, directly or
indirectly, the provider being constructed, the container MUST return `DiError::Circular` whose chain names every
provider of the cycle and starts and ends with the same provider. The call MUST NOT hang, overflow the stack or call the
factory again.

#### Scenario: Factory resolving its consumer through ModuleRef
- **WHEN** `Provider::use_factory(Token::of::<Orders>(), ..)` builds `Orders` by calling
  `ctx.module_ref().resolve::<Invoices>()`, `Invoices` is an `#[injectable]` with an `Arc<Orders>` field, and the
  testing module is compiled
- **THEN** compilation returns `DiError::Circular` within the test timeout, and the chain contains `Orders` and
  `Invoices` and starts and ends with the same name

#### Scenario: Factory resolving itself through ModuleRef
- **WHEN** the factory of `Loop` calls `ctx.module_ref().resolve::<Loop>()`
- **THEN** compilation returns `DiError::Circular` with the chain `Loop -> Loop` and the factory ran once

#### Scenario: Stored ModuleRef of another provider
- **WHEN** a singleton `Registry` with a `module_ref: ModuleRef` field exposes `async fn find<T>()` that calls
  `self.module_ref.resolve::<T>()`, and the factory of `Orders` calls `registry.find::<Invoices>()` where `Invoices`
  injects `Orders`
- **THEN** compilation returns `DiError::Circular`

#### Scenario: ModuleRef::create during construction
- **WHEN** the factory of `Orders` calls `ctx.module_ref().create::<Report>()` and `Report` has an `Arc<Orders>` field
- **THEN** compilation returns `DiError::Circular`

### Requirement: ModuleRef after construction starts a new resolution
A `ModuleRef` used outside the construction of a provider (after startup, in a handler, a lifecycle hook or a method
call that is not part of a factory) SHALL resolve as a new, independent resolution: it MUST NOT report a cycle because
of the provider that owns the `ModuleRef` or of the construction during which the `ModuleRef` was injected.

#### Scenario: Transient provider resolving a fresh copy of itself
- **WHEN** a transient `Job` with a `module_ref: ModuleRef` field calls `self.module_ref.resolve::<Job>()` after it
  was constructed
- **THEN** the call returns a new `Job` instance

#### Scenario: Resolving a dependent of the caller at run time
- **WHEN** a singleton `Orders` with a `module_ref: ModuleRef` field calls `self.module_ref.resolve::<Invoices>()`
  after `init()`, and `Invoices` is a transient provider with an `Arc<Orders>` field
- **THEN** the call returns an `Invoices` holding the existing `Orders` instance

#### Scenario: Concurrent resolutions
- **WHEN** ten tasks call `module_ref.resolve::<Job>()` at the same time after startup for a transient `Job`
- **THEN** every call succeeds and returns a distinct instance
