# Spec Delta

## ADDED Requirements

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
