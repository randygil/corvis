---
status: documented
---

# Session

Server-side sessions work like `express-session` (the [authentication guide](/security/authentication#sessions) shows login and logout with them, and [Securing authentication](/security/authentication-security) what to do before you deploy): the browser holds a signed cookie with an identifier, and the
data lives in a store on the server. The session cookie and the stored records use the same format as
`express-session` 1.19.0, so a Node application and a Corvis one that share a secret and a store can read each other's
sessions (see [Cross-reading with a Node app](#cross-reading-with-a-node-app)).

## Enabling

Sessions need the `auth-session` cargo feature:

```toml
corvis = { version = "0.1", features = ["auth-session"] }
```

Everything below lives in `corvis::auth::session`, except `CookieKeys`, which is in `corvis::http` (see
[Cookies](/http/cookies#signed-cookies)).

## Setup

`SessionModule` is a global dynamic module. Import it once:

```rust
use corvis::auth::session::{SameSite, SessionCookie, SessionModule, SessionOptions};
use std::time::Duration;

fn session_options() -> SessionOptions {
    SessionOptions::with_secrets([std::env::var("SESSION_SECRET").unwrap_or_default()])
        .expect("SESSION_SECRET must be at least 32 bytes")
        .name("connect.sid")
        .cookie(SessionCookie::default().same_site(SameSite::Strict))
        .idle_timeout(Duration::from_secs(24 * 3600))
        .absolute_timeout(Duration::from_secs(7 * 24 * 3600))
}

#[module(
    imports = [SessionModule::for_root(session_options())],
    controllers = [CartController],
)]
pub struct AppModule;
```

`SessionOptions::with_secrets([..])` and `SessionOptions::new(CookieKeys::new([..])?)` return an error when a secret
is not valid (see [Secrets and rotation](#secrets-and-rotation)). To read the secret from configuration, use
`for_root_async`, which receives a `ResolveCtx`:

```rust
SessionModule::for_root_async(|ctx: ResolveCtx| async move {
    let config = ctx.get::<ConfigService>().await?;
    Ok(SessionOptions::with_secrets([config.get::<String>("SESSION_SECRET").unwrap_or_default()])?)
})
```

All the options:

| Method | Default | Meaning |
|---|---|---|
| `.name("connect.sid")` | `connect.sid` | cookie name |
| `.cookie(SessionCookie::..)` | see [Cookie](#the-cookie) | cookie attributes |
| `.idle_timeout(d)` | 24 h | time without requests after which the session ends |
| `.absolute_timeout(d)` / `.no_absolute_timeout()` | 7 days | maximum life of a session since its identifier was generated |
| `.rolling(bool)` | `false` | send the cookie on every response |
| `.resave(bool)` | `false` | write the session to the store even when it was not modified |
| `.save_uninitialized(bool)` | `false` | store a new session that nobody wrote to |
| `.keep_session_info_on_login(bool)` | `false` | keep the data of the session across `log_in` and `log_out` |
| `.max_session_bytes(n)` | 16 KiB | size limit of a serialized record |
| `.store_timeout(d)` | 5 s | bound for every call to the store |
| `.store(store)` | `MemorySessionStore::new()` | where sessions are kept |

Invalid options stop `init()` with an error that names the option and never the secret. The startup log line states the
cookie attributes, and warnings are logged for `secure(false)`, `http_only(false)`, `no_absolute_timeout()` and for the
memory store.

## The `Session` API

`Request::session()` returns an `Option<Session>`: `None` when no session module runs for the request (for example a
request outside the cookie `Path`). The API is synchronous: the middleware does all the store access before the guards
run and after the handler returns.

```rust
use corvis::auth::session::Session;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Cart {
    items: Vec<String>,
}

#[controller("cart")]
pub struct CartController;

#[routes]
impl CartController {
    #[post("add")]
    fn add(&self, #[req] req: Request, #[body] dto: AddItemDto) -> HttpResult<Cart> {
        let session = req.session().ok_or_else(|| InternalServerErrorException::new("no session"))?;
        let mut cart: Cart = session.get("cart").unwrap_or_default();
        cart.items.push(dto.item);
        session.set("cart", &cart).map_err(HttpException::internal)?;
        Ok(cart)
    }
}
```

| Method | What it does |
|---|---|
| `id()` | the session identifier |
| `is_new()` | the session was created by this request |
| `get::<T>(key)` | a value deserialized as `T`, or `None` |
| `get_value(key)` | the JSON value of a key |
| `data()` | every value |
| `set(key, value)` | stores a value. Fails with `SessionError` for the reserved keys `cookie` and `__corvis`, or a value that does not serialize |
| `remove(key)`, `clear()` | remove one key or all of them |
| `regenerate()` | a new empty session with a new identifier; the old record is destroyed |
| `regenerate_keeping_data()` | a new identifier and the same data. Call it when privileges change and the session must go on |
| `destroy()` | the record is destroyed when the request ends, the cookie is cleared and the data dropped. Values set afterwards start a new session |

`regenerate()` is the one of `express-session`. `Debug` of a `Session` prints neither the identifier nor the data.

## The cookie

The cookie is named `connect.sid` and its value is `s:<id>.<signature>`, URI-encoded, signed with the first secret and
verified with all of them. The identifier is 24 random bytes from the operating system generator, in base64url (32
characters); it is never derived from user data. An identifier that verifies but that the store does not know is never
adopted: a new one is generated.

`SessionCookie::default()` gives `Path=/; HttpOnly; Secure; SameSite=Lax`, no `Domain` and no expiry:

| Attribute | Default | Why |
|---|---|---|
| `HttpOnly` | on | a script (and so an XSS) cannot read the identifier |
| `Secure` | on | the browser does not send the cookie over a clear-text connection. The `Set-Cookie` of a response issued over plain HTTP still travels in clear, and so does whatever is in it |
| `SameSite` | `Lax` | the browser keeps the cookie off cross-site `POST` and `fetch` requests. It is not CSRF protection by itself, see [CSRF](/security/csrf) |
| `Path` | `/` | the whole application. A request whose path does not start with the `Path` has no session |
| `Domain` | none | the cookie goes only to the host that issued it, not to subdomains |
| expiry | none | a browser-session cookie. The server ends sessions with the timeouts below |

Change them with `SessionCookie`: `.secure(bool)`, `.http_only(bool)`, `.same_site(SameSite::Strict)`,
`.without_same_site()`, `.max_age(Duration)`, `.domain("example.com")` and `.path("/app")`. With `max_age` the cookie
carries `Expires` and never `Max-Age`, like the original, and never later than the absolute limit of the session.

The `Secure` cookie is sent on every response: the framework cannot tell the transport, and a reverse proxy usually
ends TLS. Browsers differ on whether they store a `Secure` cookie from `http://localhost`.

Prefer the `__Host-` prefix (`.name("__Host-sid")`). A browser refuses to set such a cookie unless it is `Secure`, has
`Path=/` and no `Domain`, and, above all, it stops a sibling subdomain from planting it. Corvis validates those rules at
startup for `__Host-` and for `__Secure-`, without regard to case, as browsers match them (`__host-sid` with a `Domain`
does not start).

::: warning Cookie tossing
Without the `__Host-` prefix, a sibling subdomain (or any site that can run code on one) can set a cookie with the session
name for the whole domain. Two cookies with that name load no session (see below), so the planted one logs the victim out
for as long as it lives, and the victim cannot log in again from that browser until it is removed. Use `__Host-`.
:::

::: warning The cookie name is not part of the signature
Like the original, the signature covers the identifier and nothing else. A value signed with the same secret for another
cookie verifies as a session cookie value, and the session cookie sent back under another name that application code
reads with `signed_cookie` hands that code the session identifier. Never sign your own cookies with the session secrets:
give the application its own `CookieKeys` (see [Cookies](/http/cookies)).
:::

### Plain-HTTP development

Locally, without TLS, turn `Secure` off and nothing else:

```rust
SessionOptions::with_secrets([secret])?.cookie(SessionCookie::default().secure(false))
```

A warning is logged at startup. Do it from configuration, not in code that reaches production.

### Invalid and repeated cookies

Invalid cookies (absent, without `s:`, with a bad signature, an identifier that is not 1 to 128 bytes of visible ASCII,
or more than 4096 bytes after decoding) are ignored, with no store access and no error. If the request carries **two or
more** cookies with the session name (for example one planted from a sibling subdomain, also across several `Cookie`
headers), all of them are ignored and the request runs with a new session. This differs from `req.cookie()`, where the
first one wins.

## Secrets and rotation

- A secret must be at least 32 bytes; a shorter one stops startup with `the session secret at position 0 has N bytes:
  the minimum is 32 bytes`. Errors name the position and the length, never the secret.
- 1 to 8 secrets. The first signs, all verify.
- Generate one with `openssl rand -hex 32`, keep it out of the repository and never log it.

To rotate: put the new secret first and keep the old one.

```rust
SessionOptions::with_secrets([new_secret, old_secret])?
```

Keep the old one until the longest cookie lifetime has passed (the absolute timeout, 7 days by default), then remove it.
During the rotation a cookie signed with a retired secret is accepted, and the response sets it again signed with the new
one.

## Timeouts

Every record carries `__corvis: { createdAt, lastAccess }` (milliseconds) next to the `cookie` object and the data. The
middleware enforces the timeouts from those timestamps on every load, whatever the store does with the ttl:

- **Idle timeout** (24 h): time since the last access.
- **Absolute timeout** (7 days): time since the identifier was generated. Login, logout and `regenerate` start a new
  period. `.no_absolute_timeout()` removes it and logs a warning.
- `cookie.expires` in the past also ends the session.

An expired record is destroyed and the request gets a new session; if a valid cookie arrived, the browser is told to drop
it. `rolling` only extends idle time and the cookie: it never extends a session past the absolute timeout (the cookie
expiry is capped at it too).

A record without `__corvis` (written by the original) is accepted and receives both timestamps at its first load.
Timestamps from the future count as now, and unusable timestamps destroy the record.

The ttl given to stores is the smallest of the idle timeout, the remaining absolute time and the remaining life of the
cookie (at least 1 s). Every store call is bounded by `store_timeout`; a timeout is the [same 500](#failure-behavior) as
a store error.

## Limits

| Limit | Default | Range |
|---|---|---|
| secrets | none | 1 to 8, each at least 32 bytes |
| signed input | | at most 4096 bytes |
| identifier | 24 random bytes (32 characters) | accepted: 1 to 128 bytes of visible ASCII |
| cookie name, path, domain | `connect.sid`, `/`, none | at most 128, 1024 and 253 bytes |
| `idle_timeout` | 24 h | 60 s up to the absolute timeout |
| `absolute_timeout` | 7 days | at most 400 days |
| cookie `max_age` | none | at most the absolute timeout |
| `max_session_bytes` | 16 KiB | 1 KiB to 1 MiB |
| nesting depth of a record | 100 levels | the record is level 1, so a value nests 99 levels |
| `store_timeout` | 5 s | 100 ms to 60 s |
| memory store `max_sessions` | 100 000 | at least 1 |
| memory store `max_total_bytes` | 256 MiB | at least 1 |
| memory store `max_sessions_per_user` | 16 | at least 1 |

A stored record larger than `max_session_bytes` (written by another writer of a shared store) is no session when it is
read: it is not loaded, its identifier is not adopted and the record is not deleted. A warning, at most once a minute and
without identifiers, reports it.

A record nested deeper than 100 levels (counting the object that holds the session) is refused when it is saved, with the
[same 500](#failure-behavior) as a session over `max_session_bytes`, so the handler's change is not silently lost later: a
parser reads at most 128 levels, so a deeper record could be written and never read back. A stored record deeper than the
limit is no session when it is read, like an oversized one. A record that cannot be parsed at all is destroyed by the
memory store and is no session, with a warning at most once a minute.

Other combinations that stop startup: `SameSite=None` without `Secure`, a `__Host-` name without `Secure`, with a `Domain`
or with a `Path` other than `/`, a `__Secure-` name without `Secure`, and an invalid cookie name, path or domain.

## The memory store

`MemorySessionStore` is the default. It is for one process and for development:

```rust
.store(MemorySessionStore::new().max_sessions(100_000).max_total_bytes(256 << 20).max_sessions_per_user(16))
```

- Sessions are lost when the application restarts and are not shared between instances. Startup logs a warning for it.
- Records are kept serialized, and the byte bound counts each record's allocation, its identifier, its user key and a
  fixed overhead per entry (640 bytes), so `max_total_bytes` bounds the memory the store uses, whatever the shape of the
  data (an array of small numbers costs what its text costs). A test with a counting allocator checks it. The hash tables
  give back their room when they hold less than a quarter of it, so a burst of sessions that is over leaves nothing behind.
- Nothing proportional to the size of a record happens while the store's lock is held: a load clones a reference and
  parses outside the lock, and a request that changed nothing only replaces a small fragment (`cookie` and `__corvis`) of
  the entry. Large sessions of one user do not slow other users down.
- Expired records are dropped without a background task: on load and a few on each write.
- **Per user:** at most `max_sessions_per_user` sessions (16 by default), keyed by the serialized `passport.user`. A new
  session of a user at the cap evicts that same user's oldest session, never another user's: one account that logs in
  over and over cannot log the others out.
- **When the store is full** it evicts the least recently used **anonymous** session (no `passport.user`) first, then the
  oldest session of the user with the most sessions. A flood of anonymous sessions fills the store with its own records
  but logs nobody out. One rate-limited warning is logged with the count, no identifiers. Under a flood by many
  accounts the users who hold several sessions lose their oldest first.
- A record larger than `max_total_bytes` is an error.

Other stores need their own policy: a shared store has to bound the sessions of one user itself (the contract of
`SessionStore` does not).

By default only a route that writes to the session creates one, so an anonymous visit costs nothing. If a route writes
for anonymous visitors, rate limit it with [`ThrottlerModule`](/security/rate-limiting). Rate limit the login route too:
every login creates a session.

Redis and ORM stores are not part of the framework yet; they are planned as follow-up work. Until then, write the store
yourself.

## Writing a store

Implement `SessionStore`, an async trait:

| Method | Contract |
|---|---|
| `load(id) -> Result<Option<Value>, _>` | the record, or `None` |
| `create(id, &record, ttl)` | must fail with `SessionStoreError::Exists`, leaving the record, when the identifier exists |
| `update(id, &record, ttl) -> Result<bool, _>` | `Ok(false)` and nothing written when the record is missing |
| `touch(id, &record, ttl) -> Result<bool, _>` | **required, no default.** It must renew the expiry and the `cookie` and `__corvis` keys only, never the data: the copy in `record` was read when the request began and may be stale, so an unmodified request must not overwrite what another request saved. `Ok(false)` and nothing written when the record is missing |
| `destroy(id) -> Result<bool, _>` | `Ok(true)` when a record was removed, `Ok(false)` when it was already gone (not an error). The middleware uses it to learn that another request ended the session |
| `in_process() -> bool` | `false` by default; `true` makes startup log the memory warning |
| `validate() -> Result<(), String>` | checked at startup |

`create` and `update` have to be atomic (`SET NX` and `SET XX` in Redis, a conditional statement in SQL): checking that the
record exists and then writing in two steps reopens the window that lets a slow request undo a logout.

Errors are `SessionStoreError::{Exists, Timeout, Backend(String)}`; build the last with `SessionStoreError::backend(msg)`
and keep identifiers and data out of the message.

Records are the JSON of `express-session`: a `cookie` object (`originalMaxAge`, `expires` as an ISO string or `null`,
`secure`, `httpOnly`, `domain` when set, `path`, `sameSite` in lowercase when set), the data keys, and
`passport: { user: <serialized> }` for a logged-in user. That is the format `connect-redis` style stores use.

## Saving rules

These are the rules of `express-session` 1.19.0 for the same options.

- **New identifier:** `create` when the session was modified or `save_uninitialized` is on, and `Set-Cookie` with it.
- **Existing identifier:** `update` when modified or `resave` is on, otherwise `touch`.
- **`Set-Cookie`** on an existing session when `rolling` is on, when the cookie has `max_age` and the session was modified,
  or when the incoming cookie verified only with a retired secret (it is signed again with the current one).

"Modified" means the data differs from what was loaded or the identifier changed; setting a key to the value it already
has is not a modification. The store write is awaited before the response is returned. If the request ends in an error
before a response exists (for example a `404`), an existing session is updated or touched, but a new one is not stored.

The defaults `resave(false)` and `save_uninitialized(false)` differ from the original, where both default to `true`: an
anonymous visit creates nothing.

## Failure behavior

A commit that fails (a store error, a timeout, a session over `max_session_bytes`, or a failure of the identifier
generator) turns the response into a plain `500`, identical for every session, with no cookie:

```json
{"statusCode":500,"message":"Internal server error"}
```

The side effects of the handler have already happened. A store failure on load is the same `500`. A warning is logged at
most once a minute per reason, without identifiers, cookie values, data or the store's own message.

Concurrency: between concurrent requests of one session the last write wins, but destruction is final. `update` and
`touch` never create a record, and `create` never overwrites (a collision is retried once with a new identifier). A slow
request that started before a logout (or `destroy()`) cannot bring the session back: when the request ends, a session it
loaded whose record is gone (the destruction it queued found nothing, or `update` / `touch` found no record) is treated
as destroyed. Nothing is created or updated, whatever the request did to the session (`regenerate_keeping_data()`,
`regenerate()`, a data change), and the cookie is cleared. If the client disconnects mid-request, identifiers queued for
destruction (logout, `regenerate`, `destroy`) are still destroyed.

Three cases are decided on purpose:

- **A login over an anonymous session that keeps nothing of it** (the default: the data of before the login is dropped)
  creates the new session even when the previous record is gone. Otherwise someone who floods the store until the
  victim's anonymous session is evicted could make the login answer `200` and set no cookie. The user just proved who
  they are and nothing of the old session is brought back; the old identifier is destroyed all the same. With
  `keep_session_info_on_login(true)`, `regenerate_keeping_data()` and plain saves the rule above applies. It also applies
  to a login over a session that **already had a user**: such a login is not a fresh proof of identity (a handler may
  call `log_in` with the very user the session restored), so if request A logs out cookie C0 while request B on C0 logs
  in again, the session stays ended and the user logs in once more.
- **A logout does not end a successor.** A logout that loaded session X does not end a session Y that another concurrent
  request created by regenerating X. This is the behavior of the original; closing it would need a "moved to" marker in
  the record (a candidate follow-up).
- **A client that disconnects during a login made by the guard** gets the previous session destroyed, not restored: the
  login is discarded and so is the identifier it replaced, the safe direction.

## Login and logout

Logging in through a session needs a `SessionSerializer`, that decides what is stored for the user and how to get it back,
and a `SessionStrategy<S>`, registered as the strategy named `"session"` (`SESSION_STRATEGY`):

```rust
use corvis::auth::session::{SessionSerializer, SessionStrategy};

#[injectable]
pub struct UserSerializer {
    users: Arc<UsersService>,
}

#[async_trait]
impl SessionSerializer for UserSerializer {
    type User = Account;

    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!({ "id": user.id }))
    }

    async fn deserialize_user(&self, _req: &Request, stored: Value) -> Result<Option<Account>, HttpException> {
        let Some(id) = stored.get("id").and_then(Value::as_u64) else { return Ok(None) };
        Ok(self.users.find(id).await) // None when the user is gone
    }
}

// providers = [UsersService, Provider::strategy::<SessionStrategy<UserSerializer>>()]
```

**`serialize_user` must be stable across logins.** The memory store keys its per-user cap (`max_sessions_per_user`) by the
serialized user. If the value holds a timestamp, a random token or anything else that differs from one login to the next,
every login looks like a new user and the cap never applies. Store the identifier of the user and nothing that changes.

A second `SessionStrategy` stops startup. On every request whose session has `passport.user`, the middleware calls
`deserialize_user` before the guards:

- `Some(user)`: the typed user and its JSON are set on the request (`User<T>` and `Request::user()`).
- `None`: `passport.user` is removed from the stored session and the request is anonymous.
- `Err(e)`: `e` is the response and the handler does not run.

A user whose JSON is `null`, `false`, `0` or `""` is never restored and never stored; logging in with one is a `500`
with a fixed message. Wrap such identifiers in a struct, as the example above does.

The login route uses the `"local"` strategy (see [Passport](/recipes/passport#local-strategy-username-and-password)) with
`.log_in()`:

```rust
#[controller("auth")]
pub struct AuthController {
    authenticator: Arc<Authenticator>,
}

#[routes]
impl AuthController {
    #[post("login")]
    #[use_guards(AuthGuard::new("local").log_in())]
    fn login(&self, user: User<Account>) -> Account {
        user.0
    }

    #[get("me")]
    #[use_guards(AuthGuard::new("session"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }

    #[post("logout")]
    #[http_code(204)]
    async fn logout(&self, #[req] req: Request) -> HttpResult<()> {
        self.authenticator.log_out(&req).await
    }
}
```

`Authenticator::log_in(&req, &AuthenticatedUser::new(user)?)` does the same from your own code (`AuthGuard::new("local")
.log_in()` calls it after a successful authentication). It:

- generates a new identifier and destroys the old record, before the new one is created;
- drops the data of before the login, unless `keep_session_info_on_login(true)`;
- stores the serialized user under `passport.user` and sets the request user.

Replacing the identifier prevents session fixation. `Authenticator::log_out(&req)` removes the user, generates a new
identifier, destroys the old record, drops the data unless `keep_session_info_on_login`, and clears the request user. A
session without a record yet is not regenerated: logging out anonymously creates nothing.

Both fail with a `500` and a fixed message, storing nothing, when the request has no session or no `SessionStrategy` is
registered. A failed login (`401`) leaves the session and its identifier unchanged.

**A login made by the guard is kept only when the request is not rejected.** If a later guard denies, a pipe or the
validation fails, the handler returns an `Err` or panics, or an interceptor fails, the login is discarded: the session the
request arrived with, if any, is left as it was, and no logged-in cookie is issued. The router notes the rejection at the
moment it happens, before any [exception filter](/exception-filters) or interceptor can change the response, so what
a filter answers does not matter: a filter that turns the `403` into a redirect to `/mfa`, or into an "access denied" page
with status `200`, does not keep the login. The same holds when an outer interceptor recovers the failure of an inner
one, and when the handler is dropped before it ends (a timeout interceptor). An interceptor that answers by itself
**without** calling the handler and returns normally stands in for the handler: its login is kept.

```rust
#[use_guards(AuthGuard::new("local").log_in(), SecondFactorGuard)] // a denial by the second guard leaves no login,
                                                                   // whatever a filter then answers
```

A request that is cancelled before its response exists (the client disconnects, a timeout) discards the login too: the
previous session is destroyed rather than restored, the safe direction.

A handler that **returns normally** keeps the login, whatever status it sets, including a redirect it issues itself.
So a handler that implements a second step must not use `.log_in()` on the guard of the first step: authenticate with
`AuthGuard::new("local")` (no `.log_in()`), then call `Authenticator::log_in` after the last step has passed. A login made
with `Authenticator::log_in` by a handler is the handler's decision and stands whatever the status.

`AuthGuard::new("local").log_in()` is a startup error when no `SessionStrategy` is registered (the error names the route)
and when the guard names the `"session"` strategy (`cannot be combined`). Without `.log_in()` an `AuthGuard` never writes
the session.

## Guards and the session user

- The middleware restores the user on **every route the module covers**, including routes with no `AuthGuard` and
  `#[public]` routes: `Request::user()` and `User<T>` see it there, as `passport.session()` sets `req.user` everywhere.
- `AuthGuard::new("session")` succeeds with the restored user (it reads it from the session, not from `Request::user()`)
  and answers `401 {"message":"Unauthorized","statusCode":401}` for an anonymous request. The strategy of the original
  always passes.
- Another `AuthGuard` that succeeds replaces the request user for that request.
- An optional guard (`.optional()`), or a strategy that passes, clears the request user when it finds no credentials, like
  `req.user = null` in the original. So `AuthGuard::new("jwt").optional()` hides the session user on that route. Use
  `AuthGuard::chain(["session", "jwt"]).optional()` (session first) to keep it and still accept tokens.
- `.skip_public()` guards leave the session user alone on public routes.

::: warning The session user is visible on every route
The middleware restores the user on routes without an `AuthGuard` too. A hand-written guard that reads
`req.user()` (a roles guard, say) therefore authenticates by the cookie alone on those routes, with nothing checked
about the request itself:

```rust
// Authenticated by the session cookie on any route it is attached to, including a cross-site form POST.
ctx.request().user().and_then(|u| u.get("role").cloned()) == Some(json!("admin"))
```

Attach state-changing routes to `AuthGuard::new("session")` as well, and protect them against cross-site requests: the
session cookie is an ambient credential. See [CSRF protection](/security/csrf), which also covers login CSRF:
`SameSite=Lax` does not stop it.
:::

WebSocket upgrades and Socket.IO handshakes are answered before the HTTP middleware, so `Request::session()` does not
exist there and a session cookie does not authenticate a socket. Authenticate in `handle_connection` some other way (see
[Gateways](/websockets/gateways)).

## Middleware order

The middleware of `SessionModule` is module middleware, so it runs after middleware registered with
`app.use_middleware(..)`, which therefore do not see the session. To run first, register it yourself with the
`Arc<SessionManager>`:

```rust
app.use_middleware(SessionMiddleware::new(manager));
```

A request that already has a session passes through the second registration untouched. The middleware applies to every
route, like `app.use(session())`. A request without a valid cookie costs no store access, and static files served by the
router also run it when they carry a cookie.

## Cross-reading with a Node app

With the same cookie name (`connect.sid`), the same secret and the same store records, each side reads the sessions of the
other: `passport.user` holds the serialized user, and the extra `__corvis` key is ordinary data to Node. The Corvis
cookie has `Expires` and no `Max-Age`, as the Node one.

This is tested against `express-session` 1.19.0, `passport` 0.7.0, `cookie-signature` 1.0.7 and `cookie-parser` 1.4.7 with
recorded fixtures (`crates/corvis/tests/fixtures/auth/session.mjs` and `signed_cookies.mjs`).

## Divergences from the original

| `express-session` | Corvis |
|---|---|
| no `SameSite`, no `Secure` by default | `SameSite=Lax` and `Secure` |
| `resave` and `save_uninitialized` default to `true`, with a warning | both `false` |
| no extra keys | a `__corvis` key with timestamps |
| no idle or absolute timeout | idle (24 h) and absolute (7 days) timeouts |
| no size limit | 16 KiB per record |
| `store.set` creates or overwrites | `update` and `touch` never re-create a record |
| a login through the guard stays when a later guard or the handler rejects the request | discarded when the pipeline rejects the request (a denying guard, a failing pipe, an error or panic of the handler), whatever a filter answers; a handler that returns normally keeps it |
| a request that outlives a logout can save the session again | a session whose record is gone at the end of the request is treated as destroyed and the cookie is cleared |
| `MemoryStore` keeps every session of an account | at most `max_sessions_per_user` (16) in the memory store |
| the cookie keeps the signature it came with | re-signed with the new secret after a rotation |
| a `Secure` cookie is not sent over plain HTTP | it is sent on every connection |
| secrets of any length | under 32 bytes refused |
| `MemoryStore` grows without bound | bounded, with eviction |
| repeated session cookies: the first wins | all ignored |
| nothing sent after `destroy()` or expiry | the cookie is cleared |
| logging out an anonymous session regenerates it | creates nothing |
| the session strategy always passes; apps write `isAuthenticated()` | `AuthGuard::new("session")` fails for anonymous |
| a failed save is the error of the store | a plain `500` |
| store calls without a time limit | bounded by `store_timeout` |
| `j:` JSON signed cookies are decoded | not decoded |
| options `genid`, `unset`, `proxy`, per-request edits of `req.session.cookie`, `store.all`, `store.clear`, `store.length` | not available |

The original never adopts an identifier its store does not know, and Corvis behaves the same.

## What is not covered

- **WebSockets.** See above: there is no session on a socket.
- **CSRF tokens.** The cookie is `SameSite=Lax` by default, which helps but does not replace a check. See
  [CSRF](/security/csrf).
- **Redis and ORM stores.** They come later; the record format is compatible with `connect-redis` style stores.

## Several instances

The memory store belongs to one process: with several instances (or a restart on every deploy) users are logged out or
see a different session on each request. Run one instance, or use a shared store that you implement with `SessionStore`.
Every instance must use the same secrets.

If all you need is to know who the user is, a token signed with `JwtService` needs no store. See
[Authentication](/security/authentication).
