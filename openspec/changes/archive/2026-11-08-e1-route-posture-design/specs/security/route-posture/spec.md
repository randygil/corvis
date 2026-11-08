# Spec Delta

## Purpose

Guarantees that, when an application asks for it, no HTTP route, SSE endpoint or WebSocket message handler is
served without either a guard that controls access or an explicit, reviewable statement that it is open, and that the
posture of every route can be listed for review.

## ADDED Requirements

### Requirement: Route posture is opt-in and off by default
An application SHALL have three posture modes: off, warn and enforce. The mode MUST default to off, and in off mode
the framework MUST NOT change routing, guard execution, startup or responses in any way. `require_route_posture()`
SHALL select enforce mode; `set_route_posture(options)` SHALL select any mode together with exemptions.

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

### Requirement: Guards count at every level the router runs them
The guards considered for a route MUST be exactly the list the router executes for it: global guards registered with
`use_global_guards`, global guards registered as providers with `Provider::app_guard`, controller guards and route
guards.

#### Scenario: Route guarded at controller level
- **WHEN** a controller has `#[use_guards(AuthGuard)]` below `#[controller]` and one of its methods has no guard
- **THEN** that route is accepted

#### Scenario: Global guard registered as a provider
- **WHEN** a module lists `Provider::app_guard::<AuthGuard>()` and a route of any controller has no guard of its own
- **THEN** that route is accepted

### Requirement: Only guards that control access count
A guard SHALL count for posture only when it declares that it controls access. Guards MUST declare it by default; a
guard that only limits or validates requests SHALL be able to declare that it does not control access, and the
framework's `ThrottlerGuard` and `ZodGuard` MUST declare that they do not.

#### Scenario: Only a rate limiter
- **WHEN** the only guard of a route is a global `ThrottlerGuard` and the route is not public or exempt
- **THEN** the route is reported as unprotected

#### Scenario: Custom guard with default behavior
- **WHEN** a route's only guard is a user guard that does not override the declaration
- **THEN** the route is accepted

### Requirement: The public marker
`#[public]` SHALL be accepted on a route method inside `#[routes]` and below `#[controller]`. It MUST store the
metadata entry `PUBLIC_METADATA` (value `"isPublic"`) with value `true` on the handler or on the controller, readable
with `Reflector`, and MUST NOT change how requests are handled.

#### Scenario: Marker does not change handling
- **WHEN** a route has `#[public]`
- **THEN** its requests are handled exactly as without the attribute

#### Scenario: Existing global guard honors the marker
- **WHEN** a global guard skips routes whose `"isPublic"` metadata is `true` and a route has `#[public]`
- **THEN** the guard lets requests to that route through

### Requirement: Public resolution order
A route SHALL be public when the handler's `PUBLIC_METADATA` value is `true`, or when the handler has no such entry and
the controller's value is `true`. Setting the entry with `#[set_metadata(PUBLIC_METADATA, ...)]` or a custom decorator
SHALL have the same effect as `#[public]`.

#### Scenario: Public controller with an overriding route
- **WHEN** a controller has `#[public]` and one of its routes has `#[set_metadata(PUBLIC_METADATA, false)]` and no guard
- **THEN** that route is reported as unprotected and the other routes of the controller are accepted

### Requirement: Exemptions carry a reason
An exemption SHALL match routes by HTTP method (or any method) and by a pattern compared with the route's full path
template (global prefix and version segment included), using the same pattern rules as middleware route matching. An
exemption MUST have a non-empty reason; configuring one with an empty reason MUST fail at `init()`.

#### Scenario: Route protected outside the framework
- **WHEN** an exemption for `GET /metrics` with the reason "scraped from the private network only" is configured
- **THEN** the unguarded route `GET /metrics` is accepted and the report shows it as exempt with that reason

#### Scenario: Exemption without a reason
- **WHEN** an exemption is configured with an empty or blank reason
- **THEN** `init()` fails with an error naming the exemption

### Requirement: Enforce mode refuses to start
In enforce mode, if any route is unprotected, `init()` MUST return a `RoutePostureError` before any `OnModuleInit` or
`OnApplicationBootstrap` hook runs, and `listen()` and `listen_in_background()` MUST return that error without binding
a socket.

#### Scenario: Unprotected route at listen
- **WHEN** an application in enforce mode has an unprotected route and calls `listen_in_background(0)`
- **THEN** it returns a `RoutePostureError`, no socket is bound and no init hook ran

#### Scenario: All routes accepted
- **WHEN** every route of an application in enforce mode is guarded, public or exempt
- **THEN** `init()` succeeds and the application serves requests as it would with posture off

### Requirement: The posture error lists every unprotected route
The posture error message MUST state how many routes are unprotected, list every one of them (not only the first)
with its method, full path and owner and handler names, and say how to fix it (add a guard, mark it `#[public]`, or
add an exemption with a reason). It MUST NOT include request data or configuration values.

#### Scenario: Two unprotected routes
- **WHEN** an application in enforce mode has two unprotected routes
- **THEN** the error message names both routes as `Controller::handler` with their method and path, and the fix

### Requirement: Warn mode reports without failing
In warn mode, if any route is unprotected, the framework SHALL log the same list as the enforce error once, at warning
level, during `init()`, and the application MUST start normally.

#### Scenario: Migration in warn mode
- **WHEN** an application in warn mode has an unprotected route
- **THEN** a warning listing that route is logged and the application serves requests

### Requirement: SSE endpoints are checked like routes
Routes declared with `#[sse]` SHALL be subject to the same rule as other controller routes and SHALL be identified as
SSE in the error and in the report.

#### Scenario: Unguarded SSE stream
- **WHEN** an application in enforce mode has an `#[sse("stream")]` route with no guard, no public marker and no exemption
- **THEN** `init()` fails and the error lists that route marked as SSE

### Requirement: Gateway message handlers are checked
In warn and enforce mode, every `#[subscribe_message]` handler of every WebSocket gateway MUST be accepted only if a
gateway-level or handler-level guard that controls access runs for it, if it is public, or if an exemption matches its
gateway path and event. Unprotected handlers MUST appear in the error with the gateway path, the event name and
`Gateway::method`.

#### Scenario: Gateway guard covers its handlers
- **WHEN** a gateway has `#[use_guards(AuthGuard)]` below `#[websocket_gateway]`
- **THEN** all its message handlers are accepted

#### Scenario: Unguarded handler
- **WHEN** a gateway has no guard and one of its handlers has no guard, no public marker and no exemption
- **THEN** the error lists that handler with the gateway path and event

### Requirement: Global guards do not count for gateways
Because global guards do not run on gateway message handlers, they MUST NOT count for their posture.

#### Scenario: Global guard does not cover a gateway
- **WHEN** the only guard is a global `Provider::app_guard::<AuthGuard>()` and a gateway handler has no guard
- **THEN** the handler is reported as unprotected

### Requirement: Public gateways and handlers
`#[public]` SHALL be accepted below `#[websocket_gateway]` and on `#[subscribe_message]` methods, storing
`PUBLIC_METADATA`; a handler SHALL be public when its own value is `true`, or when it has none and the gateway's is.

#### Scenario: Public gateway
- **WHEN** a gateway without guards has `#[public]`
- **THEN** all its message handlers are accepted

### Requirement: Surfaces outside the route table are reported, not enforced
Static file mounts registered through `ServeStaticModule`, the WebSocket handshake of each gateway (with whether a
`verify_client` check is configured), and the number of raw `upgrade` listeners (when the report is produced after the
init hooks have run) SHALL appear in the posture report as not checked, and MUST NOT make `init()` fail.

#### Scenario: Static files with enforcement
- **WHEN** an application in enforce mode serves a folder with `ServeStaticModule` and all its routes are accepted
- **THEN** `init()` succeeds and the report lists the static mount as not checked

### Requirement: Posture report
After `init()`, in every mode including off, the application SHALL return a posture report listing every controller
route and gateway message handler with its kind (HTTP, SSE or WebSocket message), method or event, full path, owner
and handler names, its guards with whether each controls access, and its status (protected, public, exempt with its
reason, or unprotected), plus the not-checked surfaces.

#### Scenario: Report of a mixed application
- **WHEN** an application with a guarded route, a public route and an unguarded route calls `init()` and reads the report
- **THEN** the report contains the three routes with statuses protected, public and unprotected

### Requirement: Posture report from the CLI
`corvis posture` SHALL start the application of the current project far enough to compute the report, print it as a
table (or as JSON with `--json`), stop before the application listens or runs any init hook, and exit with a non-zero
status when any route is unprotected, whatever the posture mode of the application.

#### Scenario: CI gate
- **WHEN** `corvis posture --json` runs in a project with one unprotected route
- **THEN** it prints the JSON report, the application does not listen on any port, and the command exits with a non-zero status
