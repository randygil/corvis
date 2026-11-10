# Spec Delta

## ADDED Requirements

### Requirement: Gateways served over Socket.IO are checked like other gateways
When an application uses the Socket.IO gateway adapter, the route posture check SHALL treat every
`#[subscribe_message]` handler as it does under the `ws` adapter (guards counted exactly as they run, the public
marker, message exemptions), with the gateway's namespace in the place of the gateway path in error lines, in the
report and in `exempt_message`.

#### Scenario: Unguarded handler under Socket.IO
- **WHEN** enforce mode is on and a handler of a gateway with namespace `/chat` has no guard and no public marker
- **THEN** `init()` fails and the error line names `/chat`, the event and the handler

#### Scenario: Exemption by namespace
- **WHEN** `exempt_message("/chat", "ping", "health probe")` is configured and the gateway's namespace is `/chat`
- **THEN** the `ping` handler is accepted

#### Scenario: Same namespace on two Engine.IO paths
- **WHEN** two gateways declare namespace `/chat` on different Engine.IO paths and both have an unguarded `ping`
  handler
- **THEN** the error lists both, told apart by the gateway name, and one `exempt_message("/chat", "ping", ...)`
  accepts both

### Requirement: The Socket.IO connection is a reported surface
The connection of each Socket.IO gateway MUST be listed among the surfaces that are reported and not enforced, with
its namespace, its Engine.IO path and whether connection guards are enabled, also in report mode, which does not run
`after_init`. Checks the framework cannot see (connect middleware registered by application code, the
`allow_request` hook) MUST NOT be reported as protection.

#### Scenario: Connection listed as unchecked
- **WHEN** an application with a Socket.IO gateway on namespace `/chat` asks for its posture report with connection
  guards off
- **THEN** the report lists the Socket.IO connection of that gateway on `/chat` with connection guards off

#### Scenario: Report mode
- **WHEN** the application runs in report mode, which stops before `after_init`
- **THEN** the report still states for each Socket.IO gateway whether connection guards are enabled

#### Scenario: Middleware is not claimed
- **WHEN** a gateway registers a connect middleware in `after_init` and connection guards are off
- **THEN** the report lists its connection with connection guards off and does not describe it as protected

### Requirement: Connection guards make the connection an enforced entry
With connection guards enabled, enforce mode MUST treat the connection of each Socket.IO gateway as a handler with
the pseudo-event `connect`: accepted when an access guard runs for it at the global or gateway level, when the
gateway is public, or when a message exemption matches `connect`. Without connection guards the connection MUST NOT
make `init()` fail in any mode.

#### Scenario: Connection enforced with connection guards
- **WHEN** enforce mode is on, connection guards are enabled and a gateway has no global or gateway-level access
  guard and no public marker
- **THEN** `init()` fails and the error lists the gateway's `connect` entry

#### Scenario: Handler-level guards do not cover the connection
- **WHEN** enforce mode is on, connection guards are enabled and every handler of a gateway has its own access guard
  but the gateway has none
- **THEN** `init()` fails and the error lists only the gateway's `connect` entry

#### Scenario: Exempt connection
- **WHEN** `exempt_message("/chat", "connect", "anonymous lobby")` is configured for that gateway
- **THEN** `init()` succeeds

#### Scenario: Connection guards off
- **WHEN** enforce mode is on, connection guards are off and every handler of a Socket.IO gateway is accepted
- **THEN** `init()` succeeds and the connection is listed as not checked
