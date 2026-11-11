---
status: documented
---

# Securing authentication

The authentication module does a lot for you, and some things it cannot do. This page is for the person who deploys an
application that uses it: what the framework already does, what you must do before you go live, the mistakes the pieces allow
when combined, and what is still open. It does not repeat the [threat model](/security/authentication-threat-model), which
is written for someone judging the module and has a table per surface; every item here that relies on a mitigation links to
the row that explains it. The mechanics of each strategy are in [Passport](/recipes/passport), and the guide is
[Authentication](/security/authentication). The code samples come from `examples/auth-app`.

## What the framework does for you

- Credentials are compared in constant time, and secrets, tokens and hashes never reach a log, a response or a `Debug` output
  ([API keys](/security/authentication-threat-model#api-keys), [JWT](/security/authentication-threat-model#jwt)).
- A token is accepted only with an algorithm of the key's family and an expiry, `none` cannot be written, and a key that is too
  short stops startup ([JWT](/security/authentication-threat-model#jwt)).
- Passwords are hashed with a memory-hard algorithm, an unknown user costs the same as a known one, and every hash is bounded
  before it is verified ([passwords and local login](/security/authentication-threat-model#passwords-and-local-login)).
- A session identifier changes at login and logout, the cookie is signed and `HttpOnly`, and a cookie that arrives twice is no
  credential ([sessions](/security/authentication-threat-model#signed-cookies-and-sessions)).
- OAuth2 always sends `state` and PKCE, never builds a redirect URI from the request, and binds a flow to the session and the
  user that started it ([OAuth2](/security/authentication-threat-model#oauth2)).
- Every failure of a strategy answers the same `401`, a panic or an error ends the chain instead of letting the request
  through, and an empty user never authenticates ([strategy core](/security/authentication-threat-model#strategy-core)).
- A strategy name that nobody registered stops startup, and a route that runs the throttler after an `AuthGuard` is warned
  about ([guard order](/security/authentication-threat-model#guard-order-and-public-routes)).
- Every callback you write (a validator, a key provider) has a deadline ([strategy core](/security/authentication-threat-model#strategy-core)).
- The values of the credential query parameters a strategy declares are hidden in the request log
  ([defaults](/security/authentication-threat-model#defaults)).
- Login routes can be limited per client and per account before any password is hashed
  ([login rate limiting](/security/authentication-threat-model#login-rate-limiting)).

## Checklist before going live

**HTTPS and cookie flags.** Serve the application over HTTPS (or behind a proxy that ends TLS) and leave `Secure` on. Give the
session cookie, the OAuth2 state cookie and a cookie that holds a JWT the `__Host-` prefix: a browser then refuses to store one
set by a sibling subdomain (cookie tossing), which would otherwise let that subdomain log victims out or in as someone
else. `SessionOptions::name("__Host-sid")` does it for the session, and the OAuth2 state cookie has it by default. Only plain-HTTP
development turns it off, and the framework logs a warning when it is off.

**Secrets.** Each secret is at least the minimum length ([defaults](/security/authentication-threat-model#defaults)), comes from
the environment or a secret store, is different for each purpose (token key, session secret, state cookie secret, service key),
and is never committed. Generate them with `openssl rand -hex 32`. To rotate a signed-cookie or session secret, put the new one
first and keep the old one until the longest cookie lifetime has passed ([Session](/http/session#secrets-and-rotation)); a token
key rotates by issuing with the new key and verifying with a key provider that knows both until the old tokens expire. Never
sign your own cookies with the session secrets.

**Token lifetime.** A token is valid until it expires, and there is no revocation: a stolen token works until then. Make
access tokens short-lived and refresh them by logging in again or with a refresh endpoint of your own. Use a different key or
audience for each kind of token (access, refresh, password reset): the strategy does not check a token's type, so one kind
would be accepted as another.

**Login rate limiting.** Limit login routes per client address and per account, and register the throttler **before** any
`AuthGuard`, so a request over the limit costs no hash. Limit both routes of an OAuth2 login. A per-account limit means anyone
can lock an account out for its duration: that is a trade you accept or replace with your own policy. See
[Protecting a login route](/security/rate-limiting#protecting-a-login-route) and
[the threat model](/security/authentication-threat-model#login-rate-limiting).

**Credentials in URLs and logs.** Send credentials in a header or a body. A key, a token or a password in the query string is
opt-in in every strategy, ends up in the access logs of every proxy between the client and you, in browser history and in
`Referer`, and is hidden from the request log of Corvis only for the parameters a strategy declares: a parameter you read in
your own code is printed unless you register it (`Request::redact_query_param`). Use `Request::redacted_url()` when you log a URL.

**CSRF when a credential travels in a cookie.** A cookie is sent by the browser on its own. Refuse state-changing requests that
come from another site, including **login** (`SameSite=Lax` does not stop it): the example puts an origin check, the
`SameOriginGuard` of `src/same_origin.rs`, before the credentials are checked. See [CSRF protection](/security/csrf).

**A real session store.** The memory store belongs to one process: sessions are lost on restart and not shared between
instances. For more than one instance, write a `SessionStore` over a shared database (Redis and ORM stores are
[not provided yet](/security/authentication#what-is-not-included)).

**Redirect targets.** A redirect that is built from request data is an open redirect. The OAuth2 `return_to` accepts only a
relative path, and the redirect URI is configured, never built from the request; any redirect you write yourself needs the same
care ([Writing a strategy](/recipes/passport#writing-a-strategy)).

**Accounts from providers.** Identify a provider's user by `(provider, id)`. Link an existing account only through
`verified_email()`. Read no claim from the `id_token`: it is not verified.

**API keys.** Generate long random keys, show each once, store a digest, compare through the helpers, and give each client its
own key so you can revoke one.

**Passwords.** Hash with `PasswordHasher`; verify with `verify_or_dummy`; replace old hashes with `needs_rehash` after a
successful login; while most stored hashes are still bcrypt, set `dummy_algorithm` to the stored algorithm. The blocking
methods are for code outside the Tokio runtime. Enforce your own password policy: the framework has none, and checks no list of
breached passwords. See [Encryption and hashing](/security/encryption-and-hashing).

**Client addresses.** Set `trust proxy` to the proxies you control, or every client behind your load balancer shares one rate
limit ([the threat model](/security/authentication-threat-model#scope-and-assumptions)).

**Callbacks that wait.** A validator that blocks its thread instead of awaiting cannot be cancelled by its deadline. Await
database and network calls; never block. A validator that is cancelled stops at an `await`, so it must not leave state
half written: make the writes that must go together one call (the example replaces a password hash in one step).

**Caching.** Do not register `CacheInterceptor` for the whole application when it has authenticated `GET` routes
(see [Known limitations](#known-limitations)).

## Per strategy: your part

| Strategy | The framework's job | Your job |
|---|---|---|
| JWT | verify signature, algorithm family and expiry; refuse a short key and an oversized token; bound the key provider and the validator | choose the lifetime; keep tokens out of URLs; separate keys per kind of token; no revocation, so keep them short; apply the same rules at a WebSocket handshake |
| Local | read and bound the credentials; one verification per attempt; the uniform `401` | store hashes; return `None` for no user; rehash old hashes; throttle per client and per account; a password policy; lock accounts if you want that |
| API key | read from the configured places only; constant-time comparison; refuse conflicting or oversized keys | generate long keys; store digests; one key per client; rotate and revoke; keep the key out of URLs |
| Session | sign the cookie; change the identifier at login and logout; bound the store | HTTPS; `__Host-` prefix; an origin check on cookie-borne requests, login included; a store that fits your deployment; end sessions you must end |
| OAuth2 | `state`, PKCE, the flow bound to its session and user, the provider client with TLS verification always on | register the redirect URI; identify by `(provider, id)`; link only by verified e-mail; throttle both routes; store provider tokens encrypted if you keep them |
| Custom strategy | the uniform failure, panics caught, empty users refused | fail, never pass; bound what you read; constant-time comparison; no secrets in errors or panic messages |

## Common mistakes

**1. An optional guard as the only guard of a route that needs a user.** An optional guard lets the request through when
every strategy failed, so an anonymous request runs the handler.

```rust
// Anyone can call this: the guard is optional, and nothing else checks who the caller is.
#[post("close")]
#[use_guards(AuthGuard::new("jwt").optional())]
fn close_account(&self, req: Request) -> HttpResult<()> { /* ... */ }
```

Use a required guard, or ask for `User<T>`, which answers `401` when there is none. An optional guard is for routes that
anyone can call and that personalize their answer, and those are marked `#[public]`:

```rust
// examples/auth-app/src/controllers.rs
    #[get("me")]
    #[use_guards(AuthGuard::new("jwt"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }
```

**2. A guard that reads the user with no `AuthGuard` before it.** The session middleware restores the user from the cookie on
every route, so a roles guard on a route without an `AuthGuard` is authenticated by the cookie alone, and a cross-site form `POST`
carries the cookie ([the session user](/security/authentication-threat-model#the-session-user-on-routes-without-an-authguard)).

```rust
// Authenticated by whatever the session middleware restored, on a route with no AuthGuard.
#[use_guards(RolesGuard)]
```

List an `AuthGuard` that names the credentials the route accepts first:

```rust
// examples/auth-app/src/controllers.rs
    #[get("admin/users")]
    #[set_metadata(ROLES_KEY, [Role::Admin])]
    #[use_guards(AuthGuard::chain(["jwt", "session"]), RolesGuard)]
    fn admin_users(&self) -> Vec<Account> {
        self.users.accounts()
    }
```

**3. A global guard without `.skip_public()`, with routes marked `#[public]`.** Public routes are authenticated like any other
unless the guard says it honors the mark, so the login route answers `401` to everyone.

```rust
app.use_global_guards(AuthGuard::new("jwt")); // `#[public]` is ignored: the login route is guarded too
```

```rust
app.use_global_guards(AuthGuard::new("jwt").skip_public());
```

A global guard that skips public routes trusts every `#[public]` mark, so review them, and turn on
[route posture](/security/route-posture) so a forgotten guard stops startup.

**4. The throttler after the authentication.** A rate limit that runs after the `AuthGuard` limits nothing: the password has
already been hashed. Startup warns when it sees this order; register the throttler as a global guard through the container so
it runs first:

```rust
#[use_guards(AuthGuard::new("local"), ThrottlerGuard)] // the password is hashed before the limit is checked
```

```rust
// examples/auth-app/src/app_module.rs
    providers = [
        // A global guard registered through the container runs before every `AuthGuard`.
        Provider::app_guard::<ThrottlerGuard>(),
        // ...
    ],
```

**5. A wrapper guard that hides the guard inside it.** A guard that wraps an `AuthGuard` must list it in `inner_guards`,
or startup cannot check its strategy names or its order with the throttler. Do not return the inner guard from `as_any`: that is
for the guard itself.

**6. `Pass` from a custom strategy.** `StrategyOutcome::Pass` lets the request through with no user. Return `Fail` when the
credential is missing or wrong, and `Error` when a dependency failed.

**7. Echoing `auth_info()` to the client.** It holds the reason of a failure, and the framework keeps it out of the response on
purpose: an answer that says *why* a login failed tells an attacker what to try next. Log it; do not return it.

**8. Comparing secrets with `==`.** It stops at the first different byte. Compare through the helpers, which hash both sides
and compare the digests in constant time, and refuse an empty expected secret before comparing:

```rust
if key == expected { /* ... */ } // timing leaks, and two empty strings are equal
```

```rust
// examples/auth-app/src/auth/api_key.rs
        Ok(self.keys.expected.matches(key).then_some(ServiceCaller { service: "reports" }))
```

**9. Linking accounts by e-mail.** An address is a claim by the provider. Matching it without checking that the provider
verified it lets anyone who registers someone else's address at a provider take over that account:

```rust
let user = users.find_by_email(&profile.emails[0].value); // unverified, and the id is ignored
```

```rust
// examples/auth-app/src/auth/provider.rs
        let name = profile.display_name.as_deref().unwrap_or("user");
        Ok(Some(self.users.for_provider_user(&profile.provider, &profile.id, profile.verified_email(), name)))
```

**10. A validator that returns a falsy user.** `Ok(Some(expected.matches(key)))` says "found" for every key, and the user is
`false` when the key is wrong. Such a user never authenticates, and a warning tells you which validator to fix; say "no user"
with `None`.

**11. `.log_in()` on the first step of a two-step login.** A login made by the guard is kept when the handler returns normally,
whatever status it sets. A handler that implements a second step authenticates without `.log_in()` and calls
`Authenticator::log_in` after the last step.

**12. A cookie reader placed before the header reader.** `ExtractJwt::from_extractors` tries the first non-empty token only, so
with a cookie extractor first, anyone who can plant a cookie overrides the user's `Authorization` header. Read only the places
you need, the most trusted first.

**13. Completing an account link under another user.** A link started by one user and completed by another on a shared browser
is refused by the framework when both routes run the same guards. Run the same guards on the route that starts the flow and on
the callback, and link the provider account to `Request::user()` only after they authenticated the request.

## Known limitations

These findings of [the review](/security/authentication-threat-model#findings) are open. Until each is fixed, use the
workaround.

| Issue | What is open | What to do until it is fixed |
|---|---|---|
| #173 | strategies do not run at the WebSocket handshake, and `AuthGuard` denies socket messages | authenticate in `handle_connection` and apply the JWT strategy's rules there; see [WebSockets](/security/authentication#websockets) |
| #174 | `JwtService` accepts a short secret and a token without `exp`, which the JWT strategy refuses | authenticate requests with the strategy; if you verify a token with `JwtService`, check the expiry and the secret length yourself |
| #172 | no allow-list for the OAuth2 redirect URI | register one exact redirect URI per environment with the provider, and run one strategy per domain |
| #170, #171 | no Redis or ORM session store | the memory store is per process: write a `SessionStore` over a shared database |
| #177 | `CacheInterceptor` keys by URL, so a global registration replays one user's `GET` response to another | do not register it for the whole application when it has authenticated `GET` routes; cache on routes that are the same for everyone |
| #178 | in Socket.IO a connect middleware on the main namespace does not cover the other namespaces, and `Debug` of a `Handshake` prints the `auth` payload | register the middleware on every namespace that has handlers; never format a handshake with `{:?}` |

One more is a limit rather than a bug: a session store that is down fails routes that carry a session cookie, public ones and
health checks included, because the session middleware runs before every guard. Keep the store as reliable as the
application, and put a limiter ahead of `SessionModule` if requests with someone else's cookie can reach it.
