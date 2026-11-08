# Spec Delta

## Purpose

The dependency injection container never tells the user to use something Corvis does not have, and every attribute or
provider constructor it accepts has the effect its name and documentation promise.

## ADDED Requirements

### Requirement: Circular dependency errors suggest only remedies that exist
When provider resolution detects a cycle, the error MUST include the full chain of tokens and MUST suggest only
remedies available in Corvis: moving the shared logic to a third provider, resolving one side lazily with `ModuleRef`,
or communicating through `EventEmitter`. The message MUST NOT mention `forwardRef` or any other API that Corvis does not
provide.

#### Scenario: Two providers that depend on each other
- **WHEN** `AService` has an `Arc<BService>` field, `BService` has an `Arc<AService>` field, and a module registering
  both is compiled with `Test::create_testing_module()`
- **THEN** compilation fails with a message that contains
  `A circular dependency has been detected: AService -> BService -> AService`, mentions `ModuleRef`, and does not
  contain `forwardRef`

### Requirement: Unknown dependency errors use Corvis syntax
The error for a dependency that is not visible in a module MUST refer to modules with the `#[module]` attribute syntax
and MUST NOT use decorator syntax such as `@Module`.

#### Scenario: Missing provider
- **WHEN** `NeedsCats` has an `Arc<CatsService>` field and its module neither registers nor imports `CatsService`
- **THEN** the error still names `NeedsCats (?)`, `CatsService at index [0]` and the module, contains `#[module(`, and
  does not contain `@Module`

### Requirement: Scope documentation lists only the supported scopes
The public documentation of `Scope` SHALL describe exactly the variants the type has (`Default` and `Transient`) and
SHALL state that there is no request scope.

#### Scenario: Reading the rustdoc of Scope
- **WHEN** a user reads the rustdoc of `corvis::Scope`
- **THEN** it describes `Default` (singleton) and `Transient` and does not present `Request` as an available scope

### Requirement: Field attributes without an effect are rejected
The field attributes accepted by `#[injectable]` (and by `#[controller]`, `#[module]` and `#[websocket_gateway]`,
which share its code generation) MUST each have an effect. `#[optional]` on a field MUST be a compile error whose
message tells the user to declare the field as `Option<Arc<T>>`. A field of type `Option<Arc<T>>` without any
attribute MUST keep resolving to `None` when `T` is not visible.

#### Scenario: #[optional] on a field
- **WHEN** a struct annotated with `#[injectable]` has a field `#[optional] cats: Arc<CatsService>`
- **THEN** the macro fails with an error spanned on the attribute that mentions `Option<Arc<T>>`

#### Scenario: Optional dependency by type
- **WHEN** an `#[injectable]` struct has a field `cats: Option<Arc<CatsService>>` and no visible module provides
  `CatsService`
- **THEN** the application starts and the field is `None`

### Requirement: Providers registered behind a trait keep their hooks
A provider built with `Provider::use_class_as::<T, U>(token, cast)` SHALL run the same hooks as one built with
`Provider::use_class::<T>(token)`: the five lifecycle hooks, its `#[handlers]` and its gateway metadata. Each hook MUST
run exactly once per singleton, on the instance consumers receive as `Arc<U>`.

#### Scenario: Lifecycle hooks of a class behind a trait
- **WHEN** `MemoryRepo` implements `OnModuleInit` and `OnApplicationShutdown`, is registered with
  `Provider::use_class_as::<MemoryRepo, dyn Repo>("REPO", |r| r)`, and the application is initialized and closed
- **THEN** each hook runs once, and the state set by `on_module_init` is visible through the `Arc<dyn Repo>` resolved
  from `"REPO"`

#### Scenario: Event handlers of a class behind a trait
- **WHEN** a class with an `#[on_event("x")]` handler is registered with `use_class_as`, `EventEmitterModule` is
  imported, the application is initialized and `"x"` is emitted
- **THEN** the handler runs once

#### Scenario: Transient providers behind a trait
- **WHEN** `T` is declared with `#[injectable(scope = "transient")]` and registered with `use_class_as`
- **THEN** each consumer receives its own instance and, as with `use_class`, no lifecycle hook runs for transient
  instances
