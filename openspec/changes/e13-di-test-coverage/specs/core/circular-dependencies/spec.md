# Spec Delta

## Purpose

Cycles between providers cannot be built and must be reported clearly; cycles between module imports are harmless and
must be accepted.

## ADDED Requirements

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
- **THEN** compilation fails with `DiError::Circular`

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
