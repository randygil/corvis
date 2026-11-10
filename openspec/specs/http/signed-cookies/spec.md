# http/signed-cookies Specification

## Purpose
Signed cookies: values the client can read but cannot alter, in the format of `cookie-signature` 1.0.7 with the `s:`
prefix that cookie-parser and express-session use, so that cookies signed by a Node app verify in Corvis and the
reverse. Available under the `auth-session` cargo feature. Where this specification is stricter than the original,
the requirement names the divergence.

## Requirements

### Requirement: Cookie values are signed in the cookie-signature format
`CookieKeys::sign(value)` SHALL return `s:` + the value + `.` + the HMAC-SHA256 of the value with the first secret,
encoded in base64 with the standard alphabet and without `=` padding. The result MUST equal what
`'s:' + require('cookie-signature').sign(value, secret)` returns for the same value and secret.

#### Scenario: Same signature as the original
- **WHEN** the recorded values (ASCII, empty, with dots, UTF-8) are signed with the recorded secrets
- **THEN** each result equals the recorded original string

#### Scenario: Written like a signed response cookie
- **WHEN** a handler writes `res.cookie("prefs", &keys.sign("dark"), options)`
- **THEN** the `Set-Cookie` header equals the one recorded for `res.cookie('prefs', 'dark', { signed: true })` with
  the same options

### Requirement: Signed values are verified in constant time
`CookieKeys::unsign(input)` SHALL return the value only when the input starts with `s:` and the part after the last
`.` is the signature of the part before it under one of the secrets; otherwise it SHALL return nothing. The signature
comparison MUST be constant-time and MUST be made against every configured secret without stopping at the first
match. It MUST accept exactly the inputs the original accepts with the same secrets, and it MUST never panic.

#### Scenario: Valid value
- **WHEN** `unsign` receives a string produced by `sign`, or one recorded from the original with the same secret
- **THEN** it returns the original value

#### Scenario: Tampered value or signature
- **WHEN** one character of the value or of the signature is changed, the signature is truncated, or padding is
  added
- **THEN** `unsign` returns nothing

#### Scenario: Unsigned input
- **WHEN** the input has no `s:` prefix or no `.`
- **THEN** `unsign` returns nothing

#### Scenario: Arbitrary input
- **WHEN** a property test passes random strings and random mutations of valid signed values
- **THEN** `unsign` never panics and returns a value only for inputs whose signature verifies

### Requirement: Secrets can be rotated
`CookieKeys` SHALL accept 1 to 8 secrets. The first one MUST sign; every one MUST verify.
`CookieKeys::verified_with_current(input)` SHALL tell whether the first secret verifies the input.

#### Scenario: Value signed with a retired secret
- **WHEN** the keys are `[new, old]` and `unsign` receives a value signed with `old`
- **THEN** it returns the value, `verified_with_current` is `false`, and `sign` of the same value uses `new`

#### Scenario: Secret removed
- **WHEN** the keys are `[new]` and `unsign` receives a value signed with `old`
- **THEN** it returns nothing

### Requirement: Weak or missing secrets are refused
`CookieKeys::new` MUST fail when no secret is given, when more than 8 are given, or when any secret is shorter than
32 bytes. The error MUST name the position and the length of the offending secret and MUST NOT contain it.
`CookieKeys` MUST have a `Debug` output without secret material. Documented divergence: the original accepts any
secret.

#### Scenario: Short secret
- **WHEN** `CookieKeys::new(["short"])` is called
- **THEN** it returns an error that states the minimum of 32 bytes and the length 5, without the secret

#### Scenario: Application does not start
- **WHEN** a provider factory builds `CookieKeys` from a configuration value of 16 bytes and propagates the error
- **THEN** `init()` fails

#### Scenario: Debug output
- **WHEN** a `CookieKeys` value is formatted with `{:?}`
- **THEN** the output contains no secret

### Requirement: Signed input length is bounded
`unsign` MUST return nothing for an input longer than 4096 bytes, without computing a signature.

#### Scenario: Oversized cookie
- **WHEN** `unsign` receives 5000 bytes that start with `s:`
- **THEN** it returns nothing and a test counter of HMAC computations stays at zero

### Requirement: The cookie name is not part of the signature
The signature SHALL cover the value only, as in `cookie-signature`. The docs MUST tell applications to use `CookieKeys`
separate from the session secrets for their own signed cookies, because a value signed for one cookie verifies under
any other name signed with the same secret.

#### Scenario: Value signed for another cookie
- **WHEN** a value is signed with the session secret for a cookie named `prefs`
- **THEN** it verifies with the same keys, whatever the name of the cookie it is sent in (documented, not prevented)

### Requirement: Signed cookies are read from the request
`Request::signed_cookie(name, keys)` SHALL return the verified value of the cookie `name`, read with the parsing rules
of `Request::cookie`, and nothing when the cookie is absent, unsigned or does not verify. The result MUST match
cookie-parser's `req.signedCookies[name]` for string values, where the original reports an invalid signature as
`false` and Corvis as nothing.

#### Scenario: Cookie signed by the original
- **WHEN** a request carries the recorded `Cookie` header produced by the original for `prefs`
- **THEN** `signed_cookie("prefs", &keys)` returns the recorded value

#### Scenario: Unsigned cookie with the same name
- **WHEN** the request carries `prefs=dark`
- **THEN** `signed_cookie("prefs", &keys)` returns nothing, and `cookie("prefs")` still returns `dark`

### Requirement: Signed cookies are verified against the original
Signing and verification SHALL be covered by a differential test: a generator in the fixture package of
`auth/strategies` records signatures and `req.signedCookies` results from `cookie-signature` 1.0.7 and cookie-parser,
and a Rust test replays every case without Node.

#### Scenario: Fixture replay
- **WHEN** `cargo test -p corvis --features auth-session --test signed_cookies` runs on a machine without Node
- **THEN** every recorded signature and verification result is compared
