# Tasks

Prerequisite: #136 (`e3-strategy-core-and-guard-responses`) is merged into `dev`. Before starting, compare this
change with the API that landed (`corvis::auth`, `Strategy`, `Provider::strategy`, fixture package) and update the
artifacts if it differs.

## 1. Shared verification core

- [x] 1.1 Move the body of `JwtService::verify` (`crates/corvis/src/jwt.rs:259-291`) into `pub(crate) fn
  verify_claims(token, key, validation, TimeRules)` (design decision 4) and make `JwtService::verify` call it with its
  current settings; verify that every existing test in `jwt.rs` and the `auth/jwt` fuzz tests
  (`crates/corvis/src/fuzz_tests.rs`) pass unchanged
- [x] 1.2 Add clock tolerance, `require_exp` and `ignore_expiration` to `TimeRules` with the node-jsonwebtoken formulas;
  verify with unit tests at the boundaries (`now == exp + tolerance` fails, one second before passes; `nbf == now +
  tolerance` passes, one second after fails), using a fixed `now` passed in, not the clock

## 2. Options, keys and startup checks

- [x] 2.1 `JwtKey` (secret, RSA, EC, Ed25519 PEM), `JwtKeyProvider`, `JwtStrategyOptions` with the passport-jwt
  names and the Corvis limits (design decisions 2 and 6); verify with unit tests for builders and PEM parsing errors
- [x] 2.2 Startup validation (no extractor, key and provider, neither, empty secret, bad PEM, empty or mixed-family
  `algorithms` with a static key, Ed25519 without `algorithms`, empty issuer/audience, tolerance over 300 s,
  `max_token_length(0)`) and the warnings (short secret, `ignore_expiration`, `allow_missing_exp`, query extractor);
  verify with one test per case of the "Extractor and key options are checked at startup", "Verification options are
  checked at startup" (including "Empty audience") and "Weak or risky configurations" requirements
- [x] 2.3 Allow-list computation per key family and per request for key providers (design decision 5); verify with
  unit tests for "Algorithm none", "HMAC token against an RSA key", "Explicit allow-list" and "Provider returns an RSA
  key for an HMAC token"

## 3. Extractors

- [x] 3.1 Header parser `(\S+)\s+(\S+)` on latin1 values with JavaScript whitespace, first `Authorization` value only;
  `from_auth_header_as_bearer_token`, `from_auth_header_with_scheme`; verify with unit tests for tab, NBSP (`0xA0`),
  extra words, lowercase scheme, other scheme, empty value, and that `extract_bearer_token` is unchanged
- [x] 3.2 `from_header` (joined values, lowercased name), `from_url_query_parameter`, `from_body_field`,
  `from_cookie`, `from_extractors`, `custom`; non-string and empty values as no token; verify with unit tests per
  extractor, including "Non-string body field", "Uppercase header name" and "Repeated custom header"
- [x] 3.3 Proptest: random byte strings as `Authorization`, cookie, query and body values never panic in extraction or
  verification; verify it runs in `cargo test` with the default case count

## 4. Strategy and validator

- [x] 4.1 `JwtValidator` and `JwtStrategy<V>` (`Injectable` + `Strategy`): length check, extraction, key, verification,
  issuer/audience with node semantics, payload must be an object, validator call, outcome mapping, failure reasons
  (design decision 7); verify with HTTP tests through `corvis::testing::request` for "Payload passed through", "Two JWT
  strategies", "Validator returns no user", "Oversized token", "Raised limit", "Token without exp", "Opt-out",
  "Audience list", "Audience array in the token", "Wrong issuer", "Key chosen by the token's issuer", "Key provider
  error", "String payload" and "Token in a place that is not configured"
- [x] 4.2 Time-relative tests with margins of at least 20 seconds ("Expired token", "Expired within the tolerance",
  "Not yet valid", "ignore_expiration", "Non-numeric exp", "String iat", "Tolerance above the limit"); verify no
  assertion measures elapsed time

## 5. Differential fixture

- [x] 5.1 Add `passport-jwt` 4.0.1 and `cookie-parser` (exact version) to `crates/corvis/tests/fixtures/auth/package.json`
  and the lockfile; add an EC P-256 test key pair next to `crates/corvis/tests/fixtures/jwt_rsa_public.pem`; verify
  `npm ci`
- [x] 5.2 `jwt.mjs` with the strategies and cases of design decision 8 (fixed tokens and time templates), writing
  `jwt.json` with divergences marked; verify two runs give the same output apart from nothing time-dependent
- [x] 5.3 `crates/corvis/tests/auth_jwt.rs` replaying `jwt.json` (status, headers, body; templates re-signed; divergent
  cases asserted against the spec); verify `cargo test -p corvis --test auth_jwt` passes and that editing one recorded
  status makes it fail

## 6. Documentation and example

- [x] 6.1 `docs/recipes/passport.md`: JWT strategy section (validator, options table mapped from passport-jwt,
  extractors, keys and key provider, limits and defaults, the divergences: `exp` required, `iat` type, string payload,
  header name case, token length, ES512) and `docs/security/authentication.md`: `AuthGuard::new("jwt")` as the main
  path, the hand-written guard kept as the alternative, warnings about query tokens and short secrets; same in
  `docs/es/`; verify `npm run docs:check` in `docs/`
- [x] 6.2 `examples/cats-app`: replace the hand-written guard with `JwtStrategy` and `AuthGuard::new("jwt").skip_public()`
  (design decision 9); verify the example builds, its tests pass (`posture.rs`, `startup.rs`, `login_throttle.rs`), and
  it runs on 127.0.0.1

## 7. Definition of done

- [x] 7.1 `cargo build --workspace --all-targets` with no warnings
- [x] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml` passes (`jwt` alone and `--no-default-features`, which must compile without the strategy)
- [x] 7.3 `cargo test --workspace` passes, including `auth_jwt.rs` without Node
- [x] 7.4 Regression tests for every scenario of `auth/jwt-strategy`; the `auth/jwt` tests still pass
- [x] 7.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 7.6 Framework runtime code changed (`JwtService::verify` refactor): run the corvis-parity check; verify the three
  port apps keep 0 apidiff differences
- [ ] 7.7 Run the security-reviewer agent over `corvis::auth::jwt` and the refactored `JwtService::verify` (algorithm
  confusion, `none`, oversized and malformed tokens, header parsing, key provider errors); verify every finding is
  fixed or recorded as a new issue

## 8. Security review (round 1)

- [x] 8.1 `JwtService::verify`: apply `issuer_matches` and `audience_matches` and keep `jsonwebtoken` errors downcastable;
  verify the unit tests (missing, numeric and list `iss`; missing and numeric `aud`; correct; unconfigured)
- [x] 8.2 `CanActivate::inner_guards`, `check_routes` follows it, cats-app wrapper implements it; verify a wrapper with an
  unknown name refuses to start
- [x] 8.3 `crit` header and three-segment checks in the strategy (before the key provider) and `crit` in `verify_claims`;
  verify the key provider is not called
- [x] 8.4 Docs EN/ES (provider errors, extractor order, cookie CSRF, `typ`, `JwtService` strictness, composite guards) and
  spec deltas; verify `openspec validate e3-jwt-strategy --strict` and `npm run docs:check`
- [ ] 8.5 Run the corvis-parity check and a new security review round (maintainer's session)
- [x] 8.6 Round 2: `check_header` fails closed (malformed header and `crit`) in the strategy and `verify_claims`, with the
  five bypass headers tested; verify the key provider is not called
- [x] 8.7 Round 2: cap guard nesting at 32 in `auth_guards_in` with a startup error naming the route, and a test
- [x] 8.8 Round 2: docs EN/ES (`aud` wording, `Display` of verification errors) and spec deltas
- [x] 8.9 Round 3: `check_header` and `JwtService::decode` parse untrusted bytes with a plain visitor, not
  `serde_json::Value`; tests with `raw_value` on (nested token on a 2 MiB stack, feature-dependent header)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #42, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).
