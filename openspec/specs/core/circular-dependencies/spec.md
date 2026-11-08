# core/circular-dependencies Specification

## Purpose
Cycles between providers cannot be built and must be reported clearly; cycles between module imports are harmless and
must be accepted.

## Requirements

### Requirement: Provider cycles are detected and reported with the full chain
When resolving a provider requires, directly or indirectly, resolving that same provider in the same module, the
container MUST stop and return `DiError::Circular`, whose `chain` names every provider of the cycle in resolution order
and starts and ends with the same provider. Detection MUST apply to field injection, factory providers (`ResolveCtx`
lookups), aliases (`use_existing`) and transient providers, and across module boundaries. A cycle MUST NOT hang,
overflow the stack or panic.

#### Scenario: Two singletons
- **WHEN** `OrdersService` has an `Arc<InvoicesService>` field, `InvoicesService` has an `Arc<OrdersService>` field, and
  the testing module is compiled
- **THEN** compilation returns an error whose `DiError` is `Circular`, and its chain is `OrdersService ->
  InvoicesService -> OrdersService` or `InvoicesService -> OrdersService -> InvoicesService`

#### Scenario: Self-dependency
- **WHEN** a provider `Loop` has an `Arc<Loop>` field
- **THEN** compilation fails with `DiError::Circular` and the chain `Loop -> Loop`

#### Scenario: Three providers
- **WHEN** `A` injects `B`, `B` injects `C` and `C` injects `A`
- **THEN** compilation fails with `DiError::Circular` whose chain contains `A`, `B` and `C`

#### Scenario: Across modules
- **WHEN** `OrdersModule` and `BillingModule` import each other, each exports one service, and each service injects the
  other module's service
- **THEN** compilation fails with `DiError::Circular`

#### Scenario: Factories
- **WHEN** factory `"X"` calls `ctx.get_token("Y")` and factory `"Y"` calls `ctx.get_token("X")`
- **THEN** compilation fails with `DiError::Circular` (not `DiError::Factory`)

#### Scenario: Aliases
- **WHEN** `Provider::use_existing("A", "B")` and `Provider::use_existing("B", "A")` are registered
- **THEN** compilation fails with `DiError::Circular`

#### Scenario: Transient provider depending on itself
- **WHEN** a transient provider `Node` has an `Arc<Node>` field and a singleton injects `Node`
- **THEN** compilation fails with `DiError::Circular`, and the chain ends with `Node -> Node` (it may start at
  the singleton whose resolution entered the cycle)

### Requirement: Module import cycles are accepted
Static modules that import each other, directly or through other modules, SHALL be scanned once each and the
application MUST start when no provider cycle exists. Export lookups through cyclic imports and re-exports MUST
terminate.

#### Scenario: Two modules importing each other
- **WHEN** `OrdersModule` imports `BillingModule` and `BillingModule` imports `OrdersModule`, each exports one service,
  and a third provider in each module injects the other module's export
- **THEN** the application starts and both providers receive the exported instances

#### Scenario: Re-export cycle with a missing token
- **WHEN** `A` imports and re-exports `B`, `B` imports and re-exports `A`, and a provider of a module importing `A`
  injects a type no module provides
- **THEN** compilation terminates with `DiError::UnknownDependencies` instead of looping

### Requirement: Import cycles between dynamic modules are reported
When scanning would import the same module class from the same importing class through a dynamic import a second time
on one import chain, the container MUST stop and return `DiError::CircularImport`. Its `chain` MUST name the modules of
the cycle in import order, starting and ending with the same module. Scanning MUST NOT hang, overflow the stack or
panic for any module graph.

#### Scenario: Two dynamic modules importing each other
- **WHEN** `AModule`'s `#[module]` imports `BModule::for_root()`, `BModule`'s imports `AModule::for_root()`, and a
  testing module importing `AModule` is compiled
- **THEN** compilation returns an error whose `DiError` is `CircularImport` with the chain
  `AModule -> BModule -> AModule`

#### Scenario: Three dynamic modules in a cycle
- **WHEN** `AModule` imports `BModule::for_root()`, `BModule` imports `CModule::for_root()` and `CModule` imports
  `AModule::for_root()`
- **THEN** compilation fails with `DiError::CircularImport` whose chain is `AModule -> BModule -> CModule -> AModule`

#### Scenario: Root imported in its dynamic form
- **WHEN** the root imports `AModule::for_root()` and the two modules import each other as in the first scenario
- **THEN** compilation fails with `DiError::CircularImport` and the process does not abort

### Requirement: The dynamic import cycle message explains the fix
The `DiError::CircularImport` message MUST contain the chain, say that each dynamic import creates a new module
instance, and say how to break the cycle.

#### Scenario: Message
- **WHEN** compilation fails with `DiError::CircularImport`
- **THEN** the error message contains the chain and the words "dynamic import", and does not mention `forwardRef`

### Requirement: Module graphs that terminate keep starting
Detecting dynamic import cycles MUST NOT reject a module graph that the scanner could finish building: static import
cycles, a dynamic import of a class that is already being filled when it is reached through a different edge, and
repeated dynamic imports of the same module in different branches SHALL still start as before.

#### Scenario: Static modules importing each other
- **WHEN** `OrdersModule` imports `BillingModule` and `BillingModule` imports `OrdersModule` (both static)
- **THEN** the application starts

#### Scenario: Dynamic module reached again through a static module
- **WHEN** the root imports `AModule::for_root()`, `AModule`'s `#[module]` imports the static `BModule`, and
  `BModule`'s imports `AModule::for_root()`
- **THEN** the application starts, with two `AModule` nodes as before this change

#### Scenario: Same dynamic module in two branches
- **WHEN** `UsersModule` and `BillingModule` each import `MailerModule::for_root(..)`
- **THEN** the application starts with two `MailerModule` instances

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
