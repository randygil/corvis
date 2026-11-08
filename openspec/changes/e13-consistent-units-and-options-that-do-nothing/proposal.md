# Proposal

Implements [randygil/corvis#18](https://github.com/randygil/corvis/issues/18) (epic [#13](https://github.com/randygil/corvis/issues/13)).

## Why

The docs review found two APIs that mislead the user without any error:

- The two ways of setting a cookie take `max_age` in different units: `Cookie::max_age` (for `Response::with_cookie`)
  takes seconds, `CookieOptions::max_age` (for `res.cookie(...)`) takes milliseconds like Express. The same number
  gives a cookie that lives 1,000 times longer or shorter depending on which API is used, and the docs need a note to
  warn about it.
- `ValidationPipeOptions::transform` (and `ValidationPipe::transform(bool)`) is accepted and ignored: values are
  always converted to the declared Rust type. A user who writes `.transform(false)` expecting the raw value gets no
  hint that nothing changed.

Both are cheap to fix now, before the crates are published (issue #10), when changing a unit cannot surprise any
external user.

## What Changes

- `Cookie::max_age` takes **milliseconds**, like `CookieOptions::max_age` and Express' `res.cookie`; `Max-Age` is
  written as the whole number of seconds (rounded down), the same rule `serialize_cookie` already uses.
  **Behavior change:** `Cookie::new(..).max_age(3600)` now means 3.6 seconds; write `max_age(3_600_000)`.
- `ValidationPipe::transform(bool)` and the `ValidationPipeOptions::transform` field are marked `#[deprecated]` with
  a note saying the conversion to the declared type always happens; they keep compiling, so existing code gets a
  warning instead of an error.
- Docs (EN/ES): cookies page states one unit for both APIs and drops the "seconds vs milliseconds" note; the
  validation page marks `transform(bool)` as deprecated.

## Capabilities

### New Capabilities

- `http/cookies`: the unit of `max_age` in both cookie APIs and the resulting `Max-Age` attribute.
- `core/validation-pipe`: options of the built-in validation pipe that have no effect are flagged at compile time.

### Modified Capabilities

None (no spec covers these areas yet).

## Impact

- `crates/corvis/src/http/response.rs` (`Cookie::max_age`, `Cookie::try_to_string`) and its test in
  `crates/corvis/tests/static_response_security.rs`.
- `crates/corvis/src/validation/mod.rs` (`ValidationPipeOptions`, `ValidationPipe::transform`).
- Docs: `docs/http/cookies.md`, `docs/application/validation.md` and `docs/es/` translations.
- Port apps: none of the three uses `Cookie::max_age` or `ValidationPipe::transform` (their `.max_age(86_400)` is
  `CorsOptions::max_age`, in seconds like the `cors` package, and is not touched).

## Out of scope

- Unifying `Cookie::try_to_string` with `serialize_cookie`: they still differ in `Expires` (only `CookieOptions`
  writes it), value encoding (`NON_ALPHANUMERIC` vs `encodeURIComponent`) and `SameSite` capitalization; possible new issue.
- Removing the deprecated `transform` option (a later release, once the crates are published with the deprecation).
- Other `max_age` units: `CorsOptions::max_age` (seconds, like `cors`) and `ServeStaticOptions::max_age`
  (milliseconds, like `serve-static`) already follow their Node libraries and are documented as such.
- Making `transform(false)` meaningful (handing the handler the raw value): not possible with typed handler arguments.
