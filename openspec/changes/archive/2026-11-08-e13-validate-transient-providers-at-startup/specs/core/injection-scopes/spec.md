# Spec Delta

## ADDED Requirements

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
