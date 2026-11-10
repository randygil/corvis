---
status: documented
---

# Authentication threat model

This page states what the authentication module protects, from whom, by which mechanism and which test proves it, and what
it leaves to the application. It covers `corvis::auth` (the strategy core, the JWT, local, API key, session and OAuth2
strategies), the signed cookies they use, and the throttler in front of login routes. Read it when you decide whether to
rely on the module for a given route, and before you write an authorization guard, a login route or a WebSocket handshake.

Each strategy was reviewed on its own when it was written; the findings of those reviews are linked from the
[register](#findings), not repeated. This page adds what only exists **between** the modules: a session cookie next to a
token, chains, guard order, shared credential places and sockets. Every row names the test file or the specification
scenario that fails when the mitigation is removed. The user-facing guidance is in [Authentication](/security/authentication),
[Passport recipe](/recipes/passport), [Session](/http/session) and [Rate limiting](/security/rate-limiting).

## Scope and assumptions

In scope: the code in `crates/corvis/src/auth/`, `crates/corvis/src/jwt.rs`, `crates/corvis/src/http/signed_cookie.rs`, the
guard loop of the router, the `Request` user slots and the throttler hooks the module uses.

Assumptions the module relies on:

- **TLS ends at your server or at a proxy you trust.** Cookies are `Secure` by default; over plain HTTP in development you
  turn that off and the module logs a warning.
- **The operating system random generator works.** Identifiers, state values, PKCE verifiers and password salts come from
  it, and a failure is an error, never a weaker fallback.
- **Secrets are secret.** The module refuses short ones, never prints them, and cannot protect an application whose
  `JWT_SECRET` or session secret is in the repository.
- **`trust proxy` is set correctly.** The throttler and `Request::ip()` believe `X-Forwarded-For` only from the proxies you
  list.
- **The user store is the application's.** The module authenticates a request to a user value your validator returns; it
  does not decide who may exist, what a user may do, or when an account is locked.

Out of scope, with the reason: multi-factor authentication, CAPTCHA, breached-password checks and account lockout (application
policy; the [rate limiting page](/security/rate-limiting#protecting-a-login-route) shows what the throttler can do),
CSRF tokens (see [CSRF protection](/security/csrf)), an external penetration test or an audit of the cryptographic crates'
internals, and a coverage-guided fuzzer (the repository fuzzes with property tests, see [Findings](#findings)).

## Assets

| Asset | Where it lives | What an attacker gains |
|---|---|---|
| Passwords | request bodies, your user store (as hashes) | account takeover on this and other sites |
| Signing secrets (JWT HMAC secret, `CookieKeys`, OAuth2 state keys) | your configuration | forge tokens, cookies or states for any user |
| Session identifiers | the signed session cookie, the session store | take over one session |
| Tokens (JWT, API keys, OAuth2 access and refresh tokens) | request headers, your stores | act as the user or service until they expire |
| OAuth2 client secret | your configuration | impersonate your application to the provider |
| The user identity of a request (`Request::user()`, `User<T>`) | the request | every authorization decision downstream |
| Logs | your log sink | every credential that was printed |

## Trust boundaries

| Boundary | Trusted for | Not trusted for |
|---|---|---|
| Browser | holding its own cookies | anything it sends: every header, cookie, query parameter and body is attacker-controlled |
| Reverse proxy | the client address (when `trust proxy` lists it), TLS termination | nothing else; it can add headers but should strip the ones you read credentials from |
| Application and its configuration | everything | a developer mistake is the main risk the module's defaults and startup checks target |
| Session store | holding records the application wrote | availability (an outage is an error on every request that carries a session cookie, public routes included), confidentiality of the records |
| OAuth2 provider | the tokens and profile it returns after a valid code | the text of its errors: nothing it says reaches a response, a log or an error |
| Logs | an operator reading them | being free of credentials: the module redacts what it knows are credentials |

**Is a compromised session store in scope?** Partly. An attacker who can **read** the store gets the identifiers of the live
sessions, but not the signing key, so with the store alone they cannot build the signed cookie a browser sends: the cookie is
`s:<id>.<signature>` and the module verifies the signature before any store access. The records also hold the user's
serialized value (usually an id) and whatever your application put in the session, so keep secrets out of it. An attacker
who can **write** the store can create a session for any user: the store is as trusted as the database of users. Use a
store with the same access controls and the Redis or database store of your choice; the memory store is per process.

## Attackers

- **Anonymous network attacker**: sends any request, any number of them, with any credentials: guessing, stuffing, replay,
  malformed input, oversized input, cross-site requests through a victim's browser.
- **Authenticated user**: tries to act as another user, to keep a session after logout, to reach routes meant for another
  role.
- **Malicious web page**: makes the victim's browser send requests with its cookies (CSRF, login CSRF, cookie tossing from
  a sibling subdomain).
- **Malicious or compromised OAuth2 provider endpoint**: answers with oversized, slow or hostile documents.
- **Attacker with read access to logs, error reports or the session store**: looks for credentials.

Not considered: an attacker who controls the server process, the operating system generator or the build.

## Threats by surface

Each table lists the threat, what the module does about it, the test or specification scenario that proves it, and what is
left. "Spec" means the scenario of the OpenSpec capability named, kept in `openspec/specs/` once the change is archived.

::: warning Do not register `CacheInterceptor` for the whole application when it has authenticated `GET` routes
`CacheInterceptor` keys a `GET` response by its URL alone and replays it to whoever passes the route's guards. Registered
globally (`Provider::app_interceptor::<CacheInterceptor>()`, as the [caching page](/data/caching) shows for a plain
application), it serves the first user's response to every other authenticated user, and to anonymous clients on routes
whose guard is optional or that have no guard (the session user is restored on those). The authentication modules cannot fix
this: the cache is a separate module and its keys have no per-request part. It is filed as #177. Until it is fixed, apply
`#[use_interceptors(CacheInterceptor)]` only to routes whose body is the same for everyone.
:::

### Strategy core

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| A strategy that fails open lets a request through | `Fail` goes to the next strategy, `Error` ends the chain, a panic is an error; no built-in strategy returns `Pass` | `crates/corvis/tests/auth_guard.rs`, `crates/corvis/tests/auth_hardening.rs`, `crates/corvis/tests/auth_invariants.rs` (`built_in_strategies_never_pass`) | a custom strategy that returns `Pass` or succeeds with a user it did not check |
| A user that serializes to `null`, `false`, `0` or `""` authenticates | such a `Success` is a failure, for every strategy | `crates/corvis/tests/auth_empty_user.rs` | none |
| A reason for the failure reaches the client | the answer is the plain `401 {"message":"Unauthorized","statusCode":401}`; the reason is in `Request::auth_info()` only | `crates/corvis/tests/auth_invariants.rs` (`reasons_are_indistinguishable`, `chain_position_is_not_revealed`) | an `on_failure` closure that echoes the failure is the application's choice |
| A guard response is used outside the guard phase | `respond_with` works only inside `can_activate` | `crates/corvis/tests/auth_hardening.rs` | none |
| A strategy name is misspelled or hidden behind a wrapper guard | names are checked at startup, also through `CanActivate::inner_guards` | `crates/corvis/tests/auth_guard.rs` | a wrapper that exposes neither `as_any` nor `inner_guards` is not checked until its first request |
| An application callback never answers and holds the request | the JWT key provider (which runs for any well-formed token, forged ones included), the JWT validator, the local validator and the API key validator each have a deadline, 5 s by default (`key_provider_timeout`, `validator_timeout`); a callback that misses it is dropped (its future is cancelled) and every caller gets the same `503`, with one warning a minute that names the strategy | `crates/corvis/tests/auth_callback_timeouts.rs`, `crates/corvis/tests/auth_api_key_options.rs` | a callback that blocks its thread instead of awaiting cannot be cancelled; the session store and the OAuth2 provider have their own deadlines |
| Credentials in a query string reach the logs | strategies declare their credential parameters; the request log, `Debug` of a request and the router's 404 message hide them | `crates/corvis/tests/auth_api_key_log.rs`, `crates/corvis/tests/auth_oauth2_log.rs`, `crates/corvis/tests/fuzz_auth.rs` (`canary_secrets`) | parameters your own code reads are not known to the framework |

### JWT

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| Algorithm confusion, `none`, a key of another family | an allow-list of algorithms per key family; `crit` and malformed headers are refused before the key provider | `crates/corvis/tests/auth_jwt.rs`, `crates/corvis/tests/auth_jwt_header.rs` | none known |
| Weak HMAC secret | a secret under 32 bytes stops the application at startup | `crates/corvis/tests/auth_jwt_options.rs`, `crates/corvis/tests/auth_defaults.rs` (`secret_minimum`) | a long secret that is guessable |
| Token without expiry, expired or not yet valid | `exp` is required, `nbf` honored, `clock_tolerance` is zero by default and capped at 300 s | `crates/corvis/tests/auth_jwt.rs` | a token stays valid until `exp`: there is no revocation |
| Oversized or deeply nested unverified input | `max_token_length` 8192; the unverified header and payload are parsed without `serde_json::Value` | `crates/corvis/tests/auth_jwt_header.rs` | none |
| Token in a cookie is an ambient credential, and a cookie planted from a sibling subdomain | extractors are explicit; a cookie name that appears more than once in the request yields no token (the uniform `401`), as the session cookie does; a startup warning names a cookie extractor whose cookie has no `__Host-` prefix | `crates/corvis/tests/auth_invariants.rs` (`a_jwt_cookie_sent_twice_is_no_token`), `crates/corvis/tests/auth_jwt_log.rs` | a planted cookie that is the only one still authenticates as its owner: name the cookie `__Host-...`. `SameSite` and CSRF protection are the application's, and a cookie token issued without `SameSite` authenticates cross-site `POST`s in browsers that do not default to `Lax` |
| Credential in the URL | the query extractor is opt-in and logs a startup warning | `crates/corvis/tests/auth_jwt_log.rs` | the URL is still in proxies' logs |
| `JwtService` (the older module) is looser than the strategy | `verify` applies `iss` and `aud` strictly when configured and refuses `crit`; secrets under 32 bytes and a missing `exp` are still accepted there | `crates/corvis/tests/auth_jwt.rs` | tracked in #174; use the JWT strategy for authentication |
| The key provider is down | a provider that fails is a failed attempt (`401`, and a chain goes on to the next strategy), as `secretOrKeyProvider` does in the original; the next strategy still needs its own valid credential | `crates/corvis/tests/auth_invariants.rs` (`failing_dependency_before_a_weaker_strategy`) | other dependency outages (API key validator, hasher, session store, OAuth2 provider) are `5xx` errors that end the chain: the difference is deliberate parity, not a way in |

### Passwords and local login

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| Offline cracking of stolen hashes | argon2id (19456 KiB, 2 iterations, 1 lane) for new hashes, bcrypt and argon2 verification | `crates/corvis/tests/auth_password.rs` | weak passwords |
| A planted hash makes verification expensive | verification limits (argon2 memory, iterations, lanes, bcrypt cost, hash length) checked before any derivation | `crates/corvis/tests/auth_password.rs` | none |
| User enumeration by timing or response | `verify_or_dummy` verifies against a dummy hash for an unknown user; one `401` for every reason | `crates/corvis/tests/auth_local.rs`, `crates/corvis/tests/auth_invariants.rs` (`reasons_are_indistinguishable`) | a validator that does extra work for existing users |
| Resource exhaustion through hashing | bounded concurrency and waiting; credentials longer than 1024 bytes never reach the validator | `crates/corvis/tests/auth_password.rs`, `crates/corvis/tests/auth_local.rs` | an attacker can still occupy the slots: rate limit the route |
| Credentials in a URL | off by default, a startup warning when enabled | `crates/corvis/tests/auth_local.rs` | none |
| Guessing passwords | see [Login rate limiting](#login-rate-limiting) | `crates/corvis/tests/auth_login_throttle.rs` | the limit is the application's to size |

### API keys

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| Timing of the comparison | `subtle` constant-time comparison of digests; `ExpectedApiKey` needs at least 32 bytes | `crates/corvis/tests/auth_api_key.rs`, `crates/corvis/tests/auth_defaults.rs` (`secret_minimum`) | the validator's own lookup |
| Two sources with different keys, or an unusable value, let another source decide | conflicting or invalid values fail the attempt | `crates/corvis/tests/auth_api_key.rs` | none |
| The key reaches logs through the URL or `Debug` | query sources are opt-in and redacted everywhere; `Debug` prints no value | `crates/corvis/tests/auth_api_key_log.rs`, `crates/corvis/tests/auth_invariants.rs` (`debug_of_every_public_type`) | none |
| An unauthenticated request costs a lookup | the validator runs for any well-formed key and is bounded by `validator_timeout` | `crates/corvis/tests/auth_invariants.rs` (`unauthenticated_cost_of_each_route_shape`) | the application's lookup is reachable without credentials: rate limit it |

### Signed cookies and sessions

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| Forged or tampered cookie | HMAC-SHA256 with keys of at least 32 bytes, constant-time verification, up to 8 keys for rotation | `crates/corvis/tests/signed_cookies.rs`, `crates/corvis/tests/auth_session.rs` | the cookie name is not part of the signature: use separate keys per cookie |
| Session fixation | a new identifier at login and logout; the old record is destroyed | `crates/corvis/tests/auth_session.rs`, `crates/corvis/tests/auth_invariants.rs` (`callback_that_logs_in`) | none |
| A failed request creates or renews a session | a failed login writes nothing and sets no cookie while `save_uninitialized` is off (the default); deleting cookies and `rolling` renewal are allowed | `crates/corvis/tests/auth_invariants.rs` (`failed_login_with_an_anonymous_session`) | with `save_uninitialized(true)` a failed login gets a new anonymous session and cookie, which carry no authority |
| A login that a later guard rejects stays logged in | a guard login is kept only when the request is not rejected | `crates/corvis/tests/auth_session_review.rs`, `crates/corvis/tests/auth_session_review2.rs` | a handler that logs in itself decides |
| A rejected request leaves cookies a strategy appended | the rejection mark discards the session login only; a cookie a strategy appends with `append_response_header` is sent on every later `4xx` and `5xx`, as in Express | not mitigated: by design | a custom strategy must not append a cookie that carries authority; log in through the session |
| Session stolen from the store or the cookie | `HttpOnly`, `Secure`, `SameSite=Lax`; idle 24 h and absolute 7 day timeouts | `crates/corvis/tests/auth_defaults.rs` (`cookies_set_by_the_module`) | the cookie is an ambient credential: CSRF protection is yours |
| Store outage or a hostile record | every call is bounded by `store_timeout`; records are size and depth limited; a failed load is a `500` | `crates/corvis/tests/auth_session_http.rs`, `crates/corvis/tests/auth_session_contention.rs` | with the store down, every request that carries a session cookie fails, public routes and health checks included |
| Memory store flood | `max_sessions`, per-user limits and a total byte budget | `crates/corvis/tests/auth_session_memory_heap.rs` | an anonymous flood can evict anonymous sessions; use a shared store in production |
| Login CSRF | not stopped by `SameSite=Lax`: a cross-site urlencoded form post to a login route logs the victim in as the attacker. `SameOriginGuard` exists only in `examples/cats-app`, not in the framework, and is a candidate to move into `corvis::auth` | `docs/security/csrf.md` | the application adds a same-origin check or a token to login routes |

### OAuth2

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| Login CSRF and a forged callback | `state` bound to the browser (signed `__Host-` cookie or the session), checked before any provider request, used once | `crates/corvis/tests/auth_oauth2.rs`, `crates/corvis/tests/auth_invariants.rs` (`failed_oauth2_callback`, `forged_callback_on_an_authenticated_session`) | none |
| A pending flow completes under another user (a shared browser: A starts an account link, logs out, B logs in, A's callback arrives) | with the state in a cookie, the flow records a keyed digest of the session identifier (or that there was none) and the callback is refused with the uniform failure when its session differs: another session, a session where there was none, none where there was one; with `state_in_session()` the flow dies with the session. The digest is a SHA-256 of an HMAC made with a state cookie key, so the identifier cannot be recovered from the cookie | `crates/corvis/tests/auth_invariants.rs` (`oauth2_cookie_state_is_bound_to_the_session_it_started_under`) | the binding is skipped when no `SessionModule` runs; a session created during the start request itself (anything the handler writes to it) makes a flow that began without one fail; a key rotation between the redirect and the callback fails that login once. Account linking should use `state_in_session()` or rely on the binding, and compare provider accounts with `Request::user()` only after it |
| Code interception | PKCE `S256` is always on | `crates/corvis/tests/auth_oauth2.rs` | none |
| Replay of a callback | a redeemed state is refused, also while the first request is in flight and after logout | `crates/corvis/tests/auth_invariants.rs` (`callback_in_flight_twice_reaches_the_provider_once`, `callback_replayed_after_logout`) | none |
| Open redirect through `return_to` or `failure_redirect` | only a relative path under a fixed rule, or a configured absolute `https` URL | `crates/corvis/tests/auth_oauth2.rs`, `crates/corvis/tests/fuzz_auth.rs` (`redirect_targets`) | the redirect URI allow-list is tracked in #172 |
| A hostile provider answers with huge, slow or redirecting documents | no redirects, no proxy variables, deadlines and size limits, TLS verification | `crates/corvis/tests/auth_oauth2.rs`, `crates/corvis/tests/auth_oauth2_review.rs` | none |
| Tokens in logs, errors or `Debug` | `Debug` of grants and tokens prints names only; provider text never reaches a response | `crates/corvis/tests/auth_oauth2_log.rs`, `crates/corvis/tests/auth_invariants.rs` (`debug_of_every_public_type`) | the validator receives the tokens: store them encrypted |
| Starting logins costs the server | a login start with the state in the session writes a record; the in-flight request count is bounded | `crates/corvis/tests/auth_login_throttle.rs` (`oauth2_login_start_over_the_limit`) | rate limit the login route |
| A cross-site page cancels a victim's pending login | none: a browser keeps four flows per strategy, and four forced starts (top-level `GET`s to the start route) push out the victim's | `crates/corvis/tests/auth_oauth2.rs` | a nuisance, the victim retries |
| State and session regeneration disagree | the state record is removed before the provider is contacted, the identifier changes at login, no `oauth2:` key survives | `crates/corvis/tests/auth_invariants.rs` (`callback_that_logs_in`) | none |

### Combinations of strategies

These are the interactions no single strategy sees.

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| A session cookie gives an identity the route's guard did not establish | after an `AuthGuard` has run, the user is the one that guard set, or none: an optional token guard clears the session user, a token guard replaces it, a token-only route answers `401` to a cookie alone | `crates/corvis/tests/auth_invariants.rs` (`session_of_one_user_and_token_of_another`, `optional_token_guard_with_a_session_cookie`, `token_only_route_with_only_a_session_cookie`, `session_strategy_in_the_chain`) | none |
| The session user on routes without an `AuthGuard` | see [the session user](#the-session-user-on-routes-without-an-authguard) | `crates/corvis/tests/auth_invariants.rs` (`roles_guard_without_an_auth_guard`) | a hand-written guard that trusts `Request::user()` admits a session cookie on its own |
| Fall-through to a weaker strategy after a dependency failure | an `Error` ends the chain, so a provider or store outage never falls through | `crates/corvis/tests/auth_invariants.rs` (`failing_dependency_before_a_weaker_strategy`) | none |
| An OAuth2 strategy in a chain | it starts a login for any request that is not a callback, so it ends the chain with a redirect | `crates/corvis/tests/auth_invariants.rs` (`an_oauth2_strategy_in_a_chain_redirects_instead_of_falling_through`) | give OAuth2 its two routes and keep it out of chains; in the last place it turns every anonymous request into a login start |
| Two strategies read the same place differently | the JWT bearer reader and the API key `bearer()` source parse `Authorization` the same way; headers repeat as Node joins them | `crates/corvis/tests/auth_invariants.rs` (`bearer_value_shared_by_two_strategies`), `crates/corvis/tests/fuzz_auth.rs` (`gate_matches_model`) | the API key reader also gives up on a header with more than 640 bytes of leading whitespace, which only makes it stricter |
| A credential sent in the wrong place authenticates | a credential authenticates only through the strategy configured for its place | `crates/corvis/tests/auth_invariants.rs` (`token_in_a_place_that_is_not_configured`) | none |
| A session record left by OAuth2 state authenticates | the state never becomes a user; the old session is destroyed at login | `crates/corvis/tests/auth_invariants.rs` (`callback_that_logs_in`) | none |
| Cookies set on a failed request | a failed request gets no session or state cookie that carries authority | `crates/corvis/tests/auth_invariants.rs` (`failed_login_with_an_anonymous_session`, `failed_oauth2_callback`) | the state cookie is sent back as it was on a forged callback, so the browser keeps its genuine flow |
| A random request reaches a handler it should not | a model of the whole grid of guard shapes against random credentials | `crates/corvis/tests/fuzz_auth.rs` (`gate_matches_model`) | the model and the code could be wrong in the same way: the scenarios above are hand-written |
| A secret appears in output | markers in every credential place on every route, with the application's real secrets | `crates/corvis/tests/fuzz_auth.rs` (`canary_secrets`) | credentials your own code logs |

### Guard order and public routes

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| A throttler that runs after an `AuthGuard` limits nothing | a throttled request runs no strategy, no key derivation, no store write and no provider request when the throttler is first; startup logs a warning for a route whose guards, flattened in order through `inner_guards`, run an `AuthGuard` before a `ThrottlerGuard` | `crates/corvis/tests/auth_login_throttle.rs`, `crates/corvis/tests/auth_guard_order_log.rs` | a wrapper guard that exposes neither guard through `as_any` or `inner_guards` hides the warning |
| `#[public]` opens a route that needed authentication | `#[public]` is honored only by a guard built with `.skip_public()`; a guard built without it authenticates public routes too | `crates/corvis/tests/auth_invariants.rs` (`invalid_credentials_on_a_public_route`, `controller_marked_public_handler_not_marked`) | a global `.skip_public()` guard trusts every `#[public]` mark |
| A route with only an optional guard counts as protected | an optional guard does not count as access control for route posture | `crates/corvis/tests/auth_invariants.rs` (`posture_with_only_an_optional_guard`), `crates/corvis/tests/route_posture.rs` | a hand-written guard counts unless it overrides `controls_access` |
| Anonymous requests are expensive | a token that does not verify or a cookie that does not verify costs no validator call, no lookup, no store access and no provider request | `crates/corvis/tests/auth_invariants.rs` (`unauthenticated_cost_of_each_route_shape`) | the API key validator, the password derivation and a login start with the state in the session cost something by design; a request that carries a session cookie of the client's own costs two store calls (and a user lookup when it is logged in) even on a `404` or a `429`, because the session middleware runs before every guard: only a limiter ahead of `SessionModule` shields the store |
| A shared cache serves one user's response to another | not mitigated here: `CacheInterceptor` keys by URL, so a global registration replays the first user's `GET` response on authenticated routes (see the warning at the top of this section) | not mitigated: filed as #177 | do not register `CacheInterceptor` for the whole application when it has authenticated `GET` routes |

### WebSocket and Socket.IO handshakes

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| Strategies run for a socket message | `AuthGuard` denies every message without running a strategy, and the application warns once per gateway | `crates/corvis/tests/auth_ws.rs`, `crates/corvis/tests/auth_invariants.rs` (`global_auth_guard_and_a_gateway`) | none |
| A session cookie authenticates an upgrade | upgrade requests are answered before the router: no middleware, no session, no user on the handshake | `crates/corvis/tests/auth_invariants.rs` (`session_cookie_on_an_upgrade_request`) | authenticate in `handle_connection` |
| The handshake accepts a token the HTTP guard refuses | the documented pattern verifies the token with `JwtService`, whose rules are looser than the JWT strategy's | not mitigated: the pattern is in `docs/security/authentication.md`, the gap is #173 | tracked in #173 (strategies at the WebSocket handshake) and #174; until then apply the JWT strategy's rules in `handle_connection` |
| Cross-site WebSocket hijacking | `GatewayOptions::allowed_origins` | `crates/corvis/tests/ws_security.rs` | opt-in |
| Token in the handshake query reaches logs | the request line log is written by `LoggingInterceptor`, which runs for HTTP routes: a handshake is not logged by it | by construction (`crates/corvis/src/common/interceptor.rs`) | proxies and your own handshake code can still log the URL: prefer a header or the first message |
| Engine.IO handshake | Engine.IO requests are dispatched before the router, so no middleware, session or guard runs for them | by construction (`crates/corvis/src/server.rs`); `crates/corvis/tests/engine_io.rs` shows the handshake request the callback receives | authenticate in the connection handler |
| Socket.IO handshake | the layer was reviewed: a session cookie never reaches a handshake (no user and no session on it, even with a valid cookie), events for a pending or refused namespace close the connection, and the log lines carry no URL, `auth` payload or panic text | reviewed by hand (see Findings); `crates/corvis/tests/auth_invariants.rs` (`session_cookie_on_an_upgrade_request`) covers the plain upgrade | authenticate in the connect handler; strategies at the handshake are #173 |
| A connect middleware does not cover every Socket.IO namespace | none: `IoServer::use_middleware` registers on the main namespace only, as `io.use` does in Node, so a client that sends a CONNECT for `/admin` reaches its handlers without the middleware, and nothing warns | not mitigated: filed as #178 | register the middleware on every namespace that has handlers |
| `Debug` of a Socket.IO `Handshake` prints credentials | none: it prints the whole CONNECT `auth` payload and the handshake URL, query string included | not mitigated: filed as #178 | never format a handshake with `{:?}` |

### Login rate limiting

| Threat | Mitigation | Verified by | Residual risk |
|---|---|---|---|
| Password guessing from one client | the `default` throttler counts by client address, with the shorter `ttl` | `crates/corvis/tests/auth_login_throttle.rs` (`password_guessing_by_one_client_over_the_client_limit`) | addresses behind a shared NAT share a limit |
| Password guessing for one account from many addresses | `login_tracker` counts by account on the login route | `crates/corvis/tests/auth_login_throttle.rs` (`same_account_from_many_addresses`) | any client can lock an account out for the block duration |
| The username moved to another place dodges the counter | the tracker reads the username the local strategy will read, with the same options | `crates/corvis/tests/auth_login_throttle.rs` (`username_moved_to_the_query_string`, `nested_field_name`) | none |
| The counter reveals whether an account exists | counting does not depend on the account; the tracker value is a digest | `crates/corvis/tests/auth_login_throttle.rs` (`existing_and_unknown_accounts`, `storage_keys`) | none |
| Attempts without a username | counted by the client address | `crates/corvis/tests/auth_login_throttle.rs` (`missing_username`) | none |
| A throttled request still costs a derivation | the throttler runs first | `crates/corvis/tests/auth_login_throttle.rs` (`password_guessing_over_the_limit`) | none |

## The session user on routes without an AuthGuard

The session middleware restores the user on every route it covers, before any guard, as `passport.session()` sets
`req.user` in the original. The review decided to **keep that**, and to pin it with a test, for these reasons:

- **It adds no authority.** The restored user is the one the same cookie authenticates through `AuthGuard::new("session")`;
  nothing is read that a request with that cookie could not already use, and every `AuthGuard` the route has replaces or
  clears it, so the route's guard decides the identity (the first rows of the combinations table).
- **Parity is the point of the module.** Migrated applications read `req.user` on routes with no guard (optional personalization
  is the common case). Restoring only when the `"session"` strategy runs would break them silently.
- **The stricter rule does not close the real risk**, which is a hand-written authorization guard that trusts
  `Request::user()` and has no `AuthGuard` in front of it. With the stricter rule it would see no user and deny, which is
  safer, but it would make the identity depend on the order of guards. The rule to follow is the same either way: put an
  `AuthGuard` first, and a route-level `AuthGuard::new("session")` before any guard that reads the user.

The consequence, stated plainly: a guard that reads `Request::user()` on a route with no `AuthGuard` authenticates by the
session cookie alone, with nothing checked about the request (so a cross-site form post carries it). The test
`roles_guard_without_an_auth_guard` in `crates/corvis/tests/auth_invariants.rs` pins that behavior. Route posture treats such
a guard as access control unless it returns `false` from `controls_access`, so posture alone does not tell you the route is
safe. See [Session](/http/session#guards-and-the-session-user).

## Defaults

One rule per row, the same in every strategy. `crates/corvis/tests/auth_defaults.rs` reads the block under the table and
compares each line with the code, so a default that changes in one place only fails `cargo test`.

| Setting | Default | Set in | The original's default |
|---|---|---|---|
| Secret minimum (JWT HMAC, cookie keys, expected API key) | 32 bytes, startup error | `jwt`, `http::signed_cookie`, `api_key` | none |
| JWT token length | 8192 bytes | `JwtStrategyOptions::max_token_length` | none |
| JWT clock tolerance | 0 s (at most 300 s) | `JwtStrategyOptions::clock_tolerance` | 0 |
| JWT `exp` | required | `JwtStrategyOptions::allow_missing_exp` | optional |
| API key length | 512 bytes (at most 4096) | `ApiKeyStrategyOptions::max_key_bytes` | none |
| API key validator timeout | 5 s | `ApiKeyStrategyOptions::validator_timeout` | none |
| JWT key provider timeout | 5 s (1 ms to 300 s) | `JwtStrategyOptions::key_provider_timeout` | none |
| JWT validator timeout | 5 s (1 ms to 300 s) | `JwtStrategyOptions::validator_timeout` | none |
| Local validator timeout | 5 s (1 ms to 300 s) | `LocalStrategyOptions::validator_timeout` | none |
| API key source | the `X-Api-Key` header | `ApiKeyStrategyOptions` | the header |
| Username and password length | 1024 bytes each | `LocalStrategyOptions` | none |
| New password hashes | argon2id, 19456 KiB, 2 iterations, 1 lane; bcrypt cost at least 10 | `PasswordHasherOptions` | bcrypt, no minimum |
| Verification limits | bcrypt cost 14, argon2 65536 KiB, 8 iterations, 16 lanes, 512 bytes of hash | `VerifyLimits` | none |
| Wait for a hashing slot | 10 s | `PasswordHasherOptions` | none |
| Session cookie | `connect.sid`, `Path=/`, `HttpOnly`, `Secure`, `SameSite=Lax` | `SessionCookie` | no `Secure`, no `SameSite` |
| Session timeouts | idle 24 h, absolute 7 days | `SessionOptions` | none |
| Session record size and store timeout | 16384 bytes, 5 s | `SessionOptions` | none |
| Session renewal and data at login | `rolling` off; data dropped at login | `SessionOptions` | `rolling` off |
| OAuth2 state | lifetime 600 s, PKCE `S256` | `OAuth2Options` | none, optional |
| OAuth2 state cookie | `__Host-oauth2.<name>`: `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/` | `OAuth2Options` | none |
| OAuth2 limits | code 2048 bytes, connect 5 s, request 10 s, token document 65536 bytes, profile 262144 bytes, 32 requests in flight, 100000 redeemed states | `OAuth2Options`, `OAuth2HttpOptions` | none |
| Credentials in a URL | off for every strategy | each strategy | on for some |

```ini
jwt.min_secret_bytes = 32
jwt.max_token_length = 8192
jwt.clock_tolerance_seconds = 0
jwt.max_clock_tolerance_seconds = 300
jwt.require_exp = true
jwt.key_provider_timeout_seconds = 5
jwt.validator_timeout_seconds = 5
api_key.min_secret_bytes = 32
api_key.max_key_bytes = 512
api_key.validator_timeout_seconds = 5
api_key.default_source = X-Api-Key
cookie_keys.min_secret_bytes = 32
cookie_keys.max_secrets = 8
local.max_credential_bytes = 1024
local.validator_timeout_seconds = 5
local.argon2_memory_kib = 19456
local.argon2_iterations = 2
local.argon2_parallelism = 1
local.bcrypt_min_cost = 10
local.verify_max_bcrypt_cost = 14
local.verify_max_argon2_memory_kib = 65536
local.verify_max_argon2_iterations = 8
local.verify_max_argon2_parallelism = 16
local.verify_max_hash_bytes = 512
local.acquire_timeout_seconds = 10
session.cookie_name = connect.sid
session.cookie_path = /
session.cookie_http_only = true
session.cookie_secure = true
session.cookie_same_site = Lax
session.idle_timeout_seconds = 86400
session.absolute_timeout_seconds = 604800
session.max_session_bytes = 16384
session.store_timeout_seconds = 5
session.rolling = false
session.keep_session_info_on_login = false
oauth2.state_ttl_seconds = 600
oauth2.pkce = true
oauth2.state_cookie = HttpOnly; Secure; SameSite=Lax; Path=/
oauth2.max_code_bytes = 2048
oauth2.connect_timeout_seconds = 5
oauth2.timeout_seconds = 10
oauth2.max_token_bytes = 65536
oauth2.max_profile_bytes = 262144
oauth2.max_in_flight = 32
oauth2.redeemed_states_max = 100000
url_credentials.local = off
url_credentials.api_key = off
url_credentials.jwt = off
```

Known differences between strategies, each with its reason: the OAuth2 client secret has no minimum length (the provider issues
it); the JWT `from_body_field` extractor, `SessionCookie::without_same_site()` and the largest `clock_tolerance` log no startup
warning while the query extractor, a cookie extractor without `__Host-` and a non-`Secure` cookie do (an ambient cookie
weakness is in the tables above; the others are choices the application makes on purpose); `JwtService` accepts what the JWT strategy refuses (#174).

## Dependencies

The cryptographic crates each auth feature pulls, with the version in `Cargo.lock`. `crates/corvis/tests/auth_source_checks.rs`
fails when the major version of a listed crate changes without this table, and `deny.toml` forbids a second version of every
crate marked "one generation" (`cargo deny check`). No advisory applies to any of them in the RustSec database at the time of
the review (`cargo audit` and `cargo deny check`, 2026-11-10); neither ignore list (`deny.toml`, `.cargo/audit.toml`)
names one of them.

| Feature | Crate | Version | Features enabled | Role | Policy |
|---|---|---|---|---|---|
| `jwt` | `jsonwebtoken` | 9.3.1 | `default`, `pem`, `simple_asn1`, `use_pem` | token signature and decoding | one generation |
| `jwt` | `ring` | 0.17.14 | `alloc`, `dev_urandom_fallback`, `std` | signature primitives under `jsonwebtoken` and rustls | one generation |
| `auth-local` | `argon2` | 0.5.3 | `alloc`, `default`, `password-hash`, `rand` | argon2id | one generation |
| `auth-local` | `blake2` | 0.10.6 | none | hash inside argon2 | one generation |
| `auth-local` | `password-hash` | 0.5.0 | `alloc`, `rand_core` | PHC string format | one generation |
| `auth-local` | `bcrypt` | 0.17.1 | `getrandom`, `std`, `zeroize` | bcrypt hashing and verification | one generation |
| `auth-local` | `blowfish` | 0.9.1 | `bcrypt` | cipher inside bcrypt | one generation |
| `auth-local` | `cipher` | 0.4.4 | none | block cipher traits | one generation |
| `auth-api-key` | `subtle` | 2.6.1 | `i128`, `std` | constant-time comparison | one generation |
| `auth-api-key` | `sha2` | 0.10.9 | `std` | key digests | one generation |
| `auth-session` | `hmac` | 0.12.1 | none | cookie signatures | one generation |
| `auth-session` | `digest` | 0.10.7 | `alloc`, `mac`, `std`, `subtle` | traits under `hmac` and `sha2` | one generation |
| `auth-session` | `rand` | 0.8.8 | `getrandom`, `std`, `std_rng` | `OsRng` for identifiers and states | one generation |
| `auth-oauth2` | `tokio-rustls` | 0.26.6 | `ring`, `tls12` | TLS client for the provider | one generation |
| `auth-oauth2` | `rustls` | 0.23.46 | `ring`, `std`, `tls12` | TLS | one generation |
| `auth-oauth2` | `rustls-webpki` | 0.103.16 | `alloc`, `ring`, `std` | certificate verification | one generation |
| `auth-oauth2` | `webpki-roots` | 0.26.11 | none | trust anchors; a release that only re-exports 1.0.11 | shim of 1.0.11, not in the list |

`openssl`, `openssl-sys` and `native-tls` are banned, so the provider client, TLS termination and token verification keep one
reviewed stack. Two crates have more than one generation in the graph and are not in the single-generation list, for these
reasons: `getrandom` (three generations: the operating system generator comes in through `rand` 0.8, `bcrypt`, and
dependencies that are not authentication crates) and `rand_core` (0.6 for `rand` 0.8, 0.10 for other crates); `Cargo.lock`
also lists `rand` 0.10, which comes from `proptest` (tests only), and `webpki-roots` 1.0.11, which is what `webpki-roots`
0.26.11 re-exports. Secrets are generated with `OsRng` in `crates/corvis/src/auth/session/session.rs`,
`crates/corvis/src/auth/oauth2/state.rs` and `crates/corvis/src/auth/local/hasher.rs`; the source check fails if a file of the
authentication modules uses another generator.

## Findings

Findings of the earlier, per-change reviews, with their fixes and regression tests, are in the notes of each archived change
(the "Security review" sections of `design.md`): the strategy core
(`openspec/changes/archive/2026-11-10-e3-strategy-core-and-guard-responses/`), JWT
(`openspec/changes/archive/2026-11-10-e3-jwt-strategy/`), local and password hashing
(`openspec/changes/archive/2026-11-10-e3-local-strategy-and-password-hashing/`), API keys
(`openspec/changes/archive/2026-11-10-e3-api-key-strategy-and-custom-strategies/`), sessions and signed cookies
(`openspec/changes/archive/2026-11-10-e3-sessions-and-signed-cookies/`) and OAuth2
(`openspec/changes/archive/2026-11-10-e3-oauth2-strategies/`).

This review looked at interactions only (identity, chains, shared places, guard order and public routes, OAuth2 with
sessions, cookies on failure paths, CSRF posture, sockets, defaults, secrets in output, resource use, feature
combinations). A finding is fixed in the change when the fix stays inside the authentication modules and changes no
requirement of another capability; anything else is filed as an issue. No finding of high or critical severity was
deferred.

| Id | Severity | Components | Outcome |
|---|---|---|---|
| R-1 | Medium | `crates/corvis/src/jwt.rs`: `Debug` of `JwtModuleOptions` and `JwtSignOptions` printed the HMAC secret and the PEM private key | Fixed: `Debug` shows whether a secret or key is set, never its value. Test: `debug_of_every_public_type` in `crates/corvis/tests/auth_invariants.rs` fails without the fix |
| R-2 | Low | the guard-order check: a `ThrottlerGuard` after an `AuthGuard` limits nothing, and nothing said so | Fixed: startup warning, `ThrottlerGuard::as_any`. Tests: `crates/corvis/tests/auth_guard_order_log.rs` |
| R-3 | Low | login rate limiting: the per-account recipe could be dodged by moving the username to the query string or a nested field | Fixed: `login_tracker` reads what the strategy reads. Tests: `crates/corvis/tests/auth_login_throttle.rs` |
| R-4 | Info | an OAuth2 strategy in a chain ends it with a redirect | Documented here and in the [Passport recipe](/recipes/passport); pinned by `an_oauth2_strategy_in_a_chain_redirects_instead_of_falling_through` |
| R-5 | Info | the API key `bearer()` reader looks through at most `max_key_bytes + 128` bytes of leading whitespace, the token reader through any amount | Nothing to fix: the difference makes the API key reader stricter, never more permissive; pinned in `bearer_value_shared_by_two_strategies` |
| R-6 | Info | a forged OAuth2 callback gets the browser's own state cookie back unchanged (the genuine flow stays usable) | Intended; pinned in `failed_oauth2_callback` |
| #173 | Medium | strategies at the WebSocket handshake: `AuthGuard` denies socket messages and handshakes authenticate with `JwtService` | Issue filed; use `handle_connection` and the rules of the JWT strategy until it lands |
| #174 | Low | `JwtService` defaults: secrets under 32 bytes and tokens without `exp` are accepted | Issue filed; use the JWT strategy for authentication |
| #172 | Low | redirect URI allow-list for OAuth2 | Issue filed |
| #170, #171 | Info | Redis and ORM session stores | Issues filed; the memory store is per process |
| R-7 | Low | `from_cookie` in the JWT extractors read the first of several cookies of one name, so a cookie planted from a sibling subdomain in front of the victim's own authenticated as the attacker, also through `chain(["jwt", "session"])` | Fixed: a name that appears more than once yields no token, and a startup warning names a cookie extractor without `__Host-`. Test: `a_jwt_cookie_sent_twice_is_no_token` in `crates/corvis/tests/auth_invariants.rs` |
| R-8 | Low | the guard-order warning looked for a throttler only in guards after the one where it saw the `AuthGuard`: a composite guard exposing `[AuthGuard, ThrottlerGuard]` through `inner_guards` got no warning and limited nothing | Fixed: the guards are flattened in order through `inner_guards`. Test: `crates/corvis/tests/auth_guard_order_log.rs` |
| R-9 | Low | an OAuth2 flow whose state is in a cookie survived a logout and another user's login on a shared browser, and completed with `Request::user()` set to the new user | Fixed: the flow is bound to the session it started under. Test: `oauth2_cookie_state_is_bound_to_the_session_it_started_under` in `crates/corvis/tests/auth_invariants.rs` |
| R-10 | Low | the JWT key provider (reachable by any well-formed token), the JWT validator and the local validator had no deadline, while the API key validator had one | Fixed: `key_provider_timeout` and `validator_timeout`, 5 s, `503`. Tests: `crates/corvis/tests/auth_callback_timeouts.rs` |
| R-11 | Info | a guard that passed cleared the user but kept the auth info of an earlier guard | Fixed. Test: `a_later_pass_clears_the_auth_info_of_an_earlier_guard_too` in `crates/corvis/tests/auth_hardening.rs` |
| #177 | Medium | `CacheInterceptor` registered globally serves one user's `GET` response to others on authenticated routes | Issue filed (cache module, outside this change); see the warning at the top of the threats section |
| #178 | Low | Socket.IO: a connect middleware on the main namespace does not cover the others, and `Debug` of `Handshake` prints the `auth` payload | Issue filed |

Proposed issues from this review, for the maintainer to file: a session store outage fails public routes and health checks
for browsers that carry a session cookie (low, availability: restore the session user lazily or skip loading on `#[public]`
routes); coverage-guided fuzzing of the authentication parsers (`cargo fuzz`, informational); supply-chain attestations for the
cryptographic crates (`cargo vet`, informational).
