# Spec Delta

## ADDED Requirements

### Requirement: SSE endpoints are checked like routes
Routes declared with `#[sse]` SHALL be subject to the same rule as other controller routes (guards, public marker and
exemptions, where an exemption for `GET` or for any method matches them) and SHALL be identified as SSE in the error
and in the report.

#### Scenario: Unguarded SSE stream
- **WHEN** an application in enforce mode has an `#[sse("stream")]` route with no guard, no public marker and no exemption
- **THEN** `init()` fails and the error lists that route marked as SSE

#### Scenario: Public SSE stream
- **WHEN** the same route has `#[public]`
- **THEN** `init()` succeeds and the report lists the route as SSE with status public

### Requirement: Gateway message handlers are checked
In warn and enforce mode, every `#[subscribe_message]` handler of every WebSocket gateway MUST be accepted only if a
guard that controls access runs for it, if it is public, or if a message exemption matches its gateway path and
event. Unprotected handlers MUST appear in the same error and warning as unprotected routes, with the gateway path,
the event name and `Gateway::method`, and MUST be counted in the number of unprotected entries.

#### Scenario: Gateway guard covers its handlers
- **WHEN** a gateway has `#[use_guards(AuthGuard)]` below `#[websocket_gateway]`
- **THEN** all its message handlers are accepted

#### Scenario: Handler guard
- **WHEN** a gateway has no guard and one handler has `#[use_guards(AuthGuard)]`
- **THEN** that handler is accepted and the gateway's other unguarded handlers are reported as unprotected

#### Scenario: Unguarded handler
- **WHEN** a gateway has no guard and one of its handlers has no guard, no public marker and no exemption
- **THEN** the error lists that handler with the gateway path and event

### Requirement: Gateway guards count exactly as they run
The guards considered for a gateway message handler MUST be exactly the list executed for each message of that
handler: global guards when the application applies global enhancers to gateways, then gateway guards, then handler
guards. Global guards MUST count when they run on gateway handlers and MUST NOT count when the application keeps
global enhancers HTTP-only. Only guards that control access count.

#### Scenario: Global guard covers a gateway
- **WHEN** the only guard is a global `Provider::app_guard::<AuthGuard>()`, global enhancers apply to gateways, and a gateway handler has no guard of its own
- **THEN** the handler is accepted

#### Scenario: Global enhancers kept HTTP-only
- **WHEN** the same application keeps global enhancers HTTP-only
- **THEN** the handler is reported as unprotected

#### Scenario: Global rate limiter only
- **WHEN** the only guard that runs on a gateway handler is a global `ThrottlerGuard`
- **THEN** the handler is reported as unprotected

### Requirement: Public gateways and handlers
`#[public]` SHALL be accepted below `#[websocket_gateway]` and on `#[subscribe_message]` methods inside `#[handlers]`,
storing `PUBLIC_METADATA` with value `true` on the gateway or the handler, readable with `Reflector`, without changing
how messages are handled. A handler SHALL be public when its own value is the boolean `true`, or when it has no
boolean value and the gateway's is `true`.

#### Scenario: Public gateway
- **WHEN** a gateway without guards has `#[public]`
- **THEN** all its message handlers are accepted

#### Scenario: Public handler
- **WHEN** a gateway without guards has one handler marked `#[public]`
- **THEN** that handler is accepted and its other unguarded handlers are reported as unprotected

### Requirement: Misplaced public marker in gateways
`#[public]` with arguments, on a `#[handlers]` method that is not a `#[subscribe_message]` handler, or on a
`#[handlers]` method with no handler attribute MUST fail to compile with a message saying where it can be used.

#### Scenario: Public on a scheduled job
- **WHEN** `#[public]` is placed on an `#[on_event]`, `#[cron]`, `#[interval]` or `#[timeout]` method
- **THEN** compilation fails with an error saying that it is only supported on `#[subscribe_message]` methods

### Requirement: Message exemptions
An exemption for gateway messages SHALL match by gateway path, using the same pattern rules as route exemptions, and
by event name, compared exactly or with `*` for every event of the gateway. It MUST carry a non-empty reason under the
same rules as route exemptions, and a message exemption that matches no handler SHALL be logged as a warning.

#### Scenario: Health probe message
- **WHEN** a message exemption for path `/chat`, event `ping`, reason "health probe, no data" is configured and the `ping` handler has no guard
- **THEN** the handler is accepted and the report shows it as exempt with that reason

#### Scenario: Message exemption without a reason
- **WHEN** a message exemption is configured with a blank reason
- **THEN** `init()` fails with an error naming the exemption's path and event

### Requirement: Surfaces outside the route table are reported, not enforced
The posture report SHALL list as not checked: each static file mount registered through `ServeStaticModule` (with its
serve root), the WebSocket handshake of each gateway (with its path and whether a `verify_client` check is
configured), and the number of raw `upgrade` listeners when the report is produced after the init hooks have run.
These surfaces MUST NOT make `init()` fail in any mode.

#### Scenario: Static files with enforcement
- **WHEN** an application in enforce mode serves a folder with `ServeStaticModule` and all its routes and handlers are accepted
- **THEN** `init()` succeeds and the report lists the static mount as not checked

#### Scenario: Gateway handshake
- **WHEN** a gateway sets `verify_client` in `after_init` and the report is read after `init()`
- **THEN** the report lists the gateway handshake as not checked, with `verify_client` configured

#### Scenario: Raw upgrade listener
- **WHEN** a provider registers an `on_upgrade` listener in `on_application_bootstrap` and the report is read after `init()`
- **THEN** the report counts one raw `upgrade` listener as not checked

## MODIFIED Requirements

### Requirement: Enforce mode refuses to start
In enforce mode, if any route or gateway message handler is unprotected, `init()` MUST return a `RoutePostureError`
before module middleware is configured and before any gateway `after_init`, `OnModuleInit` or
`OnApplicationBootstrap` hook runs, and `listen()` and `listen_in_background()` MUST return that error without binding
a socket. The error MUST be downcastable from the returned `anyhow::Error` and logged once at error level.

#### Scenario: Unprotected route at listen
- **WHEN** an application in enforce mode has an unprotected route and calls `listen_in_background(0)`
- **THEN** it returns a `RoutePostureError`, no socket is bound and no init hook ran

#### Scenario: Unprotected gateway handler before after_init
- **WHEN** an application in enforce mode has an unprotected gateway message handler and the gateway implements `after_init`
- **THEN** `init()` returns a `RoutePostureError` and the gateway's `after_init` did not run

#### Scenario: All routes accepted
- **WHEN** every route and gateway message handler of an application in enforce mode is guarded, public or exempt
- **THEN** `init()` succeeds and the application serves requests and messages as it would with posture off

### Requirement: Posture report
After `init()`, in every mode including off, the application SHALL return a posture report listing every compiled
controller route and every gateway message handler with its kind (HTTP, SSE or WebSocket message), method or event,
full path, owner and handler names, its guards with whether each controls access, and its status (protected, public,
exempt with its reason, or unprotected), plus the not-checked surfaces. Before `init()` it SHALL return nothing. The
report SHALL be printable as a table and serializable to JSON.

#### Scenario: Report of a mixed application
- **WHEN** an application with a guarded route, a public route, an exempt route and an unguarded route calls `init()` and reads the report
- **THEN** the report contains the four routes with statuses protected, public, exempt (with its reason) and unprotected

#### Scenario: Report in off mode
- **WHEN** an application that never enabled route posture calls `init()` and reads the report
- **THEN** the report lists its routes and their statuses, and nothing was logged about posture during `init()`

#### Scenario: Report with a gateway
- **WHEN** an application with a guarded gateway handler and an `#[sse]` route calls `init()` and reads the report
- **THEN** the report lists the handler with kind WebSocket message, its event and its guards, the route with kind SSE, and the gateway handshake as not checked
