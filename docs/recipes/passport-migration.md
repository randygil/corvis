---
status: documented
---

# Migrating from Passport

This page is for someone holding a Node application that authenticates with Passport and moving it to Corvis, possibly a
route at a time. It maps each class, option and call of the Node packages to Corvis, lists what is not provided, lists every
behavior that answers differently on purpose and why, and explains how to run both applications side by side while users keep
their sessions. The options, defaults and limits themselves are in the reference, [Passport](/recipes/passport); each row below
links to it. Corvis has the same strategy model and its behavior is recorded against the original libraries (`passport`
0.7.0, `passport-jwt` 4.0.1, `passport-local` 1.0.0, `passport-headerapikey` 1.2.2, `passport-custom` 1.2.1, `express-session`
1.19.0, `passport-oauth2` 1.8.0 and the Google and GitHub profiles, `bcryptjs`), with no Node needed at test time.

The framework integration of Passport is called here "the original passport integration".

## Before you start

**What carries over unchanged**

- The names of the strategies you give to the guard: `AuthGuard::new("jwt")`, `"local"`, `"session"`. The API key strategy is
  the exception: Corvis names it `"api-key"`, and `const NAME: &'static str = "headerapikey";` in the validator keeps the old
  name.
- The answer to a request nobody authenticated: `401 {"message":"Unauthorized","statusCode":401}`, with no `WWW-Authenticate`
  header, whatever the strategies attached to their failure.
- `req.user` and `req.authInfo`: `Request::user()` (JSON) and the `User<T>` parameter, and `Request::auth_info()`.
- The format of the session cookie and of the session records, so both applications can read the same sessions, and the
  `$2a$`/`$2b$` bcrypt hashes, which verify as they are.

**What you must decide first**

- **Secret lengths.** Corvis refuses a secret under 32 bytes for an HMAC token key, a signed cookie and an API key, at startup,
  with an error that names the setting and never the value. If a Node application uses shorter ones, the migration is also the
  moment to rotate them (see [Running both side by side](#running-both-side-by-side)).
- **Token expiry.** Tokens need an `exp` claim by default. Make the Node side issue one before moving the verification.
- **Cookie flags.** The Corvis session cookie is `Secure`, `HttpOnly` and `SameSite=Lax` unless you say otherwise; the Node
  default is none of them.

## Mapping tables

### The original passport integration

| Original | Corvis |
|---|---|
| `class JwtStrategy extends PassportStrategy(Strategy)` with `validate(payload)` | a validator (`JwtValidator`, `LocalValidator`, ...) registered with `Provider::strategy::<JwtStrategy<YourValidator>>()` |
| `@UseGuards(AuthGuard('jwt'))` | `#[use_guards(AuthGuard::new("jwt"))]` |
| `AuthGuard(['jwt', 'apikey'])` | `AuthGuard::chain(["jwt", "api-key"])` |
| `AuthGuard()` with `PassportModule.register({ defaultStrategy })` | `AuthGuard::default()` with `PassportModule::register(PassportModuleOptions::new().default_strategy("jwt"))` |
| `handleRequest(err, user)` returning `user \|\| null` | `.optional()` |
| `handleRequest` throwing another exception | `.on_failure(\|failures\| UnauthorizedException::new("Token required"))` |
| `getRequest` (to read the request from a non-HTTP context) | not needed: the guard reads the HTTP request itself, and denies other contexts |
| `if (isPublic) return true` before `super.canActivate` | `.skip_public()`, with `#[public]` on the routes |
| `req.user` | `Request::user()` or the `User<T>` parameter |
| `req.authInfo` | `Request::auth_info()` |
| a `@User()` parameter decorator | the `User<T>` parameter |

The strategy class becomes a **validator**: the same `validate`, with the options in `options()` instead of the constructor.
For JWT:

```js
export class JwtStrategy extends PassportStrategy(Strategy) {
  constructor(config) {
    super({
      jwtFromRequest: ExtractJwt.fromAuthHeaderAsBearerToken(),
      secretOrKey: config.get('JWT_SECRET'),
    })
  }
  async validate(payload) {
    return { id: payload.sub, username: payload.username }
  }
}
```

```rust
// examples/auth-app/src/auth/jwt.rs
#[async_trait]
impl JwtValidator for JwtAuth {
    type User = Account;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key(JwtKey::secret(self.settings.jwt_secret.clone()))
            .algorithms([JwtAlgorithm::HS256])
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Account>, HttpException> {
        // ...
        let claims: Option<Claims> = corvis::serde_json::from_value(payload).ok();
        Ok(claims.map(|c| Account { id: c.sub, username: c.username, roles: c.roles }))
    }
}
```

`validate` returns the user (`Some`), no user (`None`: the `401`) or an exception (passed on as `done(err)` is). **Return
`None`, not a value that serializes to `null`, `false`, `0` or `""`**: the original treats a falsy user as a failure, and
Corvis does too, with a warning that names the validator. For local:

```js
export class LocalStrategy extends PassportStrategy(Strategy) {
  constructor(users) { super(); this.users = users }
  async validate(username, password) {
    const user = await this.users.findOne(username)
    if (!user || !(await bcrypt.compare(password, user.hash))) throw new UnauthorizedException()
    return user
  }
}
```

```rust
// examples/auth-app/src/auth/local.rs
#[async_trait]
impl LocalValidator for LocalAuth {
    type User = Account;

    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<Account>, HttpException> {
        let found = self.users.credentials(username);
        let stored = found.as_ref().map(|(_, hash)| hash.as_str());
        // ...
        let ok = self.hasher.verify_or_dummy(password, stored).await.map_err(PasswordError::into_http_exception)?;
        let Some((account, hash)) = found.filter(|_| ok) else { return Ok(None) };
        // ...
        Ok(Some(account))
    }
}
```

A wrong password is `Ok(None)`, not an exception: both end in the same `401`, but an exception you build yourself can
differ per user.

### `passport-jwt`

| `passport-jwt` | Corvis |
|---|---|
| `jwtFromRequest: ExtractJwt.fromAuthHeaderAsBearerToken()` and the other extractors | `.jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())`; there is no default place |
| `secretOrKey`, `secretOrKeyProvider` | `.secret_or_key(JwtKey::secret(..))`, `.secret_or_key_provider(..)` |
| `issuer`, `audience`, `algorithms` | `.issuer(..)`, `.audience(..)`, `.algorithms([..])` |
| `ignoreExpiration: true` | `.ignore_expiration(true)`, with a startup warning |
| `passReqToCallback` | always on: `validate(req, payload)` |
| `jwt.sign(payload, { expiresIn })` (`jsonwebtoken`) | `JwtService::sign` with `JwtModuleOptions::expires_in(..)`, or `sign_with(.., JwtSignOptions { expires_in, .. })` |
| `jwt.sign(payload, { notBefore, jwtid })` | `JwtSignOptions { not_before, jwt_id, .. }` |

`sign` and `sign_with` follow the rules of the original JWT module (11.0.2 over `jsonwebtoken` 9.0.3), checked against a
recording of it:

- An object payload's `iat`, `exp` and `nbf` must be numbers; otherwise the error is `"iat" should be a number of seconds`.
- `iat` is the payload's when it is a number other than `0`, and the clock otherwise. `nbf` (`not_before`) and `exp`
  (`expires_in`, per call or from the module) are counted from that `iat`. A bare number in `expires_in` or `not_before`
  is seconds.
- A payload claim together with its option is an error; nothing is overwritten. The pairs are `exp` and `expires_in`, `nbf`
  and `not_before`, `aud` and the module's `audience`, `iss` and the module's `issuer`, `sub` and `subject`, `jti` and
  `jwt_id`. The error texts are the original's, for example `Bad "options.subject" option. The payload already has an "sub" property.`
- The claims are added after the payload's own keys, in the order `iat`, `nbf`, `exp`, `aud`, `iss`, `sub`, `jti`.
- A string payload is signed as it is, with a header without `typ`, and is refused when any sign option is set
  (`Payload as string is not allowed with the following sign options: expiresIn`). A number or a boolean is signed as its
  JSON text, also without `typ`. An array or `null` is refused (`Expected "payload" to be a plain object.`).
- One deliberate difference: a string payload whose text is itself a JSON object or array is refused. The original signs
  it, and the text becomes the claims of the token, so signing a string that a client chose (an e-mail address, a link)
  would hand that client a token for any subject. Pass the object itself.

`not_before` and `jwt_id` are public fields of `JwtSignOptions`, so a struct literal needs `..Default::default()`. Earlier
versions of Corvis overwrote `exp` and `sub`, and let the payload win for `iss` and `aud`.

Every option, the extractors and the algorithm rules are in [the JWT reference](/recipes/passport#the-jwt-strategy).

### `passport-local` and `bcryptjs`

| Original | Corvis |
|---|---|
| `usernameField`, `passwordField` | `LocalStrategyOptions::default().username_field("email").password_field("pass")` |
| `passReqToCallback` | always on: `validate(req, username, password)` |
| `done(null, false)` | `Ok(None)` |
| `bcrypt.hash(password, 10)` | `hasher.hash(password).await` (argon2id by default; `PasswordHasherOptions::bcrypt(10)` keeps bcrypt) |
| `bcrypt.compare(password, hash)` | `hasher.verify(password, hash).await`, or `verify_or_dummy(password, Option<&str>)` so an unknown user costs the same |
| `bcrypt.compareSync`, `hashSync` | `verify_blocking`, `hash_blocking`, **only for code outside the Tokio runtime** (inside it they return an error) |
| a check that the hash is still current | `hasher.needs_rehash(&stored)`, then `hash` and save it after a successful login |

The hasher, the algorithms and the limits are in [Encryption and hashing](/security/encryption-and-hashing); the credential
places and limits of the strategy are in [the local reference](/recipes/passport#local-strategy-username-and-password).

### `passport-headerapikey` and `passport-custom`

| Original | Corvis |
|---|---|
| `new HeaderAPIKeyStrategy({ header: 'X-Api-Key', prefix: '' }, false, verify)` | `ApiKeyStrategyOptions::default()` (the `X-Api-Key` header), or `.source(ApiKeySource::header("Authorization").prefix("Api-Key "))` |
| the strategy name `headerapikey` | `"api-key"`, or `const NAME: &'static str = "headerapikey";` to keep it |
| `verify(apiKey, done)` looking the key up | `validate(req, &ApiKey)`, comparing a digest (`ExpectedApiKey`, `ApiKeySet`), never `==` |
| `passport-custom`: `new CustomStrategy((req, done) => ...)` | a type that implements `Strategy` (`authenticate` returns a `StrategyOutcome`); see [Coming from `passport-custom`](/recipes/passport#coming-from-passport-custom-and-passport-strategy) |
| a key read from the query string | `ApiKeySource::query("api_key")`, opt-in, with a startup warning |

### `express-session` with `passport.session()`

| Original | Corvis |
|---|---|
| `app.use(session({ secret, name, cookie, resave, saveUninitialized, rolling, store }))` | `SessionModule::for_root(SessionOptions::with_secrets([..])?.name(..).cookie(SessionCookie::..).resave(..).save_uninitialized(..).rolling(..).store(..))` |
| `app.use(passport.session())` | the middleware of `SessionModule` and `SessionStrategy<YourSerializer>`, the strategy named `"session"` |
| `passport.serializeUser(fn)` | `SessionSerializer::serialize_user` |
| `passport.deserializeUser(fn)` | `SessionSerializer::deserialize_user` (`Ok(None)` when the user is gone) |
| `req.logIn(user)` | `AuthGuard::new("local").log_in()`, or `Authenticator::log_in(&req, &AuthenticatedUser::new(user)?).await?` |
| `req.logout()` | `Authenticator::log_out(&req).await?` |
| `req.isAuthenticated()` | `AuthGuard::new("session")` answers `401` when anonymous (the session strategy of the original always passes) |
| `keepSessionInfo` | `SessionOptions::keep_session_info_on_login(true)` |
| `connect-redis` and other stores | `SessionOptions::store(..)` with a type that implements `SessionStore`; Redis and ORM stores are not provided yet |

See [Sessions](/recipes/passport#sessions) and [Session](/http/session) for the rest.

### `passport-oauth2`, `passport-google-oauth20`, `passport-github2` and `passport-oauth2-refresh`

| Original | Corvis |
|---|---|
| `new GoogleStrategy({ clientID, clientSecret, callbackURL, scope }, verify)` | `OAuth2Options::google(client_id, client_secret, redirect_uri).scope([..])`; `OAuth2Options::github(..)` for GitHub; `OAuth2Options::new(OAuth2Endpoints::new(authorization_url, token_url).userinfo_url(url), ..)` for any other provider |
| `verify(accessToken, refreshToken, profile, done)` | `validate(req, grant: OAuth2Grant)`: `grant.access_token`, `grant.refresh_token`, `grant.params`, `grant.profile` |
| `profile.id`, `profile.emails[0].value` | `profile.id` (compare it with the provider name) and `profile.verified_email()` |
| `state: true`, `pkce: true` with a session | always on; you choose where the state lives: `.state_cookie_secrets([..])` or `.state_in_session()` |
| `callbackURL` as a relative path | an absolute URL, configured: nothing is built from the `Host` header |
| `failureRedirect` on `authenticate('google', ...)` | `.failure_redirect("/login?error=oauth")` on the OAuth2 options (this strategy only) |
| `OAuth2Strategy.userProfile` override for another provider | `userinfo_url` on the endpoints (the profile is the OpenID `userinfo` document) |
| `passport-oauth2-refresh` `requestNewAccessToken` | `OAuth2Client::new(options)?.refresh(refresh_token)`; nothing is stored and nothing refreshes by itself |

The two routes (the login that redirects and the callback) carry the same `AuthGuard::new("google")`. See
[OAuth2](/recipes/passport#oauth2-google-github-and-others).

## Not provided

| In the original | In Corvis |
|---|---|
| flash messages (`failureFlash`, `successFlash`) | not provided. A failed login is the plain `401`; the reason is in `Request::auth_info()` for a handler or `.on_failure(..)` to read, never in the response |
| `failureRedirect` and `successRedirect` on the guard | not provided on `AuthGuard`. Redirect from the handler after a login; the OAuth2 strategy has `.failure_redirect(url)` |
| `passReqToCallback` | not needed: the request is always passed to the validator |
| the `property` option, `assignProperty` | not provided: the user is always stored for `User<T>` and `Request::user()` |
| `session: false` on a guard | the reverse is the default: a guard never writes a session unless it has `.log_in()` |
| `authInfo` shaped like a serialized error | `Request::auth_info()` is JSON with a fixed reason; it never repeats anything from the request |
| `req.login` callbacks, `req.logout(cb)` | `async`: `Authenticator::log_in(..).await?` and `log_out(..).await?` |

## What answers differently, on purpose

Each row is a case the differential tests record from the original and mark as a divergence, plus the startup checks, which
have no request to record. They are the same differences the reference lists next to each strategy, with the reason.

### Strategy core

| The original | Corvis | Why |
|---|---|---|
| a guard that names an unknown strategy answers `500` on each request; `AuthGuard()` without a default only logs | `init()` fails with one error that lists every route and the missing name; two strategies with one name, an empty chain and conflicting defaults fail too | a typo must stop the deployment, not surface as a `500` in production <!-- divergence: strategies.json: Corvis stops startup with an error namin --> |
| `AuthGuard` runs in a WebSocket context, where it does not work either | it denies the message with `Forbidden resource` and the application warns once per gateway | no strategy can read a socket message; failing closed |
| `req.user` is any request property | the typed user lives in its own slot; a later guard that sets its own user makes `User<T>` answer `401` | one guard decides the identity of a request |

### JWT

| The original (`passport-jwt`) | Corvis | Why |
|---|---|---|
| a token without `exp` is accepted, and never expires | refused by default; `.allow_missing_exp()` accepts it and logs a startup warning | a token that cannot expire cannot be revoked by waiting <!-- divergence: jwt.json: exp is required by default; the original --> |
| a non-numeric `iat` passes (it is checked only with `maxAge`) | fails | a malformed claim is not trusted <!-- divergence: jwt.json: a non-numeric iat fails; the original ch --> |
| a payload that is a string is handed to `validate` | the payload must be a JSON object | a validator always reads claims <!-- divergence: jwt.json: a payload that is not an object fails; t --> |
| `fromHeader` finds only names written in lowercase | header names are matched ignoring case | HTTP headers are case-insensitive <!-- divergence: jwt.json: the header name is matched case-insensit --> |
| no limit on the token length | 8192 bytes by default, checked before decoding | bounded work for an unauthenticated request <!-- divergence: jwt.json: the original has no token length limit;  --> |
| `mutatePayload: true` writes the claims into the caller's object | there is no such option: the payload is borrowed and the caller's value never changes (the token is the same) | a function that signs does not change its argument <!-- divergence: jwt_sign.json: Corvis borrows the payload and never cha --> |
| the first cookie of a name wins | a cookie sent twice is no token | a cookie planted from a sibling subdomain must not authenticate |
| the key provider and `validate` wait forever | both have a deadline and answer a `503` that is the same for every token | a slow dependency must not hold every request |
| any secret length, `ES512` | an HMAC secret under 32 bytes stops startup; `ES512` is not available | RFC 7518 section 3.2; `jsonwebtoken` 9 has no `ES512` |
| `crit` headers and malformed segments reach the key provider | refused before it | the provider runs for any well-formed token |

### Local

| The original (`passport-local`) | Corvis | Why |
|---|---|---|
| the username and password are also read from the query string | only with `.allow_query_credentials()`, with a startup warning; by default the credential is missing | a password in a URL is written to logs <!-- divergence: local.json: the original also reads credentials from --> |
| a number or a boolean credential reaches `validate` | a non-string credential is missing | type confusion in a user lookup <!-- divergence: local.json: the original passes the number to valida --> |
| no limit besides the body size | a username or a password over 1024 bytes fails before the validator | bounded work before a hash <!-- divergence: local.json: the original has no limit besides the bo --> |
| what `validate` does decides the answer: a throw for an unknown user and `false` for a wrong password are two answers | missing credentials, a wrong password and an unknown user give the same `401`, and the validator spends one hash verification either way | no user enumeration, by status or by time |

### API keys

| The original (`passport-headerapikey`) | Corvis | Why |
|---|---|---|
| the strategy is named `headerapikey` | `"api-key"` (keep the old name with `NAME`) | consistent names |
| a prefix is turned into an unescaped regular expression: `Key+` removes only `Key` and hands `+<key>` to `verify` | the prefix is removed literally | no surprise from a regular expression <!-- divergence: api_key.json: passport-headerapikey removes the prefix --> |
| a key of any length, with a NUL byte, reaches `verify` (header or bearer source) | over 512 bytes, or with a control character, fails before the validator | bounded work, and no byte that a proxy and the application read differently <!-- divergence: api_key.json: the original has no limit of its own; Co --> <!-- divergence: api_key.json: the original passes a NUL character on t --> |
| one source, or the first one wins | every configured source is read, and two sources that carry different keys fail | a proxy that authenticates by one place cannot be bypassed through another <!-- divergence: api_key.json: the original has one source (or the firs --> |
| only a header | the default is the `X-Api-Key` header only; the query string is a source only when configured, with a warning | keys in URLs are logged |

### Sessions and signed cookies

| The original (`express-session`, `cookie-signature`) | Corvis | Why |
|---|---|---|
| `destroy()` sends no `Set-Cookie` | the cookie is cleared (the browser drops it) and the record destroyed | a stale cookie must not linger <!-- divergence: session.json: express-session sends no Set-Cookie afte --> |
| an unmodified session sends no `Set-Cookie` after a secret rotation | it is sent signed with the current secret | rotation completes without waiting for a write <!-- divergence: session.json: express-session sends no Set-Cookie for  --> |
| `store.destroy` is called for the identifier of a session that was never stored | not called | no store call for nothing <!-- divergence: session.json: express-session calls store.destroy for  --> |
| on logout `passport` saves the session without the user and then regenerates | the old record is destroyed and the new one created, with no intermediate save | one less window with a half-logged-out record <!-- divergence: session.json: passport saves the session without the u --> |
| no `SameSite`, no `Secure`, no timeouts, no size limit | `SameSite=Lax`, `Secure`, idle and absolute timeouts, a size limit per record (see [Divergences](/http/session#divergences-from-the-original)) | safe defaults |
| a login through the guard stays when a later guard or the handler rejects the request | it is discarded when the request is rejected | no half-completed login |
| a signed value of any length is verified | an input over 4096 bytes is refused before any HMAC is computed | bounded work <!-- divergence: signed_cookies.json: the original verifies any length; Corvis --> |

### OAuth2

| The original (`passport-oauth2` and its profiles) | Corvis | Why |
|---|---|---|
| `state` and PKCE only with `state: true`, `pkce: true` and a session | always sent: `state`, `code_challenge` and `code_challenge_method=S256`; no way to turn `state` off | login CSRF and code interception <!-- divergence: oauth2.json: Corvis always sends state and a PKCE cha --> |
| a relative `callbackURL` is resolved against the `Host` header | the redirect URI is absolute and configured | a forged `Host` must not choose where the provider sends the code |
| the token request has the headers of the original's HTTP client | it also sends `Accept: application/json` and its own `User-Agent` | the parts that matter (method, path, content type, form fields in order) are the same <!-- divergence: oauth2.json: Corvis also sends Accept: application/js --> |
| Google's access token goes in the URL of the profile request | a bearer header for every provider, never a token in a URL | URLs are logged <!-- divergence: oauth2.json: the original sends the Google access tok --> |
| GitHub: the primary e-mail as `{ value }`, or all of them with `allRawEmails` | every address as `{ value, primary, verified }`, and a `verified` flag on every e-mail; `verified_email()` | link accounts only by a verified address <!-- divergence: oauth2.json: GitHub e-mails: with user:email Corvis k --> |
| a provider `error` other than `access_denied` answers `500` | `401` | the request caused it, not the server <!-- divergence: oauth2.json: Corvis answers 401 for a provider error  --> |
| the token endpoint rejecting the code (400 or 401) answers `500` | `401` | a reused or forged code is the request's fault <!-- divergence: oauth2.json: Corvis answers 401 when the token endpoi --> |
| a callback with a state that is not this browser's answers `403` through plain `passport` | `401` through `AuthGuard` | the uniform failure |

**Keeping this list current.** The rows come from the `divergence` marks of `crates/corvis/tests/fixtures/auth/*.json`. To
list them (from the repository root), when a later change adds a case:

```sh
node -e "const fs=require('fs');const w=(o,f)=>{if(o&&typeof o==='object'){if(o.divergence)console.log(f,'::',o.name||'','::',typeof o.divergence==='string'?o.divergence:o.divergence.reason||JSON.stringify(o.divergence));for(const k in o)w(o[k],f)}};for(const f of fs.readdirSync('crates/corvis/tests/fixtures/auth').filter(f=>f.endsWith('.json')&&!f.startsWith('package')))w(JSON.parse(fs.readFileSync('crates/corvis/tests/fixtures/auth/'+f,'utf8')),f)"
```

## Running both side by side

Moving one route at a time works when both applications accept the same credentials. Put a reverse proxy in front, send a
path prefix to Corvis and the rest to Node, and move one prefix per release.

**Sessions.** Keep the cookie name `connect.sid` while both applications run: a session cookie named `__Host-sid` on the
Corvis side would not be sent to the Node side. The cookie is read by each side when it has:

- the same name, and the same secrets: each at least 32 bytes in Corvis, the first one signs and all of them verify. A Node
  secret that is shorter has to be replaced first. List the new secret first on the Node side and wait until the cookies signed
  with the old one expire (the absolute timeout), then give Corvis the list without the short secret; users whose cookies were
  signed with it log in again;
- a store both can reach, with the record format of `express-session`: the `passport.user` key holds what `serializeUser` returned,
  and the extra `__corvis` key is ordinary data to Node. Corvis has no Redis store yet (#170), so write a `SessionStore` over the
  Redis or database your Node application already uses, with the same key prefix and expiry rules;
- the same shape stored for the user: if Node stores the bare id (`7`), `serialize_user` returns `json!(7)` and
  `deserialize_user` reads a number. Wrap an id that could be `0` in an object on both sides, or Corvis will not restore it
  (a user that serializes to `0`, `""`, `false` or `null` is never authenticated).

When the last route has moved, rename the cookie to `__Host-sid` and let users log in once more. See
[Cross-reading with a Node app](/http/session#cross-reading-with-a-node-app).

**Passwords.** Hashes made by `bcryptjs` verify in Corvis as they are. Verify with `verify_or_dummy` and call `needs_rehash`
after a successful login: the stored hash becomes argon2id, and **Node can no longer read it**. While Node still has to read
new hashes, configure `PasswordHasherOptions::bcrypt(10)`: it writes `$2b$` hashes that `bcryptjs.compareSync` accepts, and
`needs_rehash` stops asking for a change. Set `dummy_algorithm` to the algorithm and cost of the hashes that are actually
stored, so an unknown user takes as long as a known one. See [Encryption and hashing](/security/encryption-and-hashing).

**Tokens.** Corvis verifies what `jsonwebtoken` signed when it has the same secret and algorithm (`HS256` by default in the
example) and an `exp` claim, and when the issuer and audience lists you configure match what Node puts in the token. A secret
under 32 bytes is refused. Corvis can also sign tokens that Node verifies, so access tokens can come from either side.
Tokens without `exp` need `.allow_missing_exp()` for as long as they are in circulation, and the startup warning will say so.

**API keys.** The default header is the same (`X-Api-Key`); the strategy name changed to `"api-key"`. If your guards in Node
refer to `headerapikey`, keep that name with `NAME`. Corvis compares digests, so store a SHA-256 of each key (`ApiKey::sha256_hex`)
instead of the key.

**OAuth2.** The client id and secret stay the same, but the redirect URI is the URL of the Corvis callback route, so register
it with the provider (the provider can hold both) before a route that logs in moves. The `state` of a login lives in a cookie of
the Corvis side or in its session, never shared with Node: a login started on one side finishes on that side.

## Checklist

- Every secret is at least 32 bytes and comes from the environment, not from the repository.
- Tokens carry `exp`, and the issuer and audience lists match.
- The session cookie name, secrets, store and the shape of `passport.user` are shared while both stacks run.
- Password hashes are verified with `verify_or_dummy` and upgraded with `needs_rehash`, or the bcrypt option keeps Node reading.
- API keys are stored as digests, and the strategy name is the one your clients expect.
- Redirect URIs are registered for the new host.
- Every login route has the throttler **before** the `AuthGuard` ([Rate limiting](/security/rate-limiting#protecting-a-login-route)).
- Cookies have `Secure`, `HttpOnly`, `SameSite=Lax` and, once the migration is over, the `__Host-` prefix.
- Routes that Node protected globally are protected here too: a global `AuthGuard` with `.skip_public()`, or
  [route posture](/security/route-posture) enforced.
- You have read [Securing authentication](/security/authentication-security).
