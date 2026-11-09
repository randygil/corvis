# Proposal

Implements [randygil/corvis#42](https://github.com/randygil/corvis/issues/42) (epic
[#39](https://github.com/randygil/corvis/issues/39)). Design from the #40 spike
(`openspec/changes/archive/2026-11-08-e3-strategy-module-design/`: decision 2, fact 4, the `auth/jwt-strategy` delta,
task group 2.10-2.12, Review note 7 on `from_cookie`).

**Depends on #136** (`openspec/changes/e3-strategy-core-and-guard-responses/`): this change is written against the API
#136 proposes (`corvis::auth::Strategy`, `StrategyOutcome`, `AuthenticatedUser`, `Provider::strategy`, `AuthGuard`,
the fixture package `crates/corvis/tests/fixtures/auth/`) and is implemented after #136 lands. If #136 changes that
API during its review, this change is updated before work starts.

## Why

JWT is the strategy almost every migrated app uses: `class JwtStrategy extends PassportStrategy(Strategy)` with
`jwtFromRequest`, `secretOrKey`, `validate(payload)`, and `AuthGuard('jwt')` on routes. Corvis has a verified
`JwtService::verify` (`crates/corvis/src/jwt.rs:251-293`, spec `openspec/specs/auth/jwt/spec.md`) but apps wire it by
hand in a guard (`examples/cats-app/src/auth/auth_guard.rs`), with `extract_bearer_token` (`jwt.rs:315-319`), which
parses the header differently from passport-jwt (spike Context: `"Bearer\tabc"` and `"Bearer a b"`). There is no
cookie, query or body extraction, no per-request key, no issuer or audience list, no clock tolerance.

The hand-written path also leaves security decisions to each app. The strategy is the place to make them once: which
algorithms are accepted for which key, how long a token may be, whether `exp` is required, and how much clock skew is
tolerated.

## What Changes

- `corvis::auth::jwt` (under the existing `jwt` cargo feature, which already brings `jsonwebtoken` 9 and is a default
  feature): `JwtValidator` (the user's `validate(req, payload)`, name `"jwt"` overridable with `const NAME`),
  `JwtStrategy<V>` (the adapter registered with `Provider::strategy::<JwtStrategy<V>>()`), `JwtStrategyOptions` with
  the passport-jwt option names (`jwt_from_request`, `secret_or_key`, `secret_or_key_provider`, `issuer`, `audience`,
  `algorithms`, `ignore_expiration`) plus the Corvis limits below, `JwtKey`, `JwtKeyProvider` and `ExtractJwt`.
- `ExtractJwt` with the passport-jwt 4.0.1 extractors: `from_auth_header_as_bearer_token`,
  `from_auth_header_with_scheme`, `from_header`, `from_url_query_parameter`, `from_body_field`, `from_extractors`, plus
  `from_cookie` (Review note 7) and `custom`. There is no default extractor: a strategy without one does not start,
  as passport-jwt throws without `jwtFromRequest`; tokens are read only from the places configured.
- Security limits, each with a default, a startup check and a scenario:
  - algorithm allow-list: explicit `algorithms` or, by default, derived from the key type as node-jsonwebtoken 9 does
    (HMAC secret: HS256/384/512; RSA: RS* and PS*; EC: ES256/ES384); `none` is never accepted; an allow-list that
    mixes key families with a static key, or HMAC algorithms with a public key, does not start; Ed25519 keys need an
    explicit `algorithms([EdDSA])`;
  - `exp` required by default (divergence: the original accepts tokens without `exp` forever), with an explicit
    `allow_missing_exp()` that logs a startup warning;
  - `clock_tolerance` default 0 s (the original's), at most 300 s;
  - `max_token_length` default 8192 bytes, checked before any decoding;
  - issuer and audience lists with node-jsonwebtoken semantics; empty values do not start;
  - a startup warning for HMAC secrets shorter than 32 bytes, for `ignore_expiration`, and for the query parameter
    extractor (tokens in URLs end up in access logs).
- A key provider (`secret_or_key_provider`) called per request with the request and the raw token; its error makes the
  strategy fail (401), and the key it returns narrows the allowed algorithms to that key's family.
- Verification reuses the core of `JwtService::verify` through an internal function; `JwtService::verify`,
  `verify_or_unauthorized` and `extract_bearer_token` keep their behavior (spec `auth/jwt` unchanged).
- Differential test against `passport-jwt` 4.0.1 (with its `jsonwebtoken` 9.0.x): `jwt.mjs` in the fixture package of
  #136 records the cases, `crates/corvis/tests/auth_jwt.rs` replays them; divergences are marked.
- Docs (EN + ES): JWT section of `docs/recipes/passport.md`; `docs/security/authentication.md` uses
  `AuthGuard::new("jwt")` with the hand-written guard kept as an alternative. Example: `examples/cats-app` replaces its
  hand-written `AuthGuard` with `JwtStrategy` and `AuthGuard::new("jwt").skip_public()`.

## Capabilities

### New Capabilities

- `auth/jwt-strategy`: JWT strategy, extractors, keys and key provider, algorithm allow-list, claim checks, limits,
  startup checks, differential verification.

### Modified Capabilities

None. `auth/jwt` (`openspec/specs/auth/jwt/spec.md`) is unchanged and its tests run as regression tests after the
refactor of the verification core.

## Impact

- New `crates/corvis/src/auth/jwt.rs` (or `auth/jwt/` with `extract.rs`, `options.rs`), `#[cfg(feature = "jwt")]`.
- `crates/corvis/src/jwt.rs`: the verification of lines 251-293 moves into a `pub(crate)` function that both
  `JwtService::verify` and the strategy call; no public change.
- `crates/corvis/tests/fixtures/auth/`: `passport-jwt` 4.0.1 added to `package.json` and the lockfile; `jwt.mjs`,
  `jwt.json`, fixed test keys (the existing `crates/corvis/tests/fixtures/jwt_rsa_private.pem` / `jwt_rsa_public.pem`
  are reused; an EC P-256 pair is added).
- Tests: `crates/corvis/tests/auth_jwt.rs`, unit tests in the new module, a proptest for the header parser.
- Docs: `docs/recipes/passport.md`, `docs/security/authentication.md` and their `docs/es/` copies.
- Example: `examples/cats-app/src/auth/` (guard, module), its tests.
- Port apps: they use `JwtService` (through hand-written guards), whose code is refactored, so the corvis-parity check
  is required.

## Out of scope

- Upgrading `jsonwebtoken` from 9 to a newer major (spike Out of scope): possible new issue; it would add ES512.
- ES512 tokens: `jsonwebtoken` 9 has no ES512 (`crates/corvis/src/jwt.rs:167` and `179` list ES256 and ES384 only),
  so they always fail; documented.
- JWKS fetching and caching (`jwks-rsa`'s `passportJwtSecret`): a key provider can implement it; a built-in one is a
  possible new issue.
- `jsonWebTokenOptions` other than issuer, audience, algorithms, `ignoreExpiration` and `clockTolerance`: `maxAge`,
  `subject`, `jwtid`, `nonce`, `complete`, `clockTimestamp`, regular-expression audiences. Possible new issue if a
  migrated app needs one.
- Changing `extract_bearer_token` to the passport-jwt parsing (spike Out of scope): it keeps its behavior.
- `AuthGuard` on WebSocket gateways (fail closed since #136).
- Token revocation lists and refresh-token rotation: application logic, documented as a pattern at most.
