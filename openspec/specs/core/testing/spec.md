# core/testing Specification

## Purpose
Replacing enhancers in a testing module (`Test::create_testing_module()`), so a test can mock a guard, interceptor,
pipe or filter wherever the application registers it by type.

## Requirements

### Requirement: Enhancer overrides cover global enhancers registered as providers
`override_guard::<G>(mock)` SHALL replace guard `G` both where it is referenced by type in `#[use_guards(G)]` and
where it is registered as a global guard with `Provider::app_guard::<G>()`. The same SHALL hold for
`override_interceptor` / `app_interceptor`, `override_pipe` / `app_pipe` and `override_filter` / `app_filter`. The
mock MUST take the original's place in the global order.

#### Scenario: Mocking a global guard
- **WHEN** the imported module registers `Provider::app_guard::<RolesGuard>()`, `RolesGuard` rejects every request,
  and the testing module calls `.override_guard::<RolesGuard>(AllowAll)`
- **THEN** a request to any route of the module is answered by the handler, and `RolesGuard` is never called

#### Scenario: Mocking a global filter
- **WHEN** the imported module registers `Provider::app_filter::<HttpErrorFilter>()` and the testing module calls
  `.override_filter::<HttpErrorFilter>(MockFilter)`
- **THEN** an exception returned by a handler is answered by `MockFilter`

### Requirement: Overriding the provider of a global enhancer never drops it
When a testing module overrides the provider behind an `app_*` registration, the overriding value SHALL remain
registered as a global enhancer of the same kind. If the overriding value cannot act as that kind of enhancer,
`init()` MUST return an error that names the enhancer, and the application MUST NOT start without it.

#### Scenario: Override with a compatible value
- **WHEN** the provider behind `Provider::app_guard::<JwtGuard>()` is overridden with another `JwtGuard` value
- **THEN** the overriding value runs as a global guard on every route

#### Scenario: Override with an incompatible value
- **WHEN** the provider behind `Provider::app_guard::<JwtGuard>()` is overridden with a value that is not a guard
- **THEN** `init()` fails with an error naming `JwtGuard`, instead of starting without the guard
