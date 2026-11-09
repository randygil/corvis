# Spec Delta

## ADDED Requirements

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
