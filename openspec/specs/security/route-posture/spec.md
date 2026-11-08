# security/route-posture Specification

## Purpose
Guarantees that, when an application asks for it, no controller route is served without either a guard that controls
access or an explicit, reviewable statement that it is open, and that the posture of every route can be listed for
review. SSE endpoints and WebSocket gateways are added by a later change.

## Requirements

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
with the route's full path template (global prefix and version segment included), literally: a literal segment matches
the same literal segment (ignoring ASCII case); `:x` or `{x}` matches only a `:param` or `{param}` segment of the
template; `*` as a whole segment matches exactly one segment and `**` matches zero or more segments; `*` inside a
segment never crosses a `/`. In warn and enforce mode, an exemption that matches only routes that are already
protected or public MUST be logged as a warning.

#### Scenario: Parameter exemption does not cover a literal sibling
- **WHEN** an exemption for `GET /files/:id` is configured and the application also has an unguarded `GET /files/export-all`
- **THEN** only `GET /files/:id` is accepted as exempt

#### Scenario: Glob inside a segment
- **WHEN** an exemption for `GET /health*` is configured
- **THEN** it matches `/healthz` and does not match `/healthcare/records/:id`

#### Scenario: Redundant exemption
- **WHEN** an exemption matches only routes that already have an access guard or are public
- **THEN** a warning names the exemption and `init()` continues

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

### Requirement: Report mode of the application
When the environment variable `CORVIS_ROUTE_POSTURE_REPORT` is `text` or `json` (surrounding whitespace ignored),
`init()` SHALL compute the posture report regardless of the configured posture mode, write it to stdout as the text
table or as JSON, flush stdout, and end the process with exit code 40 when no entry is unprotected and 41 otherwise (never 0, so that
a process stopped in report mode cannot pass for a normal successful exit). This MUST happen after route and gateway
compilation, before any `OnModuleInit` or `OnApplicationBootstrap` hook runs and before any socket is bound. The
report SHALL list the surfaces the check cannot enforce that are known at that point (static mounts, WebSocket
handshakes with `verify_client` unknown, raw upgrade listeners as not counted). Applications created by the testing
module MUST ignore report mode, and exemptions with a blank reason MUST make `init()` fail in report mode too.

#### Scenario: Unprotected route in report mode
- **WHEN** an application with posture off and one unguarded, non-public route runs `init()` with
  `CORVIS_ROUTE_POSTURE_REPORT=text`
- **THEN** the process prints a table that lists that route as unprotected, exits with code 41, and no init hook ran

#### Scenario: JSON report with everything accepted
- **WHEN** an application whose routes are all guarded or public runs `listen(0)` with
  `CORVIS_ROUTE_POSTURE_REPORT=json`
- **THEN** the printed report parses as the JSON posture report, the process exits with code 40, and no socket was
  bound

### Requirement: The report is delimited on stdout
In report mode the report SHALL be written between the line `----- BEGIN CORVIS ROUTE POSTURE REPORT -----` and the
line `----- END CORVIS ROUTE POSTURE REPORT -----`, so it can be separated from log lines written to the same stream. When
`CORVIS_ROUTE_POSTURE_NONCE` is set, both lines SHALL contain its value after `REPORT`. `corvis posture` SHALL set it
to a random value for each run and accept only markers that contain it, keeping the last complete report.

#### Scenario: Fake markers in the output
- **WHEN** the application prints lines that look like a clean report with the plain markers before the real report
- **THEN** `corvis posture` treats them as ordinary output and reports the real one

#### Scenario: Logs before the report
- **WHEN** the application logs startup lines to stdout and then runs `init()` in report mode
- **THEN** the report appears after those lines, between the two marker lines, and nothing else is between them

### Requirement: Report mode only with a valid value
An unset or empty `CORVIS_ROUTE_POSTURE_REPORT` MUST leave `init()` unchanged. Any other value than `text` or `json`
MUST make `init()` fail with an error that names the variable and the accepted values, without ending the process.

#### Scenario: Unknown value
- **WHEN** `CORVIS_ROUTE_POSTURE_REPORT=yaml` is set and the application calls `init()`
- **THEN** `init()` returns an error naming `CORVIS_ROUTE_POSTURE_REPORT`, `text` and `json`, and the process is not
  ended by the framework

#### Scenario: Variable not set
- **WHEN** the variable is unset or empty
- **THEN** `init()` behaves exactly as without this feature

### Requirement: Posture report from the CLI
`corvis posture` SHALL start the application of the current project far enough to compute the report, print it as a
table (or as JSON with `--json`), stop before the application listens or runs any init hook, and exit with a non-zero
status when any route is unprotected, whatever the posture mode of the application. It SHALL run the project like
`corvis start` (`cargo run`, honoring `--release`, passing the arguments after `--` to the application).

#### Scenario: CI gate
- **WHEN** `corvis posture --json` runs in a project with one unprotected route
- **THEN** it prints the JSON report, the application does not listen on any port, and the command exits with a
  non-zero status

#### Scenario: Clean project
- **WHEN** `corvis posture` runs in a project whose routes are all guarded, public or exempt
- **THEN** it prints the table and exits with status 0

### Requirement: CLI output and exit status
`corvis posture` SHALL write only the report to its stdout and send Cargo output and the application's other output
to its stderr. Its exit status MUST be 0 when nothing is unprotected, 1 when the selected report lists an unprotected
entry (whatever the application's exit code), and 2, with an explanatory message on stderr, when the application ends
without printing a complete report or a clean report is not followed by the report-mode exit code 40.

#### Scenario: Output stays parseable
- **WHEN** the application logs startup lines to stdout before `init()` and `corvis posture --json` runs
- **THEN** the command's stdout contains only the JSON report

#### Scenario: Application that fails to start
- **WHEN** `corvis posture` runs in a project whose application returns a dependency injection error before `init()`
- **THEN** the command prints on stderr that the application ended without producing a posture report, with the
  application's exit code, and exits with status 2

### Requirement: Timeout for applications that never report
`corvis posture` SHALL stop the application and exit with status 2 when it has not printed a complete report within
`--timeout <seconds>` (default 120) after it started running. The time spent building MUST NOT count.

#### Scenario: Application that never reaches init
- **WHEN** `corvis posture --timeout 3` runs in a project whose `main` blocks before the application initializes
- **THEN** the command stops the application and its child processes, prints on stderr that no report arrived within
  3 seconds, and exits with status 2

### Requirement: The timeout bounds the whole run
`--timeout` SHALL bound the time from the start of the application until the command returns, also when a process the
application started keeps its stdout open. When the command ends, no process of the run MUST be left running.

#### Scenario: Helper that holds stdout
- **WHEN** the application starts a long-lived helper that inherits stdout, prints the report and exits
- **THEN** the command prints the report and returns promptly, and the helper is no longer running
