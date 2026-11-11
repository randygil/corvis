# Spec Delta

## Purpose

Cookies set through either cookie API (`res.cookie(name, value, CookieOptions)` and `Response::with_cookie(Cookie)`)
produce the same `Set-Cookie` header, the one the original `res.cookie` produces, and an invalid cookie is an error the
caller handles, never a panic.

## MODIFIED Requirements

### Requirement: Cookie max age is given in milliseconds in every API
`CookieOptions::max_age(ms)` and `Cookie::max_age(ms)` SHALL both take the lifetime in milliseconds, matching
Express' `res.cookie` `maxAge` option. The `Max-Age` attribute MUST be the number of whole seconds, rounded down
(`floor(ms / 1000)`, also for negative values).

#### Scenario: Same value, same attribute
- **WHEN** a handler sets `CookieOptions::new().max_age(90_500)` through `res.cookie("m", "1", ..)` and another
  returns `Response::text("ok").with_cookie(Cookie::new("m", "1").max_age(90_500))`
- **THEN** both `Set-Cookie` headers contain `Max-Age=90`

#### Scenario: One hour
- **WHEN** a handler returns a response with `Cookie::new("theme", "dark").max_age(3_600_000)`
- **THEN** the `Set-Cookie` header contains `Max-Age=3600`

#### Scenario: Negative value expires the cookie
- **WHEN** a handler returns a response with `Cookie::new("sid", "").max_age(-1)`
- **THEN** the `Set-Cookie` header contains `Max-Age=-1`

#### Scenario: Expires accompanies Max-Age
- **WHEN** a cookie is set with a `max_age` through either API
- **THEN** the header also carries `Expires` at the response time plus that lifetime, as the original does

## ADDED Requirements

### Requirement: Both cookie APIs serialize like the original
`serialize_cookie`, `Cookie::try_to_string`, `Response::with_cookie` and `Res::cookie` SHALL produce the `Set-Cookie`
value that the original `res.cookie` produces for the same name, value and options: the same value encoding, attribute
order, attribute spelling and errors. A fixture recorded from the original packages MUST be replayed, every case matching
or carrying a recorded reason.

#### Scenario: Value with characters encodeURIComponent keeps
- **WHEN** a cookie `a` with the value `x-y_z.1!~*'()` is set through `Response::with_cookie` and through `res.cookie`
- **THEN** both headers start with `a=x-y_z.1!~*'()`, as recorded from the original

#### Scenario: SameSite spelling
- **WHEN** a cookie is set with `SameSite::Lax` through either API
- **THEN** the header carries `SameSite=Lax`

#### Scenario: Fixture replay
- **WHEN** the cookie cases of the recorded fixture are replayed through both APIs
- **THEN** each produces the recorded header or the recorded error, except the cases listed as divergences with a reason

### Requirement: Cookie errors are values
An invalid cookie name, value, domain or path SHALL be reported as a `CookieError` whose text is the original
`TypeError` message, returned by `serialize_cookie`, `Cookie::try_to_string`, `Response::with_cookie`,
`Response::set_cookie` and `Res::cookie`. No cookie API MUST panic, and no invalid cookie MUST reach the response.

#### Scenario: Attribute injection through the path
- **WHEN** `Cookie::new("sid", "abc").path("/; Domain=evil.example")` is passed to `Response::with_cookie`
- **THEN** it returns `Err` with the text `TypeError: option path is invalid` and no header is added

#### Scenario: Handler that propagates the error
- **WHEN** a handler calls `res.cookie("sid", "abc", options)?` with an invalid domain
- **THEN** the client receives `500` with the generic internal-error body and no `Set-Cookie` header

#### Scenario: Printing a cookie
- **WHEN** application code formats an invalid `Cookie` with `{:?}`
- **THEN** the debug text is written and nothing panics
