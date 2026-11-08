# ws/exception-filters Specification

## Purpose
Exception filters on WebSocket gateways: where they can be declared, the order in which they are tried, and how the
response a filter returns becomes the `exception` frame sent to the client.

## Requirements

### Requirement: Exception filters can be declared on gateways and message handlers
The framework SHALL accept `#[use_filters(...)]` below `#[websocket_gateway]`, applying to every handler of the
gateway, and on `#[subscribe_message]` methods inside `#[handlers]`, applying to that handler. The arguments MUST be
the same as on controllers (a type resolved through DI, or an expression), and the filters MUST implement the
`ExceptionFilter` trait used by HTTP routes.

#### Scenario: Filter below the gateway attribute
- **WHEN** a gateway declares `#[use_filters(RoomFilter)]` below `#[websocket_gateway(path = "/chat")]` and the
  application is built
- **THEN** the crate compiles and `RoomFilter` is resolved from the gateway's module when the application starts

### Requirement: Filters on other handler kinds are rejected
`#[use_filters]` on a method inside `#[handlers]` that is not `#[subscribe_message]` (`#[on_event]`, `#[cron]`,
`#[interval]`, `#[timeout]`) SHALL remain a compile error.

#### Scenario: Filter on an event handler
- **WHEN** `#[use_filters(F)]` is placed on an `#[on_event("x")]` method inside `#[handlers]`
- **THEN** compilation fails with a message saying enhancers in `#[handlers]` are only supported on
  `#[subscribe_message]` methods

### Requirement: Gateway filters are tried handler first, then gateway
When a guard, interceptor, pipe or handler of a message fails, including a guard that returns `false` and a handler
that panics, the framework SHALL try the handler's filters and then the gateway's filters, and use the first one whose
`catches` returns `true`. Within one level, filters MUST be tried in the same order HTTP routes use for filters declared
at one level.

#### Scenario: Handler filter wins over gateway filter
- **WHEN** a gateway declares `#[use_filters(GatewayFilter)]`, a handler declares `#[use_filters(HandlerFilter)]`,
  both catch every exception, and the handler returns an error
- **THEN** `HandlerFilter` builds the frame and `GatewayFilter` is not called

#### Scenario: Guard refusal goes through the filters
- **WHEN** a handler guard returns `false` and the handler has a filter that catches every exception
- **THEN** the filter receives the `Forbidden resource` exception and builds the frame

### Requirement: The built-in mapping answers when no filter catches
If no handler or gateway filter catches the exception, the framework SHALL send the frame produced by the built-in
mapping (`exception_payload`), unchanged from the behavior without filters.

#### Scenario: Filter that does not catch the exception
- **WHEN** the only filter of a handler returns `false` from `catches` for the raised exception
- **THEN** the client receives the frame of the built-in mapping

### Requirement: A filter's JSON or text body becomes the frame data
The framework SHALL send `{ "event": "exception", "data": <body> }` for a filter response whose body is JSON, and the
body as a JSON string for a text or HTML body. The response status and headers MUST be ignored. When the incoming frame
carried an `id`, the `exception` frame MUST echo it.

#### Scenario: JSON body
- **WHEN** a filter returns `Response::json(json!({ "code": "ROOM_FULL" }))` for a message sent with `"id": "r1"`
- **THEN** the client receives `{ "event": "exception", "data": { "code": "ROOM_FULL" }, "id": "r1" }`

### Requirement: An empty filter response sends nothing
A filter response with an empty body SHALL mean the filter already answered, and the framework MUST NOT send any
frame for that message.

#### Scenario: Filter emits its own event
- **WHEN** a filter calls `client.emit("error", ...)` through `host.switch_to_ws().get_client::<WsClient>()` and
  returns an empty response
- **THEN** the client receives only the `error` frame the filter emitted

### Requirement: Unusable filter responses fall back to the built-in mapping
A filter response with a byte or stream body SHALL be logged as an error naming the filter and answered with the
built-in frame of the original exception. A filter that panics MUST be logged and answered with the built-in
internal-error frame.

#### Scenario: Stream body
- **WHEN** a filter returns a response with a stream body
- **THEN** an error naming the filter is logged and the client receives the built-in `exception` frame of the original
  exception

### Requirement: The filter host is the message's execution context
The host passed to `ExceptionFilter::catch` for a gateway SHALL be the message's execution context: `get_type()`
returns `ContextType::Ws`, and `switch_to_ws()` gives the client and the message data.

#### Scenario: Filter reads the message
- **WHEN** a filter catches an error raised for `{ "event": "join", "data": "lobby" }`
- **THEN** `host.get_type()` is `ContextType::Ws` and `host.switch_to_ws().get_data()` is `"lobby"`

### Requirement: Internal errors are logged only when the built-in mapping handles them
The framework SHALL log internal errors (unknown errors and panics) through `WsExceptionsHandler` when the built-in
mapping produces the frame, and MUST NOT log them when a filter catches the exception.

#### Scenario: Filter catches an internal error
- **WHEN** a handler returns an unknown error and a gateway filter catches it
- **THEN** no `WsExceptionsHandler` error line is logged for it and the client receives the filter's frame
