# Spec Delta

## ADDED Requirements

### Requirement: A guard can end the request with a response
A guard SHALL be able to store a response on the request (`Request::respond_with`). After each guard returns, the
router MUST check for a stored response and, if there is one, send it without running later guards, interceptors,
pipes or the handler, and without passing it through exception filters. This MUST work for route, controller and
global guards.

#### Scenario: Guard short-circuits with a redirect
- **WHEN** the first of two route guards stores a 302 response with `Location: /login` and returns `Ok(true)`
- **THEN** the client receives that 302 response, the second guard is not run and the handler is not called

#### Scenario: Exception filters are not involved
- **WHEN** a guard stores a response and a catch-all exception filter is installed
- **THEN** the stored response is sent unchanged and the filter is not called

### Requirement: Response headers added before the handler are kept
Code running before the handler (guards and middleware) SHALL be able to append response headers to the request
(`Request::append_response_header`). The router MUST merge them into the response finally sent, whether it comes
from the handler, from an exception filter or from a stored guard response, appending to existing values for
`Set-Cookie` and replacing them for other headers, like `res.setHeader` before `next()` in Express.

#### Scenario: Cookie set by a guard survives an exception
- **WHEN** a guard appends `Set-Cookie: sid=1; Path=/; HttpOnly` and the handler then throws
  `UnauthorizedException`
- **THEN** the 401 response carries `Set-Cookie: sid=1; Path=/; HttpOnly`, like the recorded Express response of the
  same flow

#### Scenario: Two cookies
- **WHEN** a guard appends two `Set-Cookie` headers and the handler sets a third through `#[res]`
- **THEN** the response carries all three `Set-Cookie` headers

### Requirement: Existing guard behavior is unchanged
A guard that does not use these APIs MUST behave exactly as before: `Ok(false)` answers 403 with
`{"message":"Forbidden resource","error":"Forbidden","statusCode":403}` and `Err(e)` goes to the exception filters.
The three port apps MUST keep 0 apidiff differences.

#### Scenario: Plain guard denies
- **WHEN** a guard returns `Ok(false)`
- **THEN** the response is the same 403 as before this change
