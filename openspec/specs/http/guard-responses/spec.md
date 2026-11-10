# http/guard-responses Specification

## Purpose
What a guard can do besides allowing or denying a request: end it with its own response (the equivalent of a
passport strategy writing a redirect to `res`), and append response headers that survive the handler, exception
filters and `#[res]` (the equivalent of `res.cookie` / `res.append` before `next()` in Express). Both are reached
through `ctx.switch_to_http()`, next to `set_response_header` of `core/enhancers`.

## Requirements

### Requirement: A guard can end the request with a response
The framework SHALL provide `HttpArgumentsHost::respond_with(Response) -> bool`. In an HTTP context it MUST store the
response for the current request and return `true`. After each guard returns, the router MUST check for a stored
response and, if there is one, send it without running later guards, interceptors, pipes or the handler, and without
passing it through exception filters or the route's `#[header]` and redirect settings. This MUST work for global,
controller and route guards. It MUST be valid only while a guard runs: called from an interceptor, a handler or a
context the router did not build for a guard, it MUST store nothing and return `false`.

#### Scenario: Guard short-circuits with a redirect
- **WHEN** the first of two route guards stores a 302 response with `Location: /login` and returns `Ok(true)`
- **THEN** the client receives that 302 response, the second guard is not run and the handler is not called

#### Scenario: Exception filters are not involved
- **WHEN** a guard stores a response and a catch-all exception filter is installed
- **THEN** the stored response is sent unchanged and the filter is not called

#### Scenario: Global guard
- **WHEN** a global guard stores a 302 response
- **THEN** no controller or route guard runs and the client receives the 302 response

### Requirement: The stored response wins over the guard's result
Once a guard stored a response, the router MUST send it whatever the guard returns. When the guard returned
`Ok(false)` or an error after storing a response, the router SHALL log that the result was discarded (as Express
logs `ERR_HTTP_HEADERS_SENT` when an exception follows an ended response) and MUST NOT send a 403 or pass the error
to the exception filters. A second call to `respond_with` for the same request MUST replace the stored response.

#### Scenario: Guard stores a response and then fails
- **WHEN** a guard stores a 302 response and returns `Err(UnauthorizedException::empty())`
- **THEN** the client receives the 302 response and one error is logged

### Requirement: A stored response carries the recorded headers
A response stored with `respond_with` SHALL receive the headers recorded with `set_response_header` and
`append_response_header` before it was sent, by the same merge rules as any other response of the route, and the
application's CORS headers like any other response.

#### Scenario: Rate limit headers on a redirect
- **WHEN** a first guard calls `set_response_header("X-RateLimit-Limit", "10")` and a second guard stores a 302
  response
- **THEN** the 302 response carries `X-RateLimit-Limit: 10`

### Requirement: Response headers can be appended
The framework SHALL provide `HttpArgumentsHost::append_response_header(name, value) -> bool` for guards, interceptors
and pipes. Unlike `set_response_header`, values recorded for the same name MUST accumulate in call order.

#### Scenario: Two appended values
- **WHEN** a guard appends `Link: </a>; rel=preload` and then `Link: </b>; rel=preload`, and the handler returns a JSON
  body
- **THEN** the response carries both `Link` values in that order

### Requirement: Appended headers are validated and bounded
`append_response_header` MUST validate the name and value with the rules of `set_response_header` (no CR, LF or other
characters Node refuses; no framing or connection headers) and return `false`, recording nothing, for an invalid
header. A request MUST NOT record more than 128 appended values in total: further calls return `false` and record
nothing. In a WebSocket context the call MUST record nothing and return `false`.

#### Scenario: Header injection attempt
- **WHEN** a guard calls `append_response_header("Set-Cookie", "a=1\r\nX-Evil: 1")`
- **THEN** the call returns `false` and the response carries neither header

#### Scenario: Value limit
- **WHEN** a guard appends 129 values to `X-Trace`
- **THEN** the first 128 calls return `true`, the 129th returns `false`, and the response carries 128 `X-Trace`
  values

#### Scenario: Gateway message
- **WHEN** a guard running on a gateway message calls `append_response_header("Set-Cookie", "a=1")`
- **THEN** the call returns `false` and the message is handled normally

### Requirement: Appended cookies are kept next to the response's own cookies
The router SHALL add every recorded `Set-Cookie` value to the final response, whether it comes from the handler, from
the built-in exception handling, from an exception filter, from a `#[res]` handler or from a stored guard response,
even when that response has `Set-Cookie` values of its own. Recorded values MUST come before the response's own
values, the order Express produces when a guard's `res.cookie` runs before the handler's.

#### Scenario: Cookie set by a guard survives an exception
- **WHEN** a guard appends `Set-Cookie: sid=1; Path=/; HttpOnly` and the handler then returns
  `Err(UnauthorizedException::empty())`
- **THEN** the 401 response carries `Set-Cookie: sid=1; Path=/; HttpOnly`, as Express keeps a cookie set on `res`
  before an exception (recorded in `crates/corvis/tests/fixtures/res_express.json`, case `POST login-fail`)

#### Scenario: Guard cookies and handler cookie
- **WHEN** a guard appends two `Set-Cookie` values and the handler returns a `Response` with a third `Set-Cookie`
- **THEN** the response carries the three values, the guard's two first

#### Scenario: `#[res]` handler adds a cookie
- **WHEN** a guard appends `Set-Cookie: a=1` and a `#[res]` handler calls `res.cookie("b", "2")` and sends
- **THEN** the response carries `Set-Cookie: a=1` and then the cookie `b=2`

### Requirement: Other appended headers never replace the response's own
For any name other than `Set-Cookie`, recorded values SHALL be added to the final response only when it has no header
with that name, as `set_response_header` values are; when a name was both set and appended, the appended values come
after the set value.

#### Scenario: Handler header wins
- **WHEN** a guard appends `X-Trace: guard` and the handler returns `Response::json(..).with_header("X-Trace", "mine")`
- **THEN** the response carries only `X-Trace: mine`

### Requirement: Existing guard behavior is unchanged
A guard that does not use these APIs MUST behave exactly as before this change: `Ok(false)` answers 403 with
`{"message":"Forbidden resource","error":"Forbidden","statusCode":403}`, `Err(e)` goes to the exception filters, and
`set_response_header` keeps the requirements of `core/enhancers`. The three port apps MUST keep 0 apidiff
differences.

#### Scenario: Plain guard denies
- **WHEN** a guard returns `Ok(false)`
- **THEN** the response is the same 403 as before this change
