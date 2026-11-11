# Spec Delta

## ADDED Requirements

### Requirement: Shutdown runs every hook even when one fails
`close()` SHALL run every `on_module_destroy`, `before_application_shutdown` and `on_application_shutdown` hook and stop
the scheduled jobs even when a hook returns an error or panics. Each failure MUST be logged when it happens, and
`close()` MUST then return a `ShutdownError` whose `failures()` lists every failure in the order it happened, naming the
phase and the provider. A later `close()` SHALL return `Ok(())` and run nothing.

#### Scenario: A failing destroy hook does not skip the rest
- **WHEN** the second of three providers returns `Err("flush failed")` from `on_module_destroy` and the application has
  an `#[interval]` handler
- **THEN** the third provider's `on_module_destroy`, every `before_application_shutdown` and every
  `on_application_shutdown` run, the interval no longer ticks, and `close()` returns a `ShutdownError` with one failure
  naming that provider and `"flush failed"`

#### Scenario: A panicking hook is reported like an error
- **WHEN** a provider's `on_application_shutdown` panics with `"boom"`
- **THEN** the other shutdown hooks run and `close()` returns a `ShutdownError` whose failure carries `"boom"`

#### Scenario: Failures in two phases
- **WHEN** one `on_module_destroy` and one `before_application_shutdown` fail
- **THEN** `failures()` has two entries, the destroy failure first

### Requirement: Shutdown hooks run from the root module to the deepest
The three shutdown phases SHALL walk the modules in the reverse of the init order: the root module first and the most
deeply imported module last, as the original framework does. Within a module the order SHALL stay controllers, then
providers, then the module class. The init phases keep their order (deepest module first).

#### Scenario: Order recorded from the original
- **WHEN** the root module imports `A`, which imports `B`, and every class of the three modules implements every hook
- **THEN** the sequence of hooks for `init()` and `close()` equals the one recorded from the original framework in
  `tests/fixtures/lifecycle/lifecycle.json`

### Requirement: close() stops the servers started in the background
`close()` SHALL stop every server the application started with `listen_in_background` (and its TLS variant) before the
first destroy hook runs: the server stops accepting connections and finishes the requests in flight. No request SHALL
reach a provider after its `on_module_destroy` started.

#### Scenario: No new connection after close
- **WHEN** an application listens in the background on `127.0.0.1:0` and `close()` returns
- **THEN** a new TCP connection to that address is refused

#### Scenario: A request in flight completes
- **WHEN** a request is being handled when `close()` is called
- **THEN** it receives its response before any `on_module_destroy` hook runs

### Requirement: An application dropped without close() stops its background work
When a `CorvisApplication` that was not closed is dropped, its scheduled timeouts, intervals and cron jobs SHALL stop and
its background servers SHALL stop accepting connections. Dropping MUST NOT run lifecycle hooks.

#### Scenario: Dropped after init
- **WHEN** an initialized application with an `#[interval]` handler is dropped without `close()`
- **THEN** the interval does not tick again and no `on_module_destroy` hook ran

#### Scenario: listen fails to bind
- **WHEN** `listen(addr)` initializes the application and then fails because `addr` is in use
- **THEN** the scheduled jobs started by `init()` are stopped when `listen` returns the error

### Requirement: A failed start stops the scheduled jobs it started
When `init()` or `create_application_context` fails after the scheduler was mounted (for example in
`on_application_bootstrap`), the scheduled jobs SHALL be stopped before the error is returned. When mounting the
scheduler fails half way, the jobs it already started SHALL be stopped.

#### Scenario: Bootstrap hook fails
- **WHEN** `on_application_bootstrap` returns an error in an application with an `#[interval]` handler
- **THEN** `init()` returns that error and the interval never ticks afterwards

#### Scenario: An invalid cron expression after a valid one
- **WHEN** the scheduler mounts a valid cron job and then fails on an invalid cron expression
- **THEN** the valid job is not active

### Requirement: listen() refuses ports out of range
A port given to `listen`, `listen_in_background` or their TLS variants as an `i32` SHALL be refused with an
`InvalidInput` error, binding nothing, when it is outside `0..=65535`.

#### Scenario: Port 70000
- **WHEN** `listen_in_background(70000)` is called with an `i32`
- **THEN** it returns an `InvalidInput` error that names the port and no socket is bound
