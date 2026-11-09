# Tasks

Prerequisite: #136 (`e3-strategy-core-and-guard-responses`) is merged into `dev`. Before starting, compare this
change with the API that landed (`corvis::auth`, `Strategy`, `Provider::strategy`, fixture package) and update the
artifacts if it differs.

## 1. Shared verification core

- [ ] 1.1 Move the body of `JwtService::verify` (`crates/corvis/src/jwt.rs:259-291`) into `pub(crate) fn
  verify_claims(token, key, validation, TimeRules)` (design decision 4) and make `JwtService::verify` call it with its
  current settings; verify that every existing test in `jwt.rs` and the `auth/jwt` fuzz tests
  (`crates/corvis/src/fuzz_tests.rs`) pass unchanged
- [ ] 1.2 Add clock tolerance, `require_exp` and `ignore_expiration` to `TimeRules` with the node-jsonwebtoken formulas;
  verify with unit tests at the boundaries (`now == exp + tolerance` fails, one second before passes; `nbf == now +
  tolerance` passes, one second after fails), using a fixed `now` passed in, not the clock

## 2. Options, keys and startup checks

- [ ] 2.1 `JwtKey` (secret, RSA, EC, Ed25519 PEM), `JwtKeyProvider`, `JwtStrategyOptions` with the passport-jwt
  names and the Corvis limits (design decisions 2 and 6); verify with unit tests for builders and PEM parsing errors
- [ ] 2.2 Startup validation (no extractor, key and provider, neither, empty secret, bad PEM, empty or mixed-family
  `algorithms` with a static key, Ed25519 without `algorithms`, empty issuer/audience, tolerance over 300 s,
  `max_token_length(0)`) and the warnings (short secret, `ignore_expiration`, `allow_missing_exp`, query extractor);
  verify with one test per case of the "Extractor and key options are checked at startup", "Verification options are
  checked at startup" (including "Empty audience") and "Weak or risky configurations" requirements
- [ ] 2.3 Allow-list computation per key family and per request for key providers (design decision 5); verify with
  unit tests for "Algorithm none", "HMAC token against an RSA key", "Explicit allow-list" and "Provider returns an RSA
  key for an HMAC token"

## 3. Extractors

- [ ] 3.1 Header parser `(\S+)\s+(\S+)` on latin1 values with JavaScript whitespace, first `Authorization` value only;
  `from_auth_header_as_bearer_token`, `from_auth_header_with_scheme`; verify with unit tests for tab, NBSP (`0xA0`),
  extra words, lowercase scheme, other scheme, empty value, and that `extract_bearer_token` is unchanged
- [ ] 3.2 `from_header` (joined values, lowercased name), `from_url_query_parameter`, `from_body_field`,
  `from_cookie`, `from_extractors`, `custom`; non-string and empty values as no token; verify with unit tests per
  extractor, including "Non-string body field", "Uppercase header name" and "Repeated custom header"
- [ ] 3.3 Proptest: random byte strings as `Authorization`, cookie, query and body values never panic in extraction or
  verification; verify it runs in `cargo test` with the default case count

## 4. Strategy and validator

- [ ] 4.1 `JwtValidator` and `JwtStrategy<V>` (`Injectable` + `Strategy`): length check, extraction, key, verification,
  issuer/audience with node semantics, payload must be an object, validator call, outcome mapping, failure reasons
  (design decision 7); verify with HTTP tests through `corvis::testing::request` for "Payload passed through", "Two JWT
  strategies", "Validator returns no user", "Oversized token", "Raised limit", "Token without exp", "Opt-out",
  "Audience list", "Audience array in the token", "Wrong issuer", "Key chosen by the token's issuer", "Key provider
  error", "String payload" and "Token in a place that is not configured"
- [ ] 4.2 Time-relative tests with margins of at least 20 seconds ("Expired token", "Expired within the tolerance",
  "Not yet valid", "ignore_expiration", "Non-numeric exp", "String iat", "Tolerance above the limit"); verify no
  assertion measures elapsed time

## 5. Differential fixture

- [ ] 5.1 Add `passport-jwt` 4.0.1 and `cookie-parser` (exact version) to `crates/corvis/tests/fixtures/auth/package.json`
  and the lockfile; add an EC P-256 test key pair next to `crates/corvis/tests/fixtures/jwt_rsa_public.pem`; verify
  `npm ci`
- [ ] 5.2 `jwt.mjs` with the strategies and cases of design decision 8 (fixed tokens and time templates), writing
  `jwt.json` with divergences marked; verify two runs give the same output apart from nothing time-dependent
- [ ] 5.3 `crates/corvis/tests/auth_jwt.rs` replaying `jwt.json` (status, headers, body; templates re-signed; divergent
  cases asserted against the spec); verify `cargo test -p corvis --test auth_jwt` passes and that editing one recorded
  status makes it fail

## 6. Documentation and example

- [ ] 6.1 `docs/recipes/passport.md`: JWT strategy section (validator, options table mapped from passport-jwt,
  extractors, keys and key provider, limits and defaults, the divergences: `exp` required, `iat` type, string payload,
  header name case, token length, ES512) and `docs/security/authentication.md`: `AuthGuard::new("jwt")` as the main
  path, the hand-written guard kept as the alternative, warnings about query tokens and short secrets; same in
  `docs/es/`; verify `npm run docs:check` in `docs/`
- [ ] 6.2 `examples/cats-app`: replace the hand-written guard with `JwtStrategy` and `AuthGuard::new("jwt").skip_public()`
  (design decision 9); verify the example builds, its tests pass (`posture.rs`, `startup.rs`, `login_throttle.rs`), and
  it runs on 127.0.0.1

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml` passes (`jwt` alone and `--no-default-features`, which must compile without the strategy)
- [ ] 7.3 `cargo test --workspace` passes, including `auth_jwt.rs` without Node
- [ ] 7.4 Regression tests for every scenario of `auth/jwt-strategy`; the `auth/jwt` tests still pass
- [ ] 7.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 7.6 Framework runtime code changed (`JwtService::verify` refactor): run the corvis-parity check; verify the three
  port apps keep 0 apidiff differences
- [ ] 7.7 Run the security-reviewer agent over `corvis::auth::jwt` and the refactored `JwtService::verify` (algorithm
  confusion, `none`, oversized and malformed tokens, header parsing, key provider errors); verify every finding is
  fixed or recorded as a new issue

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #42, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).
