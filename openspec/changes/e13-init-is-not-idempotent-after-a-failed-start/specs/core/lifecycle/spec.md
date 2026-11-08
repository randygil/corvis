# Spec Delta

## ADDED Requirements

### Requirement: init() after a successful init() does nothing
Once `CorvisApplication::init()` has returned `Ok(())`, every later call to `init()`, including the implicit calls made
by `listen()`, `listen_in_background()`, `handle()` and `TestingModule::init()`, SHALL return `Ok(())` without
registering routes, middleware, gateways, global enhancers or handlers again and without running any lifecycle hook
again.

#### Scenario: Two successful calls
- **WHEN** an application with a `Provider::app_guard::<CountingGuard>()` calls `init()` twice and then serves one
  request
- **THEN** both calls return `Ok(())`, `on_module_init` ran once and `CountingGuard::can_activate` ran once

#### Scenario: Bind failure after a successful init
- **WHEN** `init()` succeeded and `listen_in_background(addr)` fails because `addr` is already in use
- **THEN** a later `listen_in_background` on a free address succeeds without running `init()` steps again

### Requirement: init() is single-use after a failure
When `init()` returns an error, the application SHALL record that it failed. Every later call to `init()`, directly or
through `listen()`, `listen_in_background()`, `handle()` or `TestingModule::init()`, MUST return an
`InitFailedError` without running any initialization step: it MUST NOT register global enhancers, event listeners or
scheduled handlers again and MUST NOT run or skip lifecycle hooks. The first failing call SHALL return its original
error unchanged.

`InitFailedError` SHALL be a public type that implements `std::error::Error`, can be obtained with
`anyhow::Error::downcast_ref::<InitFailedError>()`, and whose message states that the application failed to
initialize earlier, includes the message of the first error, and says to create a new application.

#### Scenario: Retry after a failing on_module_init
- **WHEN** a provider's `on_module_init` returns `Err("database unavailable")` on the first `init()` and `init()` is
  called again on the same application
- **THEN** the first call returns the `"database unavailable"` error, the second returns an `InitFailedError` whose
  message contains `"database unavailable"`, and no `on_application_bootstrap` hook has run

#### Scenario: Global enhancers are not registered twice
- **WHEN** an application with `Provider::app_guard::<CountingGuard>()` fails its first `init()` and `init()` is called
  again
- **THEN** the application's global guard list still holds `CountingGuard` exactly once

#### Scenario: Implicit init through handle or listen
- **WHEN** `init()` failed and the caller then uses `handle(request)` or `listen_in_background(addr)`
- **THEN** both return an `InitFailedError` and no socket is bound

### Requirement: A failed application cannot serve requests
After `init()` failed, the application MUST NOT expose a request pipeline: `corvis::testing::request(&app)` SHALL
panic with a message that says `init()` failed, and no request SHALL reach a controller of that application.

#### Scenario: Test client after a failed init
- **WHEN** a test creates `request(&app)` for an application whose `init()` failed
- **THEN** it panics with a message that mentions the failed `init()`
