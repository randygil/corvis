# Design

## Context

This change adds the OAuth2 strategies on top of the strategy core merged with #136. Line numbers were checked on
`origin/dev` at `e8218d3`. The APIs of #44 (`CookieKeys`, `Session`) and #45 (`Request::redact_query_param`) are
approved designs that are not merged yet; they are described as designed.

### Corvis today

- **Strategy contract.** `Strategy::authenticate(&self, ctx: &ExecutionContext, options: &AuthenticateOptions) ->
  StrategyOutcome` (`crates/corvis/src/auth/strategy.rs:35-44`); the outcomes are `Success`, `Fail`, `Redirect { url,
  status }`, `Pass`, `Error` (47-59). The `Redirect` doc says the URL is sent as given (52-54). `Debug` of
  `StrategyOutcome` and `AuthenticatedUser` prints no value (62-72, 104-108).
- **Redirects.** `Authenticator::authenticate` turns `Redirect` into a response with `redirect_response` and stores it
  with `respond_with`; when that returns `false` the result is an internal 500
  (`crates/corvis/src/auth/authenticator.rs:153-159`). `redirect_response` (175-187) refuses a URL that is not a valid
  header value and a status outside 300-399, and writes `Location` as given with `Content-Length: 0`.
- **Guard phase.** `HttpArgumentsHost::respond_with` stores nothing outside an HTTP context or outside the guard phase
  (`crates/corvis/src/common/context.rs:343-346`); `GuardPhase::is_active` is `pub(crate)` (212-234), so code inside
  the crate can ask before doing anything else. `run_pipeline` ends the pipeline right after the guard that stored a
  response (`crates/corvis/src/router/mod.rs:806-818`), so no later guard runs after a redirect.
- **Cookies from a strategy.** `append_response_header` (`context.rs:301-331`) records `Set-Cookie` values that are
  sent on every path, including a later 401 or 403 and a stored response; at most 128 values per request (158).
- **`AuthGuard`.** `AuthGuard::new(name)` runs the strategy; `Redirected` is `Ok(true)`
  (`crates/corvis/src/auth/guard.rs:178`); every `Fail` is the empty 401 whatever status the strategy gave, unless
  `on_failure` builds another exception (179-188); `with_options(json)` hands a JSON object to the strategy (105-110).
  `Request::auth_info()` returns the challenge of a failed attempt (`crates/corvis/src/http/request.rs:269`).
- **Request data.** `Request::query()` is a JSON map whose values can be strings, arrays or objects, `query_param`
  returns a string value (`request.rs:161-164`); `cookie(name)` parses like `cookie.parse` (228); `hostname()` reads
  `Host`, or `X-Forwarded-Host` behind a trusted proxy (198-217). There is no `protocol()` or `secure()`.
- **Writing cookies.** `serialize_cookie(name, value, &CookieOptions)` (`crates/corvis/src/http/res.rs:246`,
  options at 144-160) encodes the value with `encodeURIComponent` and panics on an invalid name, domain or path.
- **Outbound HTTP.** The only client code is the health indicator: one `GET` per connection with
  `hyper::client::conn::http1` (`crates/corvis/src/modules/health.rs:303-357`), and for `https` a `tokio-rustls`
  connector with `webpki_roots::TLS_SERVER_ROOTS` that uses the installed crypto provider or `ring`
  (`health.rs:359-378`, behind `health-tls`). It reads no body and has no timeout of its own.
- **Dependencies.** `hyper` has the `client` feature (`crates/corvis/Cargo.toml:28`); `tokio-rustls` 0.26 (`ring`,
  `tls12`) and `webpki-roots` 0.26 are optional (48-49; features `tls` and `health-tls`, 60-61); `url` 2 (41),
  `form_urlencoded` (31), `base64` 0.22 (39), `rand` 0.8 (43), `sha2` 0.10 (45), `http-body-util` (26) are
  unconditional. `Cargo.lock` has no `reqwest`, `oauth2`, `hyper-rustls`, `native-tls` or `openssl`. `hmac` 0.12.1 and
  `subtle` 2.6.1 are transitive today and become optional direct dependencies with #44's `auth-session`.
- **`deny.toml`.** The graph is evaluated with all features; the license allow-list holds "only those that appear in
  the graph today; a new license forces a review"; sources are crates.io only; yanked crates are an error.
- **Fixtures.** `crates/corvis/tests/fixtures/auth/` has `package.json` (exact versions), `package-lock.json`,
  `strategies.mjs` and `strategies.json`; its `node_modules` is ignored (`.gitignore:32`).
- **Docs.** `docs/recipes/passport.md:77` states the return-to rule: a relative path that starts with a single `/`,
  is not followed by `/` or `\`, and has no backslash or control character.
- **CI.** Per-feature clippy loop (`.github/workflows/ci.yml:68-73`), per-feature test steps (113-116), `cargo deny
  check` (196).

### The original

Facts from the spike (its fact 6) and the packages' documented behavior; the implementer re-reads the sources when
writing the generator and corrects this section where they differ.

- `passport-oauth2` 1.8.0 (`lib/strategy.js`): a request with `?error=access_denied` fails; another `?error` is an
  `AuthorizationError` (500 through the framework); without `code` it redirects to the authorization URL with
  `response_type=code`, `redirect_uri`, `scope` (joined with `scopeSeparator`, a space), `client_id`, and `state` /
  `code_challenge` only when `state: true` (and `pkce: true`); with `code` it verifies the state (`fail(info, 403)`),
  posts to the token URL with `grant_type=authorization_code`, `code`, `redirect_uri`, `client_id` and
  `client_secret` in the form body (through the `oauth` package), loads the profile and calls
  `validate(accessToken, refreshToken, [params], profile)`. The default state store keeps nothing. A relative
  `callbackURL` is resolved against the request (`utils.originalURL`, which reads `Host` and, with `proxy: true`, the
  forwarded headers). Token endpoint failures are `TokenError` / `InternalOAuthError`, 500 through the framework.
- `passport-google-oauth20` 2.0.0: authorization `https://accounts.google.com/o/oauth2/v2/auth`, token
  `https://www.googleapis.com/oauth2/v4/token`, profile `https://www.googleapis.com/oauth2/v3/userinfo`; the profile
  has `provider: 'google'`, `id` (`sub`), `displayName`, `name { familyName, givenName }`, `emails [{ value,
  verified }]`, `photos [{ value }]`, `_raw`, `_json`.
- `passport-github2` 0.1.12: authorization `https://github.com/login/oauth/authorize`, token
  `https://github.com/login/oauth/access_token`, profile `https://api.github.com/user` with a `User-Agent`; when the
  scope includes `user:email` it also requests `https://api.github.com/user/emails`; the profile has `provider:
  'github'`, `id`, `nodeId`, `displayName`, `username`, `profileUrl`, `emails`, `photos`, `_raw`, `_json`.
- `passport-oauth2-refresh` 2.2.0: posts `grant_type=refresh_token` and `refresh_token` with the strategy's client
  credentials.

## Goals / Non-Goals

**Goals**

- "Sign in with Google / GitHub / a generic OAuth2 provider" with `AuthGuard`, a validator and a preset.
- The requests the provider receives and the profile the application receives equal the original's, pinned by a
  recorded fixture, except where this design names a divergence.
- Closed by default: `state` and PKCE always, exact redirect URI, a provider client that cannot be redirected, stalled
  or flooded, no secret in any log, response or `Debug` output, no open redirect.
- Every limit has a default, a bound and a startup check.
- No new crate in the workspace.

**Non-Goals**

- OpenID Connect (ID token validation, discovery), other grant types, token storage (proposal Out of scope).
- A public HTTP client.

## Decisions

### 1. Feature and module

`auth-oauth2 = ["auth-session", "dep:tokio-rustls", "dep:webpki-roots"]`, off by default, following the epic's
convention (one feature per strategy family, `auth-<name>`). `auth-session` brings `CookieKeys`, `hmac` and `subtle`
(#44 decision 1 already says "#43 enables `auth-session` or depends on it"). The code lives in
`corvis::auth::oauth2`.

Alternative: separate `auth-google` and `auth-github` features. Rejected: the presets are two constructors and two
profile parsers, with no dependency of their own.

### 2. No `oauth2` crate and no `reqwest`: a private client on `hyper` and `tokio-rustls`

The spike's table listed `oauth2` 5.0.0 with `reqwest` 0.13.5. Neither is in `Cargo.lock`. Options:

| Option | New crates | Control over limits | Notes |
|---|---|---|---|
| `oauth2` 5 + `reqwest` 0.13 | dozens (`reqwest`, `hyper-rustls`, `tower`, `rustls-platform-verifier` or `aws-lc-rs`, ...) | redirects and timeout yes; body size limit and connect pinning by hand | a second TLS provider (`aws-lc-rs`) next to `ring` is what `health.rs:367-369` already had to defend against; new licenses to review in `deny.toml` |
| `oauth2` 5 with our own `AsyncHttpClient` | `oauth2` and its tree | full | the crate then only builds one URL and two form bodies, and its typed responses do not carry the original's profile shapes |
| **Private client** (chosen) | none | full | about 300 lines; the pattern exists in `health.rs` |

The authorization code flow needs: one URL with query parameters (`url`), a SHA-256 (`sha2`), base64url (`base64`),
random bytes (`rand`), a form body (`form_urlencoded`), JSON (`serde_json`) and an HTTPS POST. All are in the tree.
The supply chain argument decides: an authentication feature should not add dozens of crates to do one POST.

The client (`crate::auth::oauth2::client`, `pub(crate)` except `OAuth2Client::refresh`):

- **Transport.** Resolve the host with `tokio::net::lookup_host`, connect with a deadline, then for `https` a
  `tokio-rustls` handshake with the server name of the URL, safe default protocol versions (TLS 1.2 and 1.3), the
  installed crypto provider or `ring` (the rule of `health.rs:367-369`), roots = `webpki_roots::TLS_SERVER_ROOTS` plus
  `OAuth2HttpOptions::extra_root_certificate(der)` for a provider behind a private CA. There is no way to turn
  certificate or host name verification off. Then `hyper::client::conn::http1` with one request and `Connection:
  close`. No connection pool: a login makes two or three requests.
- **Scheme rule.** `https` is required. `http` is accepted only when the URL's host is `localhost` or a loopback
  address literal **and** the address actually connected to is loopback; anything else is a startup error
  (configuration) or a refused request (resolution). This is what lets the tests and the example use a mock on
  `127.0.0.1` without a switch that disables TLS.
- **No redirects.** A 3xx answer is a failed request; `Location` is never read. No proxy is taken from the
  environment (`HTTP_PROXY` and friends are ignored), no cookies are kept, no `Authorization` header is ever sent to
  a host other than the configured one.
- **Deadlines.** Connect timeout (TCP plus TLS) and a total timeout that covers resolution, connect, request and the
  whole body. On expiry the connection is dropped.
- **Size limits.** The response body is read frame by frame and the request fails as soon as the limit is passed; a
  `Content-Length` over the limit fails before reading. The header block is limited with the `hyper` client
  builder's buffer size. Bodies are never decompressed (`Accept-Encoding: identity`), so there is no decompression
  bomb.
- **Concurrency.** A semaphore per strategy bounds the requests in flight; when it is full the callback answers 503
  `Service Unavailable` without waiting (decision 11).
- **Errors.** `OAuth2Error` is an enum without payloads from the provider: `Timeout`, `Connect`, `Tls`,
  `InsecureUrl`, `Redirected`, `TooLarge`, `Status(u16)`, `InvalidResponse`, `Rejected { code: Option<String> }`
  (a sanitized OAuth error code, decision 9), `Busy`. Its `Display` never contains a URL query, a body or a header.

Alternative for the roots: the operating system's store (`rustls-native-certs` or `rustls-platform-verifier`): new
crates, and behavior that changes per machine. Rejected; `webpki-roots` is what `health-tls` uses.

### 3. Validator, options and what `validate` receives

```rust
#[async_trait]
pub trait OAuth2Validator: Injectable {
    const NAME: &'static str = "oauth2";
    type User: Serialize + Clone + Send + Sync + 'static;
    fn options(&self) -> OAuth2Options;
    async fn validate(&self, req: &Request, grant: OAuth2Grant) -> Result<Option<Self::User>, HttpException>;
    fn info(&self, _user: &Self::User) -> Option<Value> { None }
}

// providers = [GoogleAuth, Provider::strategy::<OAuth2Strategy<GoogleAuth>>()]
// #[get("auth/google")]          #[use_guards(AuthGuard::new("google"))]   // redirects to Google
// #[get("auth/google/callback")] #[use_guards(AuthGuard::new("google"))]   // handler runs with User<T>

OAuth2Options::google(client_id, client_secret, "https://app.example/auth/google/callback")
    .scope(["openid", "email", "profile"])
    .state_cookie(CookieKeys::new([secret])?)          // or .state_in_session()
    .return_to_param("return_to")                      // off unless set
    .failure_redirect("/login?error=oauth")            // off unless set
    .authorization_param("access_type", "offline")
    .client_auth(ClientAuth::RequestBody)              // default; ClientAuth::Basic
    .http(OAuth2HttpOptions::default().timeout(Duration::from_secs(10)))

OAuth2Options::github(client_id, client_secret, redirect_uri).scope(["user:email"])
OAuth2Options::new(OAuth2Endpoints::new(authorization_url, token_url).userinfo_url(url), client_id, client_secret,
                   redirect_uri)
```

The shape follows #42 and #41: `OAuth2Strategy<V>` is `Injectable`, resolves `V` through DI, calls `options()` once
and validates it; an invalid configuration is a `DiError` that names the strategy, so the application does not
start. The strategy name is `V::NAME` (an app sets `"google"`, `"github"`).

`OAuth2Grant` (`#[non_exhaustive]`, `Debug` prints field names only):

| Field | Type | Content |
|---|---|---|
| `access_token` | `OAuth2Token` | the access token |
| `refresh_token` | `Option<OAuth2Token>` | present when the provider sent one |
| `params` | `OAuth2TokenResponse` | the whole token response (`get(key) -> Option<&Value>`, `expires_in()`, `scope()`, `id_token() -> Option<OAuth2Token>`, documented as **unverified**) |
| `profile` | `Option<OAuth2Profile>` | the normalized profile; `None` when no profile source is configured |

This is the original's `validate(accessToken, refreshToken, params, profile)` as one value, so adding a field later
does not break validators. The request is always passed (spike decision 2).

`OAuth2Token` wraps a `String`: `expose() -> &str`, `Debug` prints `OAuth2Token(..)`, no `Display`, no `Serialize`
(a token cannot be put in a JSON response or a session by accident; the application calls `expose()` on purpose).
`ClientSecret` is the same kind of wrapper, crate-private. `OAuth2TokenResponse`'s `Debug` prints the keys only.
`OAuth2Profile`'s `Debug` prints `provider` only (the rest is personal data, as with `AuthenticatedUser`).

Per-guard options (`AuthGuard::with_options`): only `scope` (an array of strings that replaces the configured scope
for that route, validated like the configured one; an invalid value is the 500 path). `callbackURL` and `state`
in the options are ignored: the original lets a route override both, which is how an app ends with an unverified
redirect URI.

### 4. Flow, PKCE and `state`

Only `GET` requests are processed; any other method fails (401). The strategy decides by the query, like the
original: neither `code` nor `error` → **start**; otherwise → **callback**.

**Start**

1. If the guard phase is not active (`GuardPhase::is_active`), return the internal 500 before generating or storing
   anything (the `Authenticator` would refuse the redirect anyway, `authenticator.rs:155-157`).
2. Generate `state` and the PKCE verifier: 32 bytes each from `rand::rngs::OsRng`, base64url without padding (43
   characters; 256 bits). A generator failure is a 500; no fallback.
3. `code_challenge = base64url(SHA-256(verifier))`, `code_challenge_method=S256`. There is no `plain` variant in the
   API. `.pkce(false)` exists for a provider that refuses unknown parameters and logs a startup warning.
4. Read `return_to` when enabled (decision 10).
5. Store the record (below) and return `StrategyOutcome::Redirect` to the authorization URL: the configured URL with
   its own query kept, plus `response_type=code`, `client_id`, `redirect_uri`, `scope` (when not empty), the extra
   parameters, `state`, `code_challenge`, `code_challenge_method`, in the order the recorded original writes the
   ones it has. The response also gets `Cache-Control: no-store` and `Referrer-Policy: no-referrer`
   (`set_response_header`).

**Record**: `{ "n": strategy name, "s": state, "v": verifier, "t": issued-at seconds, "r": return_to }`.

**Callback** (in this order; nothing reaches the provider before step 4)

1. `code`, `state` and `error` must each be absent or one string (a repeated parameter is an array in
   `Request::query()`): anything else fails. `code` and `error` together fail. Length limits apply (decision 11).
   `Request::redact_query_param` is called for `code` and `state` so the logged URL hides them.
2. Take the record out of the store (cookie: append the deleting `Set-Cookie`; session: remove the key). This happens
   on **every** callback, valid or not.
3. The record must exist, name this strategy, be younger than the lifetime and not from the future (beyond 60
   seconds of clock skew), and its `state` must equal the query's (`subtle::ConstantTimeEq` on the bytes). Otherwise
   → fail.
4. `error` present → fail (decision 9). Else exchange the code: `grant_type=authorization_code`, `code`,
   `redirect_uri`, `code_verifier`, client credentials (decision 6).
5. Profile (decision 7), then `validate`: `Ok(Some(user))` → `Success` (info from `info()`), `Ok(None)` → `Fail`,
   `Err(e)` → `Error(e)`.
6. The callback response gets `Cache-Control: no-store` and `Referrer-Policy: no-referrer`; the docs tell the handler
   to redirect so the `code` leaves the address bar.

**Where the record lives** (must be chosen; not choosing is a startup error):

- `state_cookie(keys)`: the cookie value is `keys.sign(base64url(JSON record))` written with `serialize_cookie`.
  Name `__Host-oauth2.<strategy name>` (the `__Host-` prefix makes browsers refuse a cookie set by a sibling
  subdomain, which closes login CSRF by cookie tossing), `Path=/`, `HttpOnly`, `Secure`, `SameSite=Lax` (the callback
  is a top-level cross-site navigation, so `Strict` would never be sent), `Max-Age` = the lifetime, no `Domain`.
  `.state_cookie_secure(false)` (plain-HTTP development) drops `Secure` and the prefix and logs a startup warning;
  `.state_cookie_name(..)` overrides the name (validated at startup; a `__Host-` or `__Secure-` name without
  `Secure` is an error). The name is fixed at startup and the value is base64url plus a signature, so the panics of
  `serialize_cookie` cannot be reached per request. Stateless: an anonymous visitor who starts a login creates
  nothing on the server.
- `state_in_session()`: the record is stored with `Session::set("oauth2:<strategy name>", record)` and removed with
  `Session::remove` in step 2. Startup fails when `SessionModule` is not installed or when the session cookie is
  `SameSite=Strict`. A request without a session (middleware not reached) is the 500 path. Cost: every login start
  creates a session record, so the login route should be rate limited (#46).

**Single use.** With the session, the record is gone from the store before the exchange, so a second callback with
the same `state` fails whatever the client sends. With the cookie, the server deletes the cookie and the record
expires after the lifetime; a client that ignores the deletion and replays its own callback within that window
passes the state check, and the provider then rejects the reused `code` (authorization codes are single use, RFC
6749 section 4.1.2), which is the 401 of decision 9. The replay needs the victim's own cookie jar, so it is not a
cross-user attack; the docs state the difference.

The deleting cookie is appended on failed callbacks too. The core's rule "cookies are appended only on success"
(#136 Implementation notes 11) is about cookies that carry authority; a deletion carries none, and not deleting would
leave a usable record behind after a failure.

One record per strategy: a second login started in another tab replaces the first, whose callback then fails. The
original's session store behaves the same way.

Alternatives considered:

- *Automatic choice* (spike: session when installed, cookie otherwise). Rejected: installing sessions for another
  reason would silently change where `state` lives, the cost profile (a store write per anonymous visitor) and the
  dependence on the session cookie's `SameSite`.
- *Encrypted cookie*. The verifier is readable by the browser that started the flow (not by scripts: `HttpOnly`).
  PKCE defends against a party that sees the `code` but not this browser's cookies, so signing is enough; encryption
  would need an AEAD crate.
- *Server-side set of used states in cookie mode* to make single use strict. Rejected: it is per process, so it gives
  a false guarantee behind a load balancer; apps that need the strict guarantee choose `state_in_session()`.
- *`state` as an HMAC of the session identifier* (no storage). Rejected: needs a session and gives no place for the
  verifier.

### 5. The redirect URI is configured, exact and absolute

`redirect_uri` is a required constructor argument. Startup checks: it parses as an absolute URL, has no fragment, its
scheme is `https` (or `http` with a loopback host, RFC 8252 section 7.3), and it has no user info. The same string is
sent in the authorization request and in the token request. Nothing reads `Request::hostname()`
(`request.rs:198-217`, which follows `Host` / `X-Forwarded-Host`).

Alternatives: resolving a relative path against the request, as the original does (a forged `Host` header then
decides where the provider sends the `code` unless the provider's own allow-list saves the app; and `Request` cannot
tell `http` from `https`); a list of allowed redirect URIs matched against the request host (safe, useful for
multi-domain apps, but needs the chosen entry in the state record and a decision about `X-Forwarded-Host`). Neither
is offered; the second is listed as out of scope and Open question 3.

### 6. Client authentication: request body by default, Basic as an option

`ClientAuth::RequestBody` (default): `client_id` and `client_secret` in the form body, which is what the original
sends and what Google's and GitHub's documentation show, so the recorded token request can be compared field by
field. `ClientAuth::Basic`: `Authorization: Basic base64(urlencode(id) ":" urlencode(secret))` with the
`application/x-www-form-urlencoded` encoding RFC 6749 section 2.3.1 requires, and no client credentials in the
body. RFC 6749 prefers Basic, but real providers differ on the encoding of special characters; the option is there
for providers that require it.

The token request sends `Content-Type: application/x-www-form-urlencoded`, `Accept: application/json` and a
`User-Agent`. The response is parsed as JSON when its content type is JSON, and as a form body when it is
`application/x-www-form-urlencoded` or `text/plain` (GitHub's historical default; the original accepts both). It must
have a non-empty string `access_token`; `token_type`, when present, must equal `bearer` ignoring case (anything else
is `InvalidResponse`: the profile request would not know how to use it).

The client secret must not be empty (startup error). The 32-byte rule of the epic applies to secrets that are ours
(the `CookieKeys` of the state cookie, enforced by #44); a client secret is issued by the provider and its length
is not ours to set.

### 7. Profiles

Every profile request sends `Authorization: Bearer <token>`; the token is never put in a URL (if the recorded
original sends `access_token` in the query for a provider, that is a marked divergence).

- **Google**: `GET https://www.googleapis.com/oauth2/v3/userinfo` (OpenID userinfo). Mapping: `id` = `sub`,
  `display_name` = `name`, `name.family_name` / `given_name`, `emails` = `[{ value: email, verified: email_verified
  }]` when `email` is present, `photos` = `[picture]`.
- **GitHub**: `GET https://api.github.com/user` with `Accept: application/vnd.github+json` and the `User-Agent` the
  recorded original sends; `id` (as a string), `node_id`, `display_name` = `name`, `username` = `login`,
  `profile_url` = `html_url`, `photos` = `[avatar_url]`. When the effective scope contains `user:email` (or
  `user`), also `GET https://api.github.com/user/emails`; `emails` is every entry as `{ value, primary, verified }`.
  Without that scope, `emails` holds the public `email` of `/user`, if any, with `verified: false`. A failed e-mail
  request is the 500 path, not a profile without e-mails (an app must not silently fall back to an unverified
  address).
- **Generic**: with `userinfo_url`, the response is read as an OpenID userinfo document (the Google mapping) with
  `provider` = the strategy name; without it, `profile` is `None`.

`OAuth2Profile` (`Serialize`, `Clone`, `#[non_exhaustive]`): `provider`, `id`, `display_name`, `username`, `name`,
`emails: Vec<OAuth2Email { value, verified: bool, primary: bool }>`, `photos`, `profile_url`, `raw: Value`. Its JSON
form uses the original's keys (`displayName`, `profileUrl`, `_json`, ...) so the fixture compares it field by field
with the profile the original hands to `validate()`; `_raw` (the body as a string) is not kept.

`OAuth2Profile::verified_email() -> Option<&str>`: the primary verified address (GitHub: `primary && verified`;
Google and generic: the address when `email_verified` is `true`). The docs say to identify users by `(provider, id)`
and to link an existing account by e-mail only through `verified_email()`; linking by an unverified address is an
account takeover. The `verified` and `primary` flags on GitHub e-mails are an addition where the original omits them
(marked in the fixture).

### 8. Nothing sensitive is logged or printed

- `Debug` is hand-written for `OAuth2Options`, `OAuth2Strategy`, `OAuth2Client`, `OAuth2Grant`, `OAuth2Token`,
  `OAuth2TokenResponse`, `OAuth2Profile`, the state record and the PKCE pair; none prints a code, token, secret,
  verifier, state, cookie value or personal data. `OAuth2Options` prints the client id, the endpoint hosts, the
  redirect URI and the scope.
- Log lines (logger `OAuth2Strategy`) are fixed strings plus: the strategy name, the endpoint **host**, the HTTP
  status, the `OAuth2Error` variant and, for a provider error, the sanitized error code. Never: query strings,
  request or response bodies, headers, `error_description`, `error_uri`.
- Exceptions built by the strategy carry fixed messages; the 500 body is the standard `Internal server error`.
- The callback calls `Request::redact_query_param("code")` and `("state")` (#45), so `LoggingInterceptor` and
  `redacted_url()` hide them. The docs warn that a reverse proxy's access log still records the callback URL; the
  `code` is single use and bound to the PKCE verifier.

### 9. Failure mapping

| Situation on the callback | Outcome | Response | Original |
|---|---|---|---|
| Method not `GET`; repeated or oversized `code` / `state` / `error`; `code` with `error` | `Fail` | 401 | not checked |
| Record missing, for another strategy, expired, or `state` different | `Fail` | 401 | `fail(.., 403)` → 401 (only with `state: true`) |
| `error=access_denied` (valid state) | `Fail` | 401 | 401 |
| Any other `error` (valid state) | `Fail` | 401 | 500 — **divergence** |
| Token endpoint answers 400 or 401 (`invalid_grant`, a reused code, ...) | `Fail` | 401 | 500 — **divergence** |
| Concurrency limit reached | `Error` | 503 `Service Unavailable` | no limit |
| Token or profile endpoint: timeout, connect or TLS failure, 3xx, 5xx, other 4xx, oversized or unparseable body, no `access_token` | `Error` | 500 `{"statusCode":500,"message":"Internal server error"}` | 500 |
| `validate` returns `None` / `Err(e)` | `Fail` / `Error(e)` | 401 / `e` | same |

Why the two divergences: both are caused by the request, not by the server. Anyone can craft
`/callback?error=x` or replay a used `code`; answering 500 lets a visitor fill the error log and trip alerts at will.
With a valid state they are still the user's own doing (consent refused in another form, a back-button replay).
The cases that stay 500 mean the provider or the configuration is broken, which an operator must see.

Every `Fail` carries `challenge: {"message": <fixed reason>}` with one of `state mismatch`, `access denied`,
`provider error`, `code rejected`, `invalid callback`, plus `"error": <code>` for a provider error. The code is the
`error` parameter (or the token response's `error`) only when it is 1 to 64 characters of `[A-Za-z0-9_.-]`; otherwise
it is omitted. The challenge is visible to `AuthGuard::on_failure` and `Request::auth_info()`, never in the default
response. `error_description` and `error_uri` are dropped unread.

`failure_redirect(url)`: when set, every `Fail` of the **callback** becomes `StrategyOutcome::Redirect { url, 302 }`
(a browser lands on the login page instead of a JSON 401). The URL is a fixed configured string, checked at startup:
a relative path under the return-to rule of decision 10, or an absolute `https` URL. Nothing from the request is
added to it. 500 and 503 outcomes are not redirected.

Alternative: map everything to the original's statuses (parity). Rejected for the two rows above (security over
parity); every other row equals the recorded original.

### 10. `return_to`

Off by default. `.return_to_param("return_to")` makes the **start** request read that query parameter. It is kept
only when it is one string that: is at most 1024 bytes; starts with `/`; whose second character is not `/` or `\`;
contains no backslash, no control character (bytes below 0x20 and 0x7F) and no non-ASCII byte (already
percent-encoded paths pass). This is the rule of `docs/recipes/passport.md:77` plus the length and ASCII bounds. A
value that fails the rule is dropped (the login continues without it) and counted in a debug log line without the
value.

The accepted value travels in the signed record (`"r"`), never in the `state` parameter or in a URL the provider
sees. After a successful callback, `corvis::auth::oauth2::return_to(&req) -> Option<String>` returns it (a private
request extension set by the strategy on success only); the handler redirects there or to its default. The callback
request's own query is never consulted for it.

Alternative: accept absolute URLs against an allow-list of hosts. Not offered: the relative rule covers
same-site navigation, and an app with several front-end origins can map a short key to a URL in its handler.

### 11. Limits and defaults (security-sensitive)

| Setting | Default | Bound / startup check | Original |
|---|---|---|---|
| Grant type | authorization code only | no API for implicit or password | same strategy, other packages exist |
| PKCE | `S256`, on | `plain` not expressible; `.pkce(false)` warns | off unless `pkce` and `state` |
| `state` | required | cannot be disabled | off (`NullStore`) |
| `state` / verifier entropy | 32 bytes from the OS generator each | fixed | 24 bytes (`uid2`) |
| State storage | none: must choose `state_cookie` or `state_in_session` | startup error when missing | session only |
| State lifetime (`state_ttl`) | 600 s | 60 s to 3600 s | session lifetime |
| State cookie | `__Host-oauth2.<name>`, `Path=/`, `HttpOnly`, `Secure`, `SameSite=Lax`, `Max-Age` = lifetime | `secure(false)` warns and drops the prefix; name validated | n/a |
| State cookie keys | `CookieKeys` (#44) | each secret at least 32 bytes (startup error, enforced by #44) | n/a |
| Redirect URI | required, exact | absolute, `https` or loopback `http`, no fragment or user info | relative allowed, resolved from the request |
| Endpoint URLs | presets or required | `https`, or `http` only to loopback; no user info, no fragment | any |
| Client id / secret | required | non-empty; never printed | required |
| Client authentication | request body | `Basic` optional | request body |
| Scope | none (Google preset: startup error when empty) | each token 1 to 128 visible ASCII characters without space, `"` or `\`; at most 64 tokens | any |
| Extra authorization parameters | none | at most 16; names and values at most 256 bytes; reserved names (`response_type`, `client_id`, `redirect_uri`, `scope`, `state`, `code_challenge`, `code_challenge_method`) are a startup error | any |
| Callback `code` length | 2048 bytes | 16 to 8192 | none |
| Callback `state` | exactly 43 base64url characters | fixed | none |
| Callback `error` length | 64 bytes for the reported code | fixed | none |
| `return_to` | off | relative-path rule, 1024 bytes | not provided |
| `failure_redirect` | off | relative-path rule or absolute `https` | `failureRedirect` unchecked |
| Redirects followed by the client | 0 | cannot be changed | followed by some clients |
| Connect timeout (TCP + TLS) | 5 s | 1 s to 30 s | none |
| Total timeout per request | 10 s | 1 s to 60 s | none |
| Token response size | 64 KiB | 1 KiB to 1 MiB | none |
| Profile response size (each request) | 256 KiB | 1 KiB to 4 MiB | none |
| Response header block | 16 KiB | fixed | Node's 16 KiB |
| Requests to the provider in flight, per strategy | 32 | 1 to 1024; over the limit → 503 at once | none |
| TLS | 1.2 and 1.3, web PKI roots (+ extra roots), host name verified | verification cannot be disabled | Node defaults |
| Proxy from environment | ignored | none | honored by some clients |
| Tokens kept by the framework | none | nothing is written to a session, cookie or log | none |

An option outside its bound is a startup error that names the option and the bound.

### 12. OpenID Connect ID token validation is out of scope

Google returns an `id_token` with the `openid` scope. Validating it needs: the provider's JWKS fetched over HTTP and
cached with rotation, RS256 verification against a key chosen by `kid`, `iss` / `aud` / `azp` / `exp` checks and a
`nonce` bound to the state record. The JWT strategy (#42) verifies with a configured key or a key provider and
explicitly excludes JWKS (its Non-Goals), so this is not "trivial with the existing JWT code": it is a key
distribution client plus a new failure surface. It is also not needed for the flow's security here: the profile comes
from the userinfo endpoint over TLS, authenticated by the access token the server itself obtained from the token
endpoint with its client secret and PKCE verifier. So the ID token is handed to the validator as
`grant.params.id_token()`, documented as unverified ("do not read claims from it; use the profile"), and OpenID
Connect is proposed as a new issue.

### 13. Refresh

`OAuth2Client::new(options)?` validates the options like the strategy does and exposes
`refresh(&self, refresh_token: &str) -> Result<OAuth2TokenSet, OAuth2Error>`: `grant_type=refresh_token`,
`refresh_token`, the client credentials (decision 6) and the scope only when `.refresh_scope(..)` is used, through
the same client and limits. `OAuth2TokenSet { access_token, refresh_token, params }`. An application builds the
client from the same `options()` as its validator (a provider with `use_factory`, or a field of the validator).
Nothing is stored and no refresh happens automatically.

Alternative: `Authenticator::refresh("google", token)`, looking the strategy up by name (the spike's wording). It
needs a downcast from `Arc<dyn Strategy>` or a new method on the `Strategy` trait; a separate value built from the
same options is simpler and keeps the core untouched (Open question 5).

### 14. Fixture and tests

Added to `crates/corvis/tests/fixtures/auth/package.json` with exact versions: `passport-oauth2` 1.8.0,
`passport-google-oauth20` 2.0.0, `passport-github2` 0.1.12, `passport-oauth2-refresh` 2.2.0. `oauth2.mjs` starts, on
`127.0.0.1` only, a fake provider (authorization, token, userinfo, GitHub `/user` and `/user/emails`, each driven by
a scenario header or path) and the original app with the three strategies pointed at it (`state: true`, `pkce:
true` with an in-memory session for the cases that need them), and writes `oauth2.json`:

- the authorization redirect: status, `Location` parameters in order (values of `state` and `code_challenge`
  replaced by placeholders), for generic, Google and GitHub, with and without scope and extra parameters;
- the token request the fake provider received: method, path, the compared headers (`content-type`, `accept`,
  `authorization` presence), the form body with `code_verifier` and secrets replaced by placeholders;
- the profile requests (path, `authorization` scheme, `user-agent`, `accept`) and the profile object handed to
  `validate()` for recorded Google and GitHub documents (with and without e-mails, with `user:email`);
- callback outcomes: success, `access_denied`, another `error`, state mismatch, token endpoint 400 / 500 / 302 /
  invalid JSON / no `access_token`, profile endpoint 500, validator returning `false`;
- the refresh request body.

All client ids, secrets, codes and tokens in the fixture are fake fixed strings. Cases the design changes carry a
`divergence` note and are asserted from the spec (the `strategies.json` convention).

`crates/corvis/tests/auth_oauth2.rs` starts its own mock provider (a `tokio` listener speaking HTTP/1.1 through
`hyper`'s server connection, bound to `127.0.0.1:0`, scripted per test: status, headers, body, delay, body size) and a
Corvis app with the strategies pointed at it, replays every recorded case, and adds the cases the original cannot
record: forged, missing, expired and replayed state in both storage modes; cookie attributes; a state cookie signed
for another strategy; `return_to` values; the limits (oversized body with and without `Content-Length`, a response
that never ends, a redirect, the concurrency limit released by a channel, not by sleeping); `http` to a non-loopback
host refused; a TLS mock with an `rcgen` certificate (dev-dependency, `Cargo.toml:69`) accepted only as an extra
root; secrets absent from `Debug` output and from captured log lines. Time moves through a crate-private clock, as in
#44; no test sleeps to make a state expire.

### 15. Example

`examples/cats-app`: enables `auth-oauth2`; a `GithubAuth` validator whose `options()` reads the client id, secret,
redirect URI and (for tests) the endpoint URLs from configuration; `GET auth/github` and `GET auth/github/callback`
with `AuthGuard::new("github")`; the callback handler finds or creates the user by `(provider, id)`, issues the app's
JWT (or logs in to the session of #44) and redirects to `oauth2::return_to(&req)` or `/`. A test runs the whole login
against a mock provider on `127.0.0.1`.

## Risks / Trade-offs

- **A hand-written HTTP client** is code the project must maintain and review. It is small, HTTP/1.1 only, built on
  `hyper`'s client connection and `tokio-rustls`, and covered by tests for every limit; the security review reads it
  line by line. The alternative costs dozens of crates.
- **Depends on designs that are not merged** (#44 `CookieKeys` and `Session`, #45 `redact_query_param`). The first
  task re-checks them; if #45's redaction API changed, the callback redaction is adapted or dropped with a note.
- **Cookie mode is not strictly single use** on the server (decision 4). Mitigated by the cookie deletion, the
  lifetime, the provider's single-use code and PKCE; session mode gives the strict guarantee.
- **The verifier is readable in the signed cookie** by whoever controls the browser's cookie jar; that party can
  already log in as itself.
- **`Secure` and `__Host-` by default** break a plain-HTTP development setup until `.state_cookie_secure(false)` is
  set (browsers differ on `Secure` cookies over `http://localhost`). The startup log states the cookie attributes.
- **401 instead of 500** for provider errors and rejected codes hides a wrong client secret behind a 401
  (`invalid_client`). Mitigated: a warning log line with the sanitized provider code on every rejected exchange.
- **GitHub e-mail request** adds a third request per login when the scope asks for it; its failure fails the login.
- **Bundled roots** (`webpki-roots`) are updated by dependency upgrades, not by the operating system.
- **Profile parity is pinned** to the recorded package versions; a provider that changes its userinfo document shows
  up as missing fields, not as an error.
- **No connection reuse**: each login opens two or three TLS connections to the provider. Acceptable for a login.

## Open questions

1. **State storage must be chosen explicitly** (decision 4) instead of the spike's automatic rule. Confirm, or make
   the signed cookie the default when `CookieKeys` are given through a shared provider.
2. **401 for provider errors other than `access_denied` and for a rejected code** (decision 9) instead of the
   original's 500. Confirm the two divergences.
3. **Redirect URI from the request host** is not offered at all (decision 5). The brief allowed "unless explicitly
   enabled"; the safe form (a list of allowed URIs matched to the host) is proposed as a follow-up issue. Confirm.
4. **Client authentication default**: request body (parity with the original and the providers' docs) versus Basic
   (RFC 6749's preference).
5. **Refresh API**: `OAuth2Client::new(options)` (decision 13) versus a lookup by strategy name on `Authenticator`.
6. **Extra root certificates** are given as DER bytes; a PEM helper would need a parser the crate does not have
   today outside `jsonwebtoken`. Confirm DER is enough, or name the PEM parser to use.
7. **Default limits**: state lifetime 600 s, timeouts 5 s / 10 s, 64 KiB / 256 KiB bodies, 32 requests in flight.
8. **Change name**: the issue and `ROADMAP.md:101` say `e3-generic-oauth2-google-and-github`; this folder is
   `e3-oauth2-strategies`. One of the two should be updated by the maintainer's session.
9. **`failure_redirect` in the strategy** although the core left `failureRedirect` out (its Non-Goals): here it is a
   fixed, startup-checked URL used only for callback failures. Confirm it belongs in this change.

## Review notes

These settle the open questions and override the design where they differ.

1. The `state` storage is chosen explicitly (signed cookie or session); not choosing is a startup error.
2. The two differences that answer 401 (a provider `error` other than `access_denied`, and a rejected code) are
   accepted and listed as differences.
3. The redirect URI is absolute, exact and configured. Deriving it from the request host is not offered; an
   allowlist of redirect URIs is a follow-up issue, opened when this change is archived.
4. Client authentication defaults to the request body; `ClientAuth::Basic` is opt-in.
5. Refresh is `OAuth2Client::new(options)`, not a lookup by name in `Authenticator`.
6. Extra trust roots are given in DER only. No PEM helper and no new dependency for it.
7. The default limits are confirmed: `state` lifetime 600 s, timeouts 5 s (connect) and 10 s (total), response
   bodies 64 KiB (token) and 256 KiB (profile), 32 requests in flight.
8. The change keeps the name `e3-oauth2-strategies`; `ROADMAP.md` is updated to match.
9. `failure_redirect` belongs to this change. It is validated at startup with the documented relative-path rule or
   as an absolute URL given in configuration, never built from request data.
10. Implemented after #44 and #45 land: the first task compares this design with the merged code and records the
    adjustments in Implementation notes. The ID token is handed over unverified and the docs say so in a warning.
11. The security review is mandatory before the merge (state binding, PKCE, the HTTP client's TLS and limits,
    redirect handling, secrets in logs).

## Implementation notes

### Design against the merged code (task 1.1)

The design's line references are not repeated: the files moved. These are the APIs the code was written against.

| Design says | Merged code |
|---|---|
| `Request::redact_query_param("code")` in the callback | Replaced by the core: `Strategy::credential_query_params()` (`crates/corvis/src/auth/strategy.rs`), collected once at startup by `auth::credential_query_params` and applied to every request of the application. `OAuth2Strategy` returns `["code", "state"]`; `LoggingInterceptor` and `Request::redacted_url()` hide them. A middleware that logs `Request::url()` does not (the cats example's now logs `redacted_url()`). |
| `GuardPhase::is_active` is `pub(crate)` | It was private to `common/context.rs`: made `pub(crate)` (no behavior change). |
| `CookieKeys` (`sign`, `unsign`) | `crates/corvis/src/http/signed_cookie.rs`, `corvis::http::CookieKeys`: `sign` gives `s:<value>.<signature>`, `unsign` verifies against every key and refuses more than 4096 bytes, `CookieKeysError` for a short secret; `Debug` prints the number of secrets only. |
| `Request::session()`, `Session::set` / `get` / `remove` | `request.rs` (`session()`), `auth/session/session.rs` (`get_value`, `set`, `remove`). A guard that stores a redirect is not a rejected request: the record written at login start is committed (tested). |
| "the session cookie's `SameSite` accessor" | `SessionManager::options.cookie.same_site` (crate-private fields); the strategy resolves `SessionManager` through the DI context, which also gives the "SessionModule is not imported" error. |
| a crate-private clock "as in #44" | `session::store::now_ms()`, re-exported `pub(crate)` for the feature: it follows tokio's paused clock, so the expiry tests move time with `tokio::time::advance`. |
| `serde_json::Value` for the provider's JSON | `PlainJson` lives in `crate::jwt`, which needs the `jwt` feature; `oauth2/json.rs` has its own visitor (no `$serde_json::private::RawValue` key handling, so the parser's depth limit is the only budget even with `raw_value` on). |
| empty users | `AuthenticatedUser::is_empty`: a validator returning a user that serializes to `null`, `false`, `0` or `""` is a 401 (tested); the strategy never sets `return_to` for such a user. |
| strategy panics, names, `inner_guards`, `WarnLimiter` | Used as merged: the callback's log lines go through a `WarnLimiter` (one line a minute per reason). |
| Open questions | Settled by the Review notes above. |

### Decisions and deviations

1. **Several logins in progress per browser.** The design keeps one record per strategy, so a second tab breaks the first. The record is a list of at most four flows (`state`, issue time, `return_to`, plus the verifier in the session form). A callback takes out the flow of its own `state`, comparing every stored state in constant time, and writes the others back. A forged or unknown state spends nothing: the cookie is rewritten with the flows it had. So scenario "State cookie deleted" holds for every callback that matches a flow, and a callback that matches none leaves the others alone instead of deleting them (a cross-site link could otherwise abort a user's login in progress). The fifth login drops the oldest. Two tabs that start at the same instant can still overwrite each other's `Set-Cookie`: the browser keeps the last one, a limit of cookie storage.
2. **The PKCE verifier is not in the cookie.** Cookie storage derives it: base64url SHA-256 of the signature (first cookie key) of `corvis-oauth2-pkce-v1|name|state|issued`. The browser knows the `state` but not the key, so the verifier never leaves the server. Cost: a key rotation between the redirect and the callback fails that one login. Session storage keeps a random verifier in the session record. Scenario "Record contents" holds for the session form; the cookie form holds the name, state, issue time and `return_to`.
3. **GitHub joins its scope with commas** (`scopeSeparator: ','` in `passport-github2`), where the design said a space. The separator belongs to the preset and a scope value cannot contain it.
4. **Parameter order and form field order come from the recording.** Authorization: the URL's own parameters, the extra ones, `response_type`, `redirect_uri`, `scope`, `code_challenge`, `code_challenge_method`, `state`, `client_id` (the design listed `client_id` second). Token request: `grant_type`, `redirect_uri`, `code_verifier`, `client_id`, `client_secret`, `code`; refresh: `grant_type`, `client_id`, `client_secret`, `refresh_token`.
5. **A 200 token answer with an `error` and no `access_token` is a rejection (401)**, GitHub's way to refuse a code (the original answers 500). A token endpoint answering 400 or 401 is a rejection too, as designed.
6. **`OAuth2TokenResponse` holds no token**: `access_token`, `refresh_token` and `id_token` are removed from the members and typed (`OAuth2Token`), so no serializer reaches them through `get()`. The original keeps `access_token` and `id_token` in `params`. Each token is limited to 16 KiB.
7. **`OAuth2Options::state_cookie_secrets(..)`** (not in the design) builds the keys from raw secrets and reports a missing or short one as a startup error that names the option, because `options()` cannot return an error. `state_cookie(keys)` stays.
8. **GitHub e-mails.** With `user:email` the profile keeps every `/user/emails` entry as `{ value, primary, verified }` (the recorded original with `allRawEmails: true`, minus `visibility`); the original's default keeps only the primary one as `{ value }`. An empty list or no usable entry fails the login, as in the original. `id` accepts a number or a string of digits only.
9. **`return_to_param` cannot be `code`, `state` or `error`** (the callback's own parameters). A request that is not a `GET` is a plain 401 and is not redirected to the failure page (it is not a callback).
10. **Provider answers are strict about content types**: JSON (`application/json`, `+json`) or form / `text/plain` for the token response, JSON for profiles; a `Content-Encoding` other than identity is refused; a form body with a repeated key is refused.
11. **Verified e-mail is a JSON boolean.** Google's `email_verified` as a string, a number or `null` is not verified.
12. **Session storage and cookie tossing.** With the session as the store, login CSRF needs the victim's browser to carry a session cookie planted by the attacker (a sibling subdomain, or a network attacker over plain HTTP); the cookie store, with its `__Host-` cookie, has no such exposure. The docs recommend the `__Host-` prefix and `Secure` for the session cookie when `state_in_session()` is used; the framework cannot enforce it.
13. **Example.** `examples/cats-app` mounts the GitHub login only when `GITHUB_CLIENT_ID` is set (its other tests and its startup are unchanged), answers the callback with the application's JWT as JSON instead of a redirect (a token in a redirect URL ends up in logs and history), and its request-log middleware now logs `redacted_url()`. Endpoints come from `GITHUB_ENDPOINTS_BASE`, which only tests set.
14. **"Loopback name resolving elsewhere"** is covered by a unit test of the address rule (`addresses_allowed`), not by an integration test: a name cannot be made to resolve to a non-loopback address without touching the resolver.
15. **Tests**: `crates/corvis/tests/auth_oauth2.rs` (behavior), `auth_oauth2_original.rs` (the recorded cases), `auth_oauth2_log.rs` (own binary, process-wide logger), the unit tests of `auth/oauth2/*`, and `examples/cats-app/tests/github_login.rs`. The mock provider (`tests/oauth2_common/mod.rs`) is raw HTTP/1.1 over TCP on `127.0.0.1`, with TLS from `rcgen` for the certificate cases.

### Left to the maintainer's session

Task 1.3 (the CI run in a pull request), 9.6 (parity: the only core change is a visibility, `GuardPhase::is_active`, plus a feature-gated re-export), 9.8 (the security review), the archive, the follow-up issues and the `ROADMAP.md` name.

## Security review (round 1)

The review ran a proof of concept for all eight areas. `state`, PKCE, `redirect_uri`, return-to, the HTTPS client, JSON
parsing, logs and the session interaction held. The maintainer decided these fixes.

- **M1 (Medium), one client exhausting the in-flight slots.** In cookie mode a flow record stays valid in the browser
  if it ignores the clearing cookie, so one `state` could be presented again and again, each copy taking a slot (and
  spending the application's quota at the provider): more than 32 concurrent copies made every login answer 503.
  Fixed three ways. (a) Each strategy instance keeps a bounded set of the verified states it presented to the token
  endpoint (SHA-256 of the state, a hash set and an insertion-ordered queue, retained for `state_ttl` plus the accepted
  clock skew, `redeemed_states_max` entries, default 100000, 16 to 10 million; the oldest are dropped when full; O(1)
  amortized per callback). A callback whose state is in the set, including one being redeemed now, gets the uniform
  "state mismatch" failure before any token request. Only states that matched the flow record enter the set, so
  callbacks with forged states grow nothing. This is a cost control per instance: a second instance behind a load
  balancer has its own set, session storage already removes the record, and a deployment with several instances
  that needs a guarantee needs session storage with a shared store. (b) The slots one client address can hold at once
  are bounded to a share of `max_in_flight` (`max_in_flight_per_client`, default a quarter, at least 1, 1 to 1024),
  keyed by the address the framework resolves for the request (`Request::ip`, so `trust proxy` is honoured), normalized
  as the throttler does (IPv6 to its /64, IPv4-mapped to IPv4; a value that is not an address falls back to the
  socket peer). The slot is taken before the state is claimed and held through the token and profile requests, so a
  refusal burns nothing of the flow and a released slot leaves no entry. An address over its share gets the 503
  while others proceed. (c) The docs ask to throttle the callback route per client as well as the login route and
  explain both caps.
- **M2 (Medium), `provider` collided across instances.** `provider` was fixed to `"github"` or `"google"` even when
  `endpoints()` pointed the preset at another instance (GitHub Enterprise), so `(provider, id)` was shared between
  github.com and the enterprise server. Now `provider` is the preset's name only when all of the preset's endpoints are
  unchanged; for any replaced endpoint and for a generic strategy it is the strategy's name, unique in the application
  (checked at startup). `OAuth2Profile::issuer` is the host (and port) of the authorization endpoint. The `issuer`
  is not part of the JSON form, which keeps the original's keys. A strategy named like its preset (`github`) with
  replaced endpoints therefore still gets `github`: name strategies of other instances differently. The docs and the
  cats example key accounts by `(provider, id)` with that rule and warn against keying by e-mail alone.
- **L1 (Low), cookie tossing.** A state cookie without the `__Host-` prefix, and a session cookie without it under
  `state_in_session()`, now log one startup warning each, naming the strategy and the cookie, saying a sibling
  subdomain can plant a cookie (login CSRF) and how to fix it. Not an error: plain-HTTP development and shared
  domains are legitimate.
- **L2 (Low, example).** The cats example's exception filter answered `request.url()` and echoed the callback's `code`
  and `state`; it now answers `redacted_url()` (tested). The strategies also declare `error_description` and
  `error_uri` as query parameters to hide: they are no credentials, but they are text the other side of the redirect
  chose, and a log line or an error body that prints the redacted URL must not reflect them.
- **I1.** Flow expiry used unchecked arithmetic and overflowed for a timestamp of `i64::MIN`. It now uses checked
  arithmetic and treats an impossible timestamp as an invalid flow.
- **I2.** A validator returning `None` or an empty user answered 401 even with `failure_redirect`. With a failure
  page every refusal of the callback goes there; without one it stays a 401. The reason is not shown to the client.
- **I5 (docs).** The profile `id` is the provider's value as a string, compared exactly (`"00012"` and `"12"` differ);
  the docs recommend storing it as given.

Tests: `crates/corvis/tests/auth_oauth2_review.rs` (M1, M2, I1, I2), the unit tests of `redeem.rs`, `options.rs` and
`state.rs`, `auth_oauth2_log.rs` (L1) and `examples/cats-app/tests/github_login.rs` (L2). `auth_oauth2.rs`: the
cookie-mode replay test now expects no second token request.

## Security review (round 2)

The second pass could not get around the round 1 fixes (200 concurrent callbacks of one flow cost one token request
and no 503; a third party with a victim's `state` but not the cookie cannot burn it; spoofed forwarding headers do not
multiply the per-address share; the slot counters return to zero after thousands of cycles over every exit path) and
judged the change ready to merge. What it noted, and what was done:

- **Low: a 404 answered the callback's query string.** The router builds the message of its `NotFoundException` from
  the URL (`Cannot GET /path?...`); a callback URL that misses its route (a typo in the path, a `POST`) got its `code`,
  `state` and error text back in the body. The router now uses `redacted_url()` there, which is the same text when the
  application registered no credential parameter.
- **Low: a strategy named like a preset that is not the preset shared the preset's id space.** A generic strategy (or a
  preset pointed at another server) named `github` produced `provider = "github"`, the pair github.com itself yields
  for the same number. Such a strategy now gets `name@issuer` (`provider_namespace` in `options.rs`), with a warning at
  startup that names the value. Refusing to start was considered and not chosen: the cats example and the tests of
  this change name their strategy `github` and put a test double behind it. Round 1's sentence "`(provider, id)` never
  collides" held only for strategies with names of their own; the docs, the option's rustdoc and the example's comment
  now state the rule.
- **Retention boundary of the redeemed set.** An entry was dropped in the second it expired in while a flow is still
  accepted at exactly that age, so a state could pass once more at that instant. Entries are kept through that second.
- **Prefix check of the state cookie name** compared `__Host-` / `__Secure-` with case; browsers do not. It ignores
  case now, like the warning.
- Documented, not changed: the global in-flight 503 happens after the state was claimed, so that flow must be started
  again, while the per-address 503 claims nothing on the server (both clear the browser's state cookie); with
  `trust_proxy(true)` a client chooses its own address key through `X-Forwarded-For`, as for the throttler, so a hop
  count is the setting to use; four addresses at their share fill the default cap of 32, which is inherent to a
  global cap; `issuer` is the host of the authorization endpoint, from configuration.

Tests: `a_strategy_named_like_a_preset_that_is_not_the_preset_warns_and_names_its_provider` (`auth_oauth2_log.rs`),
the provider assertions of `auth_oauth2.rs`, `auth_oauth2_original.rs` and `auth_oauth2_review.rs`, the unit tests of
`redeem.rs` and `options.rs`, and `a_callback_url_that_misses_its_route_does_not_answer_its_query_back`
(`auth_oauth2_review.rs`).

