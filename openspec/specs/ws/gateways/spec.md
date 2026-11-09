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

### Requirement: Global guards, interceptors and pipes apply to gateway handlers
By default the framework SHALL apply the application's global guards, interceptors and pipes to every
`#[subscribe_message]` handler, whether they were registered with `use_global_guards`, `use_global_interceptors`,
`use_global_pipes` or with `Provider::app_guard`, `app_interceptor`, `app_pipe`, as the original framework does in a
non-hybrid application.

#### Scenario: Global guard rejects a message
- **WHEN** the application registers `Provider::app_guard::<DenyGuard>()`, `DenyGuard` returns `false`, and a client
  sends `{ "event": "join", "data": "lobby" }` to a gateway without its own guards
- **THEN** the handler does not run and the client receives
  `{ "event": "exception", "data": { "status": "error", "message": "Forbidden resource" } }`

#### Scenario: Guard tells transports apart
- **WHEN** a global guard returns `true` when `ctx.get_type() == ContextType::Ws` and `false` otherwise
- **THEN** HTTP routes answer 403 and gateway messages reach their handlers

#### Scenario: Gateway marked public through metadata
- **WHEN** a global guard skips targets with `#[set_metadata(IS_PUBLIC_KEY, true)]` and the gateway declares that
  metadata below `#[websocket_gateway]`
- **THEN** messages to the gateway reach their handlers without credentials

#### Scenario: Global pipe transforms a message body
- **WHEN** the application calls `use_global_pipes(TrimPipe)` and a handler takes `#[message_body] text: String`
- **THEN** the handler receives the trimmed text

### Requirement: Global enhancers run before the gateway's and the handler's
Gateway handlers SHALL order enhancers as HTTP routes do: global ones first (provider-registered ahead of
`use_global_*`), then the gateway's, then the handler's. Global pipes MUST run on every `#[message_body]` argument
before the gateway's, handler's and parameter's pipes.

#### Scenario: Global guard runs before the gateway's guards
- **WHEN** the application calls `use_global_guards(A)` and the gateway declares `#[use_guards(B)]`, both returning
  `true` and recording their call
- **THEN** `A` is recorded before `B` for every message

### Requirement: Global exception filters do not apply to gateway handlers
The framework SHALL NOT apply filters registered with `use_global_filters` or `Provider::app_filter` to gateway
handlers; only handler and gateway filters and the built-in mapping take part, as in the original framework.

#### Scenario: HTTP-only global filter
- **WHEN** the application registers `Provider::app_filter::<AllFilter>()` and a gateway handler fails
- **THEN** `AllFilter` is not called and the client receives the built-in `exception` frame

### Requirement: Applications can keep global enhancers HTTP-only
The framework SHALL provide `CorvisApplication::apply_global_enhancers_to_gateways(bool)`, `true` by default. When set
to `false` before the application is initialized, gateway handlers MUST run only their gateway's and their own
enhancers, as before this change.

#### Scenario: Switch turned off
- **WHEN** the application registers `Provider::app_guard::<DenyGuard>()`, calls
  `app.apply_global_enhancers_to_gateways(false)` before `init()`, and a client sends a message
- **THEN** the handler runs and HTTP routes still answer 403

### Requirement: The switch has no effect after initialization
Calling `apply_global_enhancers_to_gateways` after the application was initialized SHALL NOT change the gateways
already built and MUST log a warning saying the call has no effect.

#### Scenario: Switch called too late
- **WHEN** `apply_global_enhancers_to_gateways(false)` is called after `init()`
- **THEN** a warning is logged and global guards keep applying to gateway handlers

### Requirement: Gateway message handlers accept `#[ip]`
A parameter of a `#[subscribe_message]` method in `#[handlers]` marked `#[ip]` SHALL receive the client address of
the connection's handshake, the value of `client.handshake().ip()`: resolved with the application's `trust proxy`
setting and not grouped by IPv6 prefix.

#### Scenario: Behind one trusted proxy
- **WHEN** an application with `set_trust_proxy(1)` has a gateway handler `fn who(&self, #[ip] ip: String) -> String`
  that returns `ip`, and a client connects from `127.0.0.1` with `X-Forwarded-For: 203.0.113.7` and sends the event
- **THEN** the reply's data is `"203.0.113.7"`

#### Scenario: Default setting
- **WHEN** the same application without a trust setting receives that connection and message
- **THEN** the reply's data is `"127.0.0.1"`

#### Scenario: Full IPv6 address
- **WHEN** an application with `set_trust_proxy(1)` receives the message through a handshake with
  `X-Forwarded-For: 2001:db8:1:2:aaaa:bbbb:cccc:dddd`
- **THEN** the reply's data is `"2001:db8:1:2:aaaa:bbbb:cccc:dddd"`

### Requirement: `#[ip]` combines with the other gateway parameters
A gateway handler's `#[ip]` parameter SHALL NOT consume the frame's `data`, so it can be combined with
`#[message_body]`, `#[payload]` and `#[connected_socket]` parameters in any order.

#### Scenario: Mixed with the body and the socket
- **WHEN** a handler is `fn echo(&self, #[ip] ip: String, #[message_body("text")] text: String, #[connected_socket]
  client: WsClient) -> Value` returning `{ ip, text, same: client.handshake().ip() == ip }`, and a client from
  `127.0.0.1` sends `{ "event": "echo", "data": { "text": "hi" } }`
- **THEN** the reply's data is `{ "ip": "127.0.0.1", "text": "hi", "same": true }`

### Requirement: The gateway `#[ip]` value is converted like a controller's
The address SHALL be converted to the parameter type as a controller's `#[ip]` parameter is. When the conversion fails
the client MUST receive an `exception` frame and the handler MUST NOT run.

#### Scenario: Conversion error
- **WHEN** a handler declares `#[ip] ip: u32` and a client sends its event
- **THEN** the client receives an `exception` frame and the handler does not run

### Requirement: Pipes do not run on `#[ip]` in gateways
Global, gateway, handler and parameter pipes SHALL NOT run on a gateway handler's `#[ip]` parameter, and `#[ip]` in
`#[handlers]` MUST NOT take arguments.

#### Scenario: Gateway pipe and an IP parameter
- **WHEN** a gateway has `#[use_pipes(CountingPipe)]`, where `CountingPipe` counts its calls, and a handler that takes
  only `#[ip] ip: String` receives a message
- **THEN** the handler runs and `CountingPipe` was not called

#### Scenario: Arguments are a compile error
- **WHEN** a `#[subscribe_message("who")]` method `who` declares `#[ip(ParseIntPipe)] ip: String`
- **THEN** compilation fails with an error naming `who` and stating that `#[ip]` takes no arguments in gateway handlers

### Requirement: `#[ip]` outside message handlers is a compile error
In `#[handlers]`, `#[ip]` on a parameter of an `#[on_event]`, `#[cron]`, `#[interval]` or `#[timeout]` method MUST be
a compile error whose message names the method and says that `#[ip]` is only available on `#[subscribe_message]`
handlers (and on routes in `#[routes]`).

#### Scenario: Event handler
- **WHEN** a provider declares `#[on_event("order.created")] fn on_order(&self, #[ip] ip: String)` in `#[handlers]`
- **THEN** compilation fails with an error naming `on_order` and `#[subscribe_message]`

#### Scenario: Message handler compiles
- **WHEN** the same parameter is declared on a `#[subscribe_message("order")]` method
- **THEN** the code compiles
