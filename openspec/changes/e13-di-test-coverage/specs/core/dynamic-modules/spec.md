# Spec Delta

## Purpose

How a user-defined `DynamicModule` (the `for_root` / `register` pattern) combines with its static `#[module]`
definition and where its providers are visible.

## ADDED Requirements

### Requirement: Static and dynamic parts are combined
A `DynamicModule::new::<M>()` import SHALL contain everything `M`'s `#[module]` declares plus what the `DynamicModule`
adds with `.provider(s)`, `.import(s)`, `.controller(...)` and `.export(s)`. Providers of both parts SHALL see each
other as providers of the same module.

#### Scenario: Options provided by the dynamic part
- **WHEN** `MailerModule` statically registers and exports `MailerService`, which injects
  `#[inject("MAILER_OPTIONS")] Arc<MailerOptions>`, and `MailerModule::for_root(options)` adds
  `Provider::use_value("MAILER_OPTIONS", options)`
- **THEN** a module importing `MailerModule::for_root(..)` can inject `Arc<MailerService>`, whose options are the ones
  passed to `for_root`

#### Scenario: Controller added by the dynamic part
- **WHEN** `for_root` adds `.controller(ControllerDef::of::<MailerDebugController>())`
- **THEN** the application serves that controller's routes, and a `for_root` that does not add it does not serve them

### Requirement: Providers added by the dynamic part are private unless exported
A provider added with `.provider(...)` SHALL be private to the dynamic module unless the static part or `.export(...)`
exports its token. An export of a token the module neither provides nor re-exports MUST make startup fail with
`DiError::InvalidExport`.

#### Scenario: Private options
- **WHEN** a module importing `MailerModule::for_root(..)` injects `#[inject("MAILER_OPTIONS")] Arc<MailerOptions>`
- **THEN** compilation fails with `DiError::UnknownDependencies`

#### Scenario: Exported by the dynamic part
- **WHEN** `for_root` also calls `.export("MAILER_OPTIONS")`
- **THEN** the importing module can inject the options

#### Scenario: Invalid export
- **WHEN** `for_root` calls `.export("MISSING")` and no provider or import supplies `"MISSING"`
- **THEN** compilation fails with `DiError::InvalidExport` naming `MailerModule`

### Requirement: Dynamic providers override static ones with the same token
When the static part and the `DynamicModule` register a provider with the same token, the one added by the
`DynamicModule` SHALL be used.

#### Scenario: Default replaced by for_root
- **WHEN** `#[module]` registers `Provider::use_value("GREETING", "static")` and the import adds
  `.provider(Provider::use_value("GREETING", "dynamic"))`
- **THEN** a provider of that module injecting `"GREETING"` receives `"dynamic"`, and without the dynamic provider
  it receives `"static"`

### Requirement: Each dynamic import is a separate module instance
Every `DynamicModule` value that appears in an `imports` list SHALL create its own module node with its own provider
instances.

#### Scenario: Two importers with different options
- **WHEN** `UsersModule` imports `MailerModule::for_root(a)` and `BillingModule` imports `MailerModule::for_root(b)`
- **THEN** the two `MailerService` instances are different and each sees its own options

### Requirement: Factories of a dynamic module resolve in its context
A factory provider added by a `DynamicModule` SHALL resolve its dependencies from the dynamic module, not from the
importing module: it sees the dynamic module's providers, the exports of its imports (including those added with
`.import(...)`) and global modules.

#### Scenario: Dependency of the importer
- **WHEN** `for_root_async(factory)` registers a factory that calls `ctx.get::<Settings>()`, and `Settings` is a
  non-global provider of the importing module only
- **THEN** compilation fails with `DiError::NotFound`

#### Scenario: Dependency imported by the dynamic part
- **WHEN** the same factory is registered with `.import(ModuleImport::of::<SettingsModule>())`, and `SettingsModule`
  exports `Settings`
- **THEN** the application starts and the factory receives `SettingsModule`'s `Settings`

#### Scenario: Dependency from a global module
- **WHEN** `Settings` is exported by a global module imported by the root
- **THEN** the application starts without adding imports to the dynamic module
