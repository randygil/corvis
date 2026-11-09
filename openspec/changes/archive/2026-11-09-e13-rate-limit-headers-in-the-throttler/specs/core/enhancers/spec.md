# Spec Delta

## Purpose

Response headers set by enhancers (guards, interceptors, pipes) through the execution context, and how they are
merged into the response of an HTTP route.

## ADDED Requirements

### Requirement: Enhancers can set response headers through the execution context
The framework SHALL provide `HttpArgumentsHost::set_response_header(name, value) -> bool`, reached through
`ctx.switch_to_http()`, for guards, interceptors and pipes that receive the route's execution context. It MUST record
the header for the current request and return `true`; a later call with the same name MUST replace the recorded value.

#### Scenario: Guard sets a header on a successful request
- **WHEN** a guard calls `set_response_header("X-Trace", "abc")` and returns `true`, and the handler returns a JSON body
- **THEN** the response carries `X-Trace: abc`

### Requirement: Invalid headers are rejected
`set_response_header` SHALL reject a name or value that is not a valid HTTP header, including values containing CR or
LF: it MUST return `false` and record nothing.

#### Scenario: Header injection attempt
- **WHEN** a guard calls `set_response_header("X-Trace", "a\r\nSet-Cookie: x=1")`
- **THEN** the call returns `false` and the response carries neither `X-Trace` nor the injected `Set-Cookie`

### Requirement: Setting a response header in a WebSocket context does nothing
In a WebSocket context `set_response_header` SHALL record nothing and return `false`.

#### Scenario: Gateway message
- **WHEN** a guard running on a gateway message calls `set_response_header("X-Trace", "abc")`
- **THEN** the call returns `false` and the message is handled normally

### Requirement: Enhancer headers never replace headers the response already has
The framework SHALL add each recorded header to the final response only when the response has no header with that
name. Headers set by the handler, by a returned `Response`, by `#[header]` on the route, by an exception filter, or on
`Res` by a `#[res]` handler MUST win over enhancer headers.

#### Scenario: Route header wins
- **WHEN** a route declares `#[header("X-Trace", "route")]` and a guard sets `X-Trace: guard`
- **THEN** the response carries `X-Trace: route`

#### Scenario: Handler header wins
- **WHEN** a guard sets `X-Trace: guard` and the handler returns `Response::json(..).with_header("X-Trace", "handler")`
- **THEN** the response carries `X-Trace: handler`

### Requirement: Enhancer headers are kept on successful and error responses
The framework SHALL merge the recorded headers into the route's response on success, on an error handled by the
built-in filter, and on an error handled by an exception filter (route, controller or global), including headers
recorded by an interceptor after the handler ran.

#### Scenario: Error without filter
- **WHEN** a guard sets `X-Trace: abc` and the handler returns `Err(NotFoundException::new("x"))`
- **THEN** the 404 response carries `X-Trace: abc`

#### Scenario: Error handled by a filter
- **WHEN** a guard sets `X-Trace: abc` and `X-Other: 1`, the handler fails, and a filter returns a response with
  `X-Trace: filter`
- **THEN** the response carries `X-Trace: filter` and `X-Other: 1`

#### Scenario: Interceptor sets a header after the handler
- **WHEN** an interceptor calls `set_response_header("X-Elapsed", "5")` after `next.handle().await`
- **THEN** the response carries `X-Elapsed: 5`

### Requirement: Enhancer headers reach `#[res]` routes
For routes with a `#[res]` parameter, headers recorded before the `Res` is created SHALL be copied into it after the
route's `#[header]` values, and headers recorded afterwards MUST be written to the `Res` when it has no header with
that name and its headers were not sent yet. Headers recorded after the response was sent MUST be ignored.

#### Scenario: `#[res]` passthrough
- **WHEN** a route with a `#[res(passthrough)] res: Res` parameter returns a value and a guard set `X-Trace: abc`
- **THEN** the response carries `X-Trace: abc`

#### Scenario: `#[res]` handled by the handler
- **WHEN** a route with a `#[res] res: Res` parameter calls `res.send("ok")` and a guard set `X-Trace: abc`
- **THEN** the response carries `X-Trace: abc`, and a handler that called `res.set("X-Trace", "mine")` before sending
  gets `X-Trace: mine`
