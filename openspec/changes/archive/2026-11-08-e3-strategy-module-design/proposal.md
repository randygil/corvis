# Proposal

Implements [randygil/corvis#40](https://github.com/randygil/corvis/issues/40) (epic
[#39](https://github.com/randygil/corvis/issues/39), follow-ups #41-#47). This is a spike: the deliverable is the
design in `design.md`, the requirements the epic's features must meet (spec deltas) and the plan for the follow-up
issues (`tasks.md`). No framework code changes in this change.

## Why

Almost every backend built on the original model authenticates through the Passport integration: strategies
registered by name, a `validate()` method that turns credentials into a user, `AuthGuard('jwt')` on routes,
`req.user` in handlers, and a fixed `401 {"message":"Unauthorized","statusCode":401}` when every strategy fails.
Corvis has none of that today:

- `docs/recipes/passport.md:1-7` marks the integration as `status: unavailable` and says there is "no equivalent
  system of interchangeable authentication strategies".
- The only building blocks are `JwtService` (`crates/corvis/src/jwt.rs:186-311`), `extract_bearer_token`
  (`jwt.rs:314-318`), `Request::set_user` / `user_as` (`crates/corvis/src/http/request.rs:222-232`) and a guard
  the user writes by hand (`docs/security/authentication.md:133-165`).

Migrating an app means rewriting every strategy as a hand-made guard and re-checking by hand that every failure
path still answers exactly the same. The epic needs one design that all its tasks (local, JWT, OAuth2, sessions,
API keys) plug into, decided before the first of them starts.

## What Changes

- `design.md` compares the ways to model strategies in Corvis (registry + object-safe trait, typed validator traits
  with adapters, a macro, wrapping existing Rust crates such as `axum-login`), with evidence from the Node sources
  (`@nestjs/passport` 12.0.1, `passport` 0.7.0, `passport-jwt` 4.0.1, `passport-local` 1.0.0, `passport-oauth2`
  1.8.0, `express-session` 1.19.0) and from the Corvis code, and recommends one.
- Spec deltas fix the observable behavior every follow-up must match, each verified by a differential test against
  the Node library: the strategy registry and `AuthGuard` (`auth/strategies`), the local strategy (`auth/local`),
  the JWT strategy (`auth/jwt-strategy`), OAuth2 (`auth/oauth2`), sessions (`auth/sessions`) and API keys
  (`auth/api-key`), plus the core router prerequisite (`http/guard-responses`). Where the original behavior is
  unsafe (OAuth2 without `state`, silent strategy overrides, timing-dependent comparisons), the specs diverge on
  purpose and say so.
- The differential tests follow the pattern already in the repo (`crates/corvis/tests/fixtures/res_express.oracle.mjs`
  recorded into a JSON fixture and replayed by `cargo test`), so neither `cargo test` nor CI needs Node.
- `tasks.md` is the plan for #41-#47, one group per issue, including the two core prerequisites the design found
  (both scheduled in #42 with the strategy core): a way for a guard to answer with a redirect or add `Set-Cookie`
  headers, and a startup check for unknown strategy names.

## Capabilities

### New Capabilities

- `auth/strategies`: named strategy registry, strategy outcomes, `AuthGuard` (single, chained, optional), user and
  auth info on the request, failure responses.
- `auth/local`: username/password strategy and password hashing compatible with bcryptjs hashes.
- `auth/jwt-strategy`: JWT strategy with extractors, key provider, issuer/audience and `ignore_expiration`.
- `auth/oauth2`: authorization code flow with `state` and PKCE, Google and GitHub profiles.
- `auth/sessions`: sessions and signed cookies compatible with express-session, login/logout through a session.
- `auth/api-key`: API key in a header or query parameter, and custom strategies.
- `http/guard-responses`: a guard can end the request with a response (redirect) and add response headers
  (`Set-Cookie`) that survive the handler and exception filters.

### Modified Capabilities

None. `auth/jwt` (`openspec/specs/auth/jwt/spec.md`, `JwtService` token verification) is reused unchanged by
`auth/jwt-strategy`.

## Impact

- This change: planning artifacts only, under `openspec/changes/e3-strategy-module-design/`.
- Follow-ups (planned in `tasks.md`): a new `auth` module in `crates/corvis` behind cargo features; small core
  additions in `crates/corvis/src/router/mod.rs` (guard short-circuit response and extra response headers),
  `crates/corvis/src/di/provider.rs` (strategy providers) and `crates/corvis/src/application.rs` (registry built in
  `init()`, startup check); docs `docs/recipes/passport.md`, `docs/security/authentication.md`,
  `docs/security/encryption-and-hashing.md` and their Spanish translations.
- Port apps: consi and facten use hand-written guards over `@nestjs/jwt` and `bcryptjs`, not Passport
  (`corvis-tests/nest/consi-backend/src/auth/jwt-auth.guard.ts`); their Rust ports use `bcrypt = "0.17"`. The
  epic adds opt-in APIs, so their behavior must not change; the parity check guards the core router additions.

## Out of scope

- Implementing any of the strategies (that is #41-#45).
- `AuthGuard` on WebSocket gateways (handshake authentication through strategies): possible new issue; today
  `examples/tertulia` verifies the token in `handle_connection`.
- OpenID Connect discovery and ID token validation (`openidconnect` crate), SAML, LDAP, WebAuthn/passkeys, 2FA/TOTP:
  possible new issues.
- Flash messages (`failureFlash`, `successFlash`, `req.flash`) and `failureMessage` stored in the session: the
  `AuthGuard` callback path of the original never uses them.
- Changing `extract_bearer_token` (`crates/corvis/src/jwt.rs:314-318`) to the passport-jwt parsing: the JWT
  strategy gets its own extractors and the existing function keeps its behavior; aligning it could be a new issue.
- CSRF tokens for session-based apps (`docs/security/csrf.md` stays `unavailable`): possible new issue once
  sessions exist.
- Upgrading `jsonwebtoken` from 9 (`crates/corvis/Cargo.toml:34`) to 11.x: possible new issue, independent of this
  epic.
- The route posture check (`#[public]`, epic #24): this design states how `AuthGuard` interacts with it, but the
  check itself belongs to #26.
