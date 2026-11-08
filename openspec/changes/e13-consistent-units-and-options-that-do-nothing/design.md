# Design

## Context

See proposal.md for motivation; the behavior is in the two spec deltas. Current state, verified in the code:

**Cookies**

- `CookieOptions` (`crates/corvis/src/http/res.rs:143-163`) documents `max_age` as milliseconds;
  `CookieOptions::max_age(ms)` is at `res.rs:166`. `serialize_cookie` (`res.rs:245-275`) writes
  `Max-Age={ms.div_euclid(1000)}` and an `Expires` computed from `now + ms` (`res.rs:256-259`), like Express 5's
  `res.cookie` plus the `cookie` package. Unit test: `cookies_like_express` (`res.rs:964-973`, `90_500` →
  `Max-Age=90`).
- `Cookie` (`crates/corvis/src/http/response.rs:208-262`), used by `Response::with_cookie` (`response.rs:166`), takes
  `max_age(seconds)` (`response.rs:241`) and `Cookie::try_to_string` writes it unchanged as `Max-Age={m}`
  (`response.rs:282-284`), without `Expires`.
- Test: `crates/corvis/tests/static_response_security.rs:294-297` asserts
  `Cookie::new("sid", "a b")...max_age(60)...` serializes with `Max-Age=60`.
- Docs: `docs/http/cookies.md:70` (`.max_age(ms)` in milliseconds) and `docs/http/cookies.md:89-94` (example with
  `Cookie::max_age(3600)` and the note "`Cookie::max_age` takes **seconds**, while `CookieOptions::max_age` takes
  milliseconds"); same in `docs/es/http/cookies.md`.
- Other `max_age` APIs follow their Node libraries and are out of scope: `CorsOptions::max_age(seconds)`
  (`crates/corvis/src/router/cors.rs:197`) and serve-static's `max_age(ms)` (`crates/corvis/src/http/serve_static.rs:107`).

**ValidationPipe `transform`**

- `ValidationPipeOptions::transform` (`crates/corvis/src/validation/mod.rs:111-113`) is documented in code as
  "Always on in corvis ... kept for API familiarity"; the default is `true` (`mod.rs:126`) and the builder
  `ValidationPipe::transform(bool)` is at `mod.rs:157`.
- `ValidationPipe::transform` (the `PipeTransform` impl, `mod.rs:195-212`) never reads `options.transform`; primitive
  coercion is decided by `coerce_primitives: metadata.kind != ParamKind::Body` (`mod.rs:206`), and handler arguments
  are deserialized with the lenient deserializer anyway (`crates/corvis/src/common/lenient.rs`).
- Docs: `docs/application/validation.md:88` says "Accepted for compatibility"; same in `docs/es/application/validation.md:88`.
- No use of `.transform(true|false)` on `ValidationPipe` or of `ValidationPipeOptions` struct literals exists in
  `crates/`, `examples/` or the three port apps.

## Goals / Non-Goals

**Goals:**
- One unit for cookie lifetime across both cookie APIs, matching Express.
- No option of `ValidationPipe` that silently does nothing.

**Non-Goals:**
- Unifying the two cookie serializers (see proposal, Out of scope).
- Removing deprecated options in this change.

## Decisions

1. **Milliseconds for `Cookie::max_age`.** The framework's reference model is Express, whose `res.cookie` takes
   `maxAge` in milliseconds, and `CookieOptions` already follows it; users migrating code write milliseconds.
   `Cookie::try_to_string` writes `Max-Age={ms.div_euclid(1000)}`, the exact rule of `serialize_cookie`, so both APIs
   agree, including for negative values. The `Cookie::max_age` doc comment and the `max_age` field doc say
   "milliseconds".
   Alternatives considered:
   - Seconds everywhere: contradicts Express and the documented `CookieOptions`, and would silently change every
     `res.cookie` call migrated from Node.
   - `std::time::Duration` in both: unambiguous, but `CookieOptions` intentionally mirrors the Express option and the
     public `max_age: Option<i64>` field; a larger API change than the issue asks. Listed as an open question.
   - Keep seconds and rename (`max_age_secs`): leaves two units in the API, which is the problem.
   The change is silent for existing callers of `Cookie::max_age`. This is acceptable now because the crates are not
   published yet (#10) and no example or port app calls it; the docs page carries a short "changed" note.
2. **Should `Cookie` also write `Expires`?** Not in this change: the issue is about the unit. It is listed in Out of
   scope with the other serializer differences.
3. **Deprecate `transform` instead of removing or implementing it.** Mark both the builder method and the field
   `#[deprecated(note = "values are always converted to the declared type; this option has no effect")]`. The
   framework's own `Default` impl uses `#[allow(deprecated)]` locally so the workspace stays warning-free. The doc
   comment points to the docs section on conversion.
   Alternatives considered:
   - Remove both now: hard compile error for code ported with `transform: true`, which is the most common option in
     the original model; deprecation gives the same information with a gentler path.
   - Implement `transform(false)` by skipping the primitive coercion in validation: handler arguments would still be
     converted by the lenient deserializer, so validation and the handler would disagree (`#[is_int]` failing on
     `"2"` while the handler would accept it). Rejected.

## Risks / Trade-offs

- [Silent unit change for `Cookie::max_age` callers] → the crates are unpublished, there are no known callers outside
  this repository's tests, the docs note the change, and the test in `static_response_security.rs` is updated to the
  new unit.
- [Deprecation warnings break builds with `-D warnings` in user code] → that is the intended signal; the fix is to
  delete the call.

## Open questions

- Should cookie lifetimes move to `std::time::Duration` (in both APIs) in a later change, before 1.0? This change
  keeps milliseconds for Express parity.
- Should the deprecated `transform` option be removed before the first crates.io release (#10) instead of shipping
  deprecated?

## Review notes (2026-11-08)

- `Cookie::max_age` stays an integer in milliseconds (same unit as `CookieOptions` and Express); a `Duration` API is
  not part of this change.
- `transform` is deprecated here; whether deprecated items are removed before the first crates.io release is decided
  in #10.
