# Spec Delta

## Purpose

A global module makes its exports visible to every module of the application without importing it.

## ADDED Requirements

### Requirement: Global modules can be declared in three equivalent forms
`#[module(global, ...)]`, `#[module(global = true, ...)]` and a `#[global]` attribute placed below `#[module(...)]`
SHALL each declare a global module with the same behavior.

#### Scenario: Each form
- **WHEN** three global modules are declared, one with each form, each exporting one provider, and the root module
  imports all three
- **THEN** a provider of a sibling module that imports none of them can inject all three exports

### Requirement: Exports of a global module are visible everywhere once it is in the graph
The exports of a global module SHALL be visible from every module of the application, provided some module imports it.
A global module that no module imports MUST NOT contribute providers. Only exported providers are shared: a provider of
a global module that is not exported MUST NOT be visible outside it. Re-exported modules of a global module SHALL be
visible too.

#### Scenario: Visible without import
- **WHEN** `AppModule` imports the global `AuditModule` and `ChatModule`, and a provider of `ChatModule` injects
  `Arc<AuditService>` exported by `AuditModule`
- **THEN** the application starts and the provider receives `AuditModule`'s instance

#### Scenario: Not in the graph
- **WHEN** no module imports the global `AuditModule` and a provider injects `Arc<AuditService>`
- **THEN** compilation fails with `DiError::UnknownDependencies`

#### Scenario: Not exported
- **WHEN** the global `AuditModule` registers `AuditStore` without exporting it and a provider of another module injects
  `Arc<AuditStore>`
- **THEN** compilation fails with `DiError::UnknownDependencies`

#### Scenario: Re-export from a global module
- **WHEN** the global `CoreModule` imports and re-exports `ClockModule`, which exports `Clock`
- **THEN** a provider of a module that imports neither can inject `Arc<Clock>`

### Requirement: Lookup precedence over global modules
A module's own providers SHALL take precedence over the exports of its imports, and those SHALL take precedence over
the exports of global modules.

#### Scenario: Own provider shadows the global export
- **WHEN** the global `AuditModule` exports `AuditService` and `ChatModule` registers its own `AuditService`
- **THEN** providers of `ChatModule` receive `ChatModule`'s instance and providers of other modules receive
  `AuditModule`'s instance

### Requirement: Global modules are shared instances
A static global module SHALL be a single node in the graph, so all modules MUST receive the same instances of its
exports.

#### Scenario: Two consumers in different modules
- **WHEN** providers of two modules that do not import the global `AuditModule` inject `Arc<AuditService>`
- **THEN** both receive the same instance

### Requirement: DynamicModule::global overrides the attribute
`DynamicModule::global(true)` SHALL make that imported instance of the module global even if its `#[module]` is not,
and `DynamicModule::global(false)` SHALL make it non-global even if its `#[module]` is global. When `global` is not
called, the attribute's value SHALL apply.

#### Scenario: Making a module global at import time
- **WHEN** a non-global `MailerModule` is imported by the root as `DynamicModule::new::<MailerModule>().global(true)`
- **THEN** a provider of a sibling module can inject `MailerModule`'s export

#### Scenario: Making a global module non-global at import time
- **WHEN** a `#[module(global)]` module is imported by the root as `DynamicModule::new::<M>().global(false)`
- **THEN** a provider of a sibling module that injects `M`'s export makes compilation fail with
  `DiError::UnknownDependencies`
