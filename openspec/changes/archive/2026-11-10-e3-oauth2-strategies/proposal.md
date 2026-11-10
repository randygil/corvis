# Proposal

Implements [randygil/corvis#43](https://github.com/randygil/corvis/issues/43) (epic
[#39](https://github.com/randygil/corvis/issues/39)). Design from the #40 spike
(`openspec/changes/archive/2026-11-08-e3-strategy-module-design/`: fact 6, the crates table, decision 9, the
`auth/oauth2` delta and task group 6), revised here against the strategy core that was merged
(`openspec/changes/archive/2026-11-10-e3-strategy-core-and-guard-responses/`, Implementation notes 11 and 12).

The issue text and `ROADMAP.md:101` name the change `e3-generic-oauth2-google-and-github`; this change is named
`e3-oauth2-strategies` as assigned (see Open question 8 in `design.md`).

**Depends on #44** (`openspec/changes/e3-sessions-and-signed-cookies/`, approved, not implemented yet): `CookieKeys`
(`sign`, `unsign`, the 32-byte secret rule, redacted `Debug`), the `auth-session` cargo feature with `hmac` and
`subtle`, and, for the optional session storage of `state`, `Request::session()`, `Session::get` / `set` / `remove`
and the session cookie options. **Depends on #45** (`e3-api-key-strategy-and-custom-strategies`, approved) for
`Request::redact_query_param`. Order of implementation: #136 (merged) → #42 → #41 → #45 → #44 → **#43** → #46 → #47.
Both APIs are described here as designed, not as shipped; the first task compares them with the merged code.

## Why

"Sign in with Google / GitHub" is the login most new apps start with, and the original stack does it with
`passport-oauth2` and its provider packages. Corvis has the strategy core (`Strategy`, `StrategyOutcome::Redirect`,
`AuthGuard`, guard responses) but no OAuth2 client: `docs/recipes/passport.md:9` lists OAuth2 as pending (#43) and
`docs/security/authentication.md:257` says there are no OAuth strategies.

A hand-written OAuth2 login is where well-known mistakes live, and several are the original's defaults:

- no `state` (`passport-oauth2` uses a `NullStore` unless `state: true`): login CSRF;
- no PKCE unless a session and `state: true` are configured;
- a callback URL built from the request `Host` header when a relative `callbackURL` is configured;
- an HTTP client that follows redirects from the token endpoint (server-side request forgery) and reads responses of
  any size with no deadline;
- provider errors and token responses printed in error pages and logs;
- an unverified provider e-mail used to link accounts;
- a `return_to` parameter turned into an open redirect.

The framework should ship the flow with those closed by default and with every limit written down, and keep the
requests a provider sees and the profile the application receives equal to the original's so a migration is
mechanical.

## What Changes

- New cargo feature `auth-oauth2` (off by default) = `auth-session` + the optional `tokio-rustls` and `webpki-roots`
  dependencies the `health-tls` feature already uses. **No new crate enters the workspace**: the spike's `oauth2` 5.x
  and `reqwest` 0.13 are not added (design decision 2).
- `corvis::auth::oauth2`:
  - `OAuth2Validator` (the application's `validate`, receiving the request and an `OAuth2Grant`: access token,
    refresh token, token response, profile) and `OAuth2Strategy<V>`, registered with
    `Provider::strategy::<OAuth2Strategy<V>>()` and used through `AuthGuard::new("google")` on the login route and
    on the callback route;
  - `OAuth2Options` with the presets `OAuth2Options::google(..)` and `OAuth2Options::github(..)` and a generic
    constructor (authorization URL, token URL, optional OpenID userinfo URL);
  - authorization code flow only, PKCE `S256` on by default, `plain` not expressible;
  - `state`: 32 random bytes per login, bound to the browser by a signed, `HttpOnly`, `SameSite=Lax`, `Secure`,
    `__Host-` cookie that lives 10 minutes (default), or by the session (opt-in), checked in constant time before any
    request to the provider, and used once;
  - the redirect URI is an exact configured absolute URL; it is never built from the request;
  - a private HTTP client for the token, refresh and profile requests: `hyper` client connection over `tokio-rustls`
    (TLS 1.2+, bundled web PKI roots plus optional extra roots), HTTPS required except for loopback addresses, no
    redirects followed, no proxy from the environment, connect and total timeouts, response size limits, bounded
    concurrency;
  - client authentication in the request body by default (what the original sends), HTTP Basic as an option;
  - profiles: Google from the OpenID userinfo endpoint; GitHub from `/user` plus `/user/emails`; the normalized
    profile of the original packages plus a `verified` flag per e-mail and `OAuth2Profile::verified_email()`;
  - failures mapped without leaking: a provider `error`, a missing, forged, expired or replayed `state` and a code
    the provider rejects are the plain 401 (or a redirect to a configured failure page); a provider that is down or
    answers garbage is the fixed 500; provider text never reaches a response;
  - `return_to`: opt-in, accepted only as a relative path under the documented rule, carried inside the signed state
    record, exposed to the handler with `oauth2::return_to(&req)`;
  - `OAuth2Client::refresh(..)`: exchanges a refresh token with the same client; the framework stores no token;
  - `Debug` of every type that holds a code, a token, a secret, a verifier or a profile prints no value; nothing of
    that kind is logged, and the callback's `code` and `state` are redacted from the logged URL.
- Fixtures recorded ahead of time from `passport-oauth2` 1.8.0, `passport-google-oauth20` 2.0.0, `passport-github2`
  0.1.12 and `passport-oauth2-refresh` 2.2.0 against a fake provider (`crates/corvis/tests/fixtures/auth/oauth2.mjs` →
  `oauth2.json`); `crates/corvis/tests/auth_oauth2.rs` replays them against a mock provider on `127.0.0.1`. Tests
  need neither Node nor the network.
- Docs: an OAuth2 section in `docs/recipes/passport.md` and `docs/es/recipes/passport.md` (setup for Google and
  GitHub, every limit and default, the divergences), `docs/security/authentication.md` and its Spanish page.
- Example: `examples/cats-app` gets a GitHub login whose endpoints come from configuration, tested against a local
  mock provider.

## Capabilities

### New Capabilities

- `auth/oauth2`: authorization code flow with `state` and PKCE, the provider HTTP client and its limits, Google and
  GitHub profiles, failure mapping, `return_to`, refresh.

### Modified Capabilities

None. `auth/strategies` and `http/guard-responses` are used as merged; `http/signed-cookies` and `auth/sessions`
(from #44) are used as designed.

## Impact

- `crates/corvis/Cargo.toml`: feature `auth-oauth2`; no new dependency entries (`tokio-rustls` line 48 and
  `webpki-roots` line 49 are already optional dependencies).
- `crates/corvis/src/auth/oauth2/` (new): `mod.rs`, `options.rs`, `strategy.rs`, `state.rs`, `client.rs`,
  `profile.rs`, `secret.rs`.
- `crates/corvis/tests/auth_oauth2.rs` and `crates/corvis/tests/fixtures/auth/` (`oauth2.mjs`, `oauth2.json`,
  `package.json`, `package-lock.json`).
- `.github/workflows/ci.yml`: the feature in the per-feature loop and a test step.
- `docs/recipes/passport.md`, `docs/security/authentication.md`, their Spanish pages; `examples/cats-app`.
- No runtime code outside the feature changes, so applications that do not enable `auth-oauth2` (the three port apps
  among them) compile the same code as before.

## Out of scope

- OpenID Connect ID token validation, discovery (`/.well-known/openid-configuration`), JWKS, `nonce`. The raw
  `id_token` is handed to the validator unverified and documented as such (design decision 12). May become a new
  issue ("OpenID Connect strategy").
- The implicit flow, the resource owner password grant, client credentials, device flow, `response_mode=form_post`.
- Storing access or refresh tokens, token revocation, automatic refresh.
- A redirect URI derived from the request host, and a list of redirect URIs chosen per request (design decision 5;
  may become a new issue for multi-domain apps).
- RFC 9207 (`iss` authorization response parameter), pushed authorization requests, DPoP, mutual TLS client
  authentication, `private_key_jwt`.
- Other providers (Microsoft, Apple, Facebook, GitLab): the generic strategy covers them without a normalized
  profile.
- Rewriting `HttpHealthIndicator` on top of the new private client (`crates/corvis/src/modules/health.rs:303-357`
  keeps its own request code); a follow-up could share it.
- A public general-purpose HTTP client.
- Throttling of the login and callback routes (#46) and the overall authentication docs and example (#47).
- `AuthGuard` on WebSockets (denied by the core).
