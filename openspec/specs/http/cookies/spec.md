# http/cookies Specification

## Purpose
Cookie lifetime set through either cookie API (`res.cookie(name, value, CookieOptions)` and
`Response::with_cookie(Cookie)`) uses one unit and produces the same `Max-Age` attribute.

## Requirements

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
