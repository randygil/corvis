# ws/gateways Specification

## Purpose
A gateway has at most one handler per event name, so no message handler, and none of its guards, is silently
replaced by another.

## Requirements

### Requirement: Duplicate events in one gateway fail startup
When a gateway declares more than one `#[subscribe_message]` handler for the same event name, application
initialization (`init()`, and `listen` / `listen_in_background`, which call it) SHALL fail with an error that names
the gateway, the event and the first two handler methods that use it, in declaration order:
`<Gateway> has more than one handler for the "<event>" event (<first_method> and <second_method>); a gateway can have only one handler per event`.

#### Scenario: Two handlers for the same event
- **WHEN** `ChatGateway` declares `#[subscribe_message("message")] fn on_message` and later
  `#[subscribe_message("message")] fn on_message_v2`
- **THEN** `app.init()` returns the error
  `ChatGateway has more than one handler for the "message" event (on_message and on_message_v2); a gateway can have only one handler per event`
  and the application does not listen

#### Scenario: Guarded handler shadowed by an unguarded one
- **WHEN** a gateway declares `#[use_guards(AdminGuard)] #[subscribe_message("purge")] fn purge` and later
  `#[subscribe_message("purge")] fn purge_all` without guards
- **THEN** initialization fails with the duplicate-event error naming `purge` and `purge_all`, instead of serving
  `purge_all` to every client

#### Scenario: Gateway hooks do not run
- **WHEN** a gateway that implements `OnGatewayInit` has a duplicate event
- **THEN** initialization fails and that gateway's `after_init` is not called

#### Scenario: Event names that differ only in case
- **WHEN** a gateway declares `#[subscribe_message("message")]` and `#[subscribe_message("Message")]`
- **THEN** initialization succeeds and each frame reaches the handler whose event matches it exactly

#### Scenario: Same event in different gateways
- **WHEN** a gateway on `/chat` and a gateway on `/admin` both declare `#[subscribe_message("ping")]`
- **THEN** initialization succeeds and each gateway answers `ping` with its own handler

### Requirement: Duplicate events are checked before the gateway starts
The check MUST run before the gateway is registered and before its `after_init` hook runs. Event names MUST be
compared exactly, the same way incoming frames are matched to handlers. This intentionally diverges from the original framework (see design.md).

#### Scenario: after_init does not run
- **WHEN** a gateway with two handlers for "message" also implements `after_init`
- **THEN** `init()` fails and `after_init` was never called, and handlers whose event names differ only in case are
  both accepted
