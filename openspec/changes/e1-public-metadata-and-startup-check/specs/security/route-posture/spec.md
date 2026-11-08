# Spec Delta

## Purpose

Guarantees that, when an application asks for it, no controller route is served without either a guard that controls
access or an explicit, reviewable statement that it is open, and that the posture of every route can be listed for
review. SSE endpoints and WebSocket gateways are added by a later change.

## ADDED Requirements

### Requirement: Route posture is opt-in and off by default
An application SHALL have three posture modes: off, warn and enforce. The mode MUST default to off, and in off mode
the framework MUST NOT change routing, guard execution, startup, logging or responses in any way.
`require_route_posture()` SHALL select enforce mode; `set_route_posture(options)` SHALL select any mode together with
exemptions. Both MUST be called before `init()` or `listen()` to take effect.

#### Scenario: Existing application without the option
- **WHEN** an application with unguarded routes calls `init()` or `listen()` without enabling route posture
- **THEN** it starts exactly as before, with no posture error and no posture log line

#### Scenario: Enforcement enabled
- **WHEN** an application calls `require_route_posture()` before `init()` or `listen()`
- **THEN** the posture check runs during `init()`

### Requirement: A route is accepted only when guarded, public or exempt
In warn and enforce mode, every compiled controller route (every HTTP method, path alias and URI version) MUST be
accepted only if a guard that controls access runs for it, if it is marked public, or if a configured exemption
matches it. Every other route is unprotected.

#### Scenario: Route with no guard anywhere
- **WHEN** a route has no route, controller or global guard, is not marked public and matches no exemption
- **THEN** the route is reported as unprotected

#### Scenario: Public route
- **WHEN** a route without guards is marked public
- **THEN** the route is accepted

#### Scenario: Path aliases and versions are checked one by one
- **WHEN** an unguarded handler is published at two paths, or at two URI versions, and an exemption matches only one of them
- **THEN** the matched path is accepted as exempt and the other is reported as unprotected

### Requirement: Guards count at every level the router runs them
The guards considered for a route MUST be exactly the list the router executes for it: global guards registered with
`use_global_guards`, global guards registered as providers with `Provider::app_guard`, controller guards and route
guards.

#### Scenario: Route guarded at controller level
- **WHEN** a controller has `#[use_guards(AuthGuard)]` below `#[controller]` and one of its methods has no guard
- **THEN** that route is accepted

#### Scenario: Global guard registered with use_global_guards
- **WHEN** the application calls `use_global_guards(AuthGuard)` and a route has no guard of its own
- **THEN** that route is accepted

#### Scenario: Global guard registered as a provider
- **WHEN** a module lists `Provider::app_guard::<AuthGuard>()` and a route of any controller has no guard of its own
- **THEN** that route is accepted

### Requirement: Only guards that control access count
A guard SHALL count for posture only when it declares that it controls access. Guards MUST declare it by default; a
guard that only limits or validates requests SHALL be able to declare that it does not control access, and the
framework's `ThrottlerGuard` and `ZodGuard` MUST declare that they do not. The declaration MUST NOT change how the
guard runs at request time.

#### Scenario: Only a rate limiter
- **WHEN** the only guard of a route is a global `ThrottlerGuard` and the route is not public or exempt
- **THEN** the route is reported as unprotected

#### Scenario: Custom guard with default behavior
- **WHEN** a route's only guard is a user guard that does not override the declaration
- **THEN** the route is accepted

### Requirement: The public marker
`#[public]` SHALL be accepted below `#[controller]` and on a route method inside `#[routes]`. It MUST store the
metadata entry `PUBLIC_METADATA` (value `"isPublic"`) with value `true` on the controller or on the handler, readable
with `Reflector`, and MUST NOT change how requests are handled. `#[public]` with arguments, on a method of a
`#[routes]` block that has no route attribute, or on any other item MUST fail to compile with a message saying where
it can be used.

#### Scenario: Marker does not change handling
- **WHEN** a route has `#[public]`
- **THEN** its requests are handled exactly as without the attribute

#### Scenario: Existing global guard honors the marker
- **WHEN** a global guard skips routes whose `"isPublic"` metadata is `true` and a route has `#[public]`
- **THEN** the guard lets requests to that route through

#### Scenario: Marker with arguments
- **WHEN** a route method has `#[public(true)]`
- **THEN** compilation fails with an error saying that `#[public]` takes no arguments

### Requirement: Public resolution order
A route SHALL be public when the handler's `PUBLIC_METADATA` value is the boolean `true`, or when the handler has no
boolean value for it and the controller's value is `true` (the resolution of `Reflector::get_all_and_override`).
Setting the entry with `#[set_metadata(PUBLIC_METADATA, ...)]`, with the same literal key, or with a custom decorator
SHALL have the same effect as `#[public]`.

#### Scenario: Public controller with an overriding route
- **WHEN** a controller has `#[public]` and one of its routes has `#[set_metadata(PUBLIC_METADATA, false)]` and no guard
- **THEN** that route is reported as unprotected and the other routes of the controller are accepted

#### Scenario: Existing metadata convention
- **WHEN** a route has `#[set_metadata(IS_PUBLIC_KEY, true)]` with `IS_PUBLIC_KEY = "isPublic"` and no guard
- **THEN** the route is accepted as public

### Requirement: Exemptions match by method and full path
An exemption SHALL match routes by HTTP method (or any method with `RequestMethod::All`) and by a pattern compared
with the route's full path template (global prefix and version segment included), using the same pattern rules as
middleware route matching.

#### Scenario: Route protected outside the framework
- **WHEN** an exemption for `GET /metrics` with the reason "scraped from the private network only" is configured
- **THEN** the unguarded route `GET /metrics` is accepted and the report shows it as exempt with that reason

#### Scenario: Wildcard under a global prefix
- **WHEN** the global prefix is `api`, URI versioning publishes `/api/v1/internal/jobs`, and an exemption for any method and `api/*/internal/*` is configured
- **THEN** that route is accepted as exempt

### Requirement: Exemptions carry a reason
An exemption MUST have a non-empty reason. In warn and enforce mode, an exemption with an empty or blank reason MUST
make `init()` fail before any route is compiled.

#### Scenario: Exemption without a reason
- **WHEN** an exemption is configured with an empty or blank reason
- **THEN** `init()` fails with an error naming the exemption's method and pattern

### Requirement: Stale exemptions warn
In warn and enforce mode, an exemption that matches no route SHALL be logged as a warning and MUST NOT make `init()`
fail.

#### Scenario: Stale exemption
- **WHEN** an exemption matches no compiled route
- **THEN** a warning naming the exemption is logged and `init()` continues

### Requirement: Enforce mode refuses to start
In enforce mode, if any route is unprotected, `init()` MUST return a `RoutePostureError` before module middleware is
configured and before any gateway `after_init`, `OnModuleInit` or `OnApplicationBootstrap` hook runs, and `listen()`
and `listen_in_background()` MUST return that error without binding a socket. The error MUST be downcastable from the
returned `anyhow::Error` and logged once at error level.

#### Scenario: Unprotected route at listen
- **WHEN** an application in enforce mode has an unprotected route and calls `listen_in_background(0)`
- **THEN** it returns a `RoutePostureError`, no socket is bound and no init hook ran

#### Scenario: All routes accepted
- **WHEN** every route of an application in enforce mode is guarded, public or exempt
- **THEN** `init()` succeeds and the application serves requests as it would with posture off

### Requirement: The posture error lists every unprotected route
The posture error message MUST state how many routes are unprotected, list every one of them (not only the first)
with its method, full path and `Controller::handler`, sorted by path then method, and say how to fix it (add a guard,
mark it `#[public]`, or add an exemption with a reason, and that guards which do not control access do not count). It
MUST NOT include request data or configuration values.

#### Scenario: Two unprotected routes
- **WHEN** an application in enforce mode has two unprotected routes
- **THEN** the error message names both routes as `Controller::handler` with their method and path, and the fix

### Requirement: Warn mode reports without failing
In warn mode, if any route is unprotected, the framework SHALL log the same list as the enforce error once, at warning
level, during `init()`, and the application MUST start normally.

#### Scenario: Migration in warn mode
- **WHEN** an application in warn mode has an unprotected route
- **THEN** a warning listing that route is logged and the application serves requests

### Requirement: Posture report
After `init()`, in every mode including off, the application SHALL return a posture report listing every compiled
controller route with its kind, method, full path, controller and handler names, its guards with whether each controls
access, and its status (protected, public, exempt with its reason, or unprotected). Before `init()` it SHALL return
nothing. The report SHALL be printable as a table and serializable to JSON.

#### Scenario: Report of a mixed application
- **WHEN** an application with a guarded route, a public route, an exempt route and an unguarded route calls `init()` and reads the report
- **THEN** the report contains the four routes with statuses protected, public, exempt (with its reason) and unprotected

#### Scenario: Report in off mode
- **WHEN** an application that never enabled route posture calls `init()` and reads the report
- **THEN** the report lists its routes and their statuses, and nothing was logged about posture during `init()`
