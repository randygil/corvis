---
status: documented
---

# Rate limiting

Limiting how many requests a client can make in a time window protects the API from abuse and
brute-force attacks. Corvis solves this with `ThrottlerModule`, which stores the counters, and
`ThrottlerGuard`, which checks them on every request.

## Configuration

Import the module with one or more limits and register the guard. `ThrottlerOptions::new(ttl, limit)`
takes the window in **milliseconds** and the maximum number of requests per window:

```rust
use corvis::prelude::*;

#[module(
    imports = [
        ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 100)]),
        AuthModule,
        CatsModule,
    ],
    providers = [
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::app_guard::<AuthGuard>(),
    ],
)]
pub struct AppModule;
```

`ThrottlerModule::for_root` is global: you do not need to import it in every module. With
`Provider::app_guard` the guard is applied to all controller routes; you can also
apply it only to a controller or to a method with `#[use_guards(ThrottlerGuard)]`.

When the limit is exceeded, the request receives:

```json
{ "statusCode": 429, "message": "ThrottlerException: Too Many Requests" }
```

## How counting works

- The key of each counter combines the limit name, the client IP (`req.ip()`), the
  controller and the handler. Each route has its own counter per client. The IP is normalized like the original
  library does: an IPv4 address is used as it is, an IPv4-mapped IPv6 address (`::ffff:1.2.3.4`) counts as its
  IPv4 form, `::1` stays alone, and any other IPv6 address counts as its /64 prefix (see
  [IPv6 clients](#ipv6-clients)).
- The window slides: each request counts for `ttl` milliseconds from the moment it was made, and the
  count of a request is the number of hits of that key in the last `ttl`. A client cannot send twice the
  limit around the edge of a fixed window.
- The request that takes the count over the limit blocks the key (see [Blocking](#blocking)). Requests
  rejected while the key is blocked are not counted and do not extend the block.
- With several limits in the vector, they are checked in ascending `ttl` order (ties keep the order
  given) and the first one that rejects the request stops the
  check: later limits do not count that request. To
  tell them apart, `ThrottlerOptions` has a public `name` field (`"default"` by default):

  ```rust
  ThrottlerModule::for_root(vec![
      ThrottlerOptions { name: "short".into(), ttl: 1_000, limit: 3 },
      ThrottlerOptions { name: "long".into(), ttl: 60_000, limit: 100 },
  ])
  ```

## Blocking

A key that goes over its limit is blocked for the **block duration**, counted from the moment of the
block: every request in that time answers `429` with `Retry-After` set to the time left. The first request
after the block starts counting again from one. The block duration is the throttler's `ttl` (after its ttl
override) unless you set it:

- For a throttler, on the storage, by throttler name: `ThrottlerStorage::new(...).with_block_duration("default", 300_000)`
  (milliseconds). A name that matches no throttler panics when the storage is built.
- For a route or controller, with `throttle("name").block_duration(ms)` (milliseconds, for that throttler; see
  [Overriding one throttler](#overriding-one-throttler)); it takes precedence over the storage value.
- `0` turns the block off: a request is rejected while the key already holds `limit` hits from the last
  `ttl` and is not counted; `Retry-After` is the time until the oldest of those hits expires.

A limit of `0` rejects every request, including the first one after a block. The original library
(6.7.1) lets one request through each time a block ends; Corvis does not, like the library's current source.

This changes the behavior of apps that used the guard before: the block now lasts a whole block duration
from the moment of the block instead of the rest of a fixed window, and hits are counted in a sliding
window.

## Changing the limit per route

Routes and controllers override the options of one throttler, as `@Throttle({ short: { limit, ttl, blockDuration } })`
does in the original library. The guard reads these metadata keys, on the method or on the controller:

| Constant or helper | Effect |
|---|---|
| `SKIP_THROTTLE` | `#[set_metadata(SKIP_THROTTLE, true)]` skips the throttler named `default`, like a bare `@SkipThrottle()` |
| `skip_throttle([..])`, `skip_throttle_key(name)` | Skip throttlers by name (see [Skipping throttlers](#skipping-throttlers)) |
| `throttle(name)` | Overrides one throttler: `.limit(n)`, `.ttl(ms)`, `.block_duration(ms)` (`0`: no block), `.tracker(id)`, `.key_generator(id)` |
| `throttle_key(OPTION, name)` | The metadata key of one option of one throttler, for `#[set_metadata]` (`THROTTLER:LIMIT` followed by the name) |
| `THROTTLE_LIMIT`, `THROTTLE_TTL`, `THROTTLE_BLOCK_DURATION` | The bare keys: the entry of the throttler named `default` only |
| `THROTTLE_TRACKER`, `THROTTLE_KEY_GENERATOR` | Prefixes of the tracker and key generator keys; they have no bare form |

### Overriding one throttler

`throttle(name)` returns a builder that turns into a `CustomDecorator`, so it goes in `#[apply_decorators]` on a
controller or a route (`@Throttle({ short: { limit: 1 }, long: { limit: 100 } })`):

```rust
#[routes]
impl AuthController {
    #[post("login")]
    #[apply_decorators(throttle("short").limit(1), throttle("long").limit(5).ttl(120_000))]
    async fn sign_in(&self, #[body] dto: SignInDto) -> HttpResult<Value> {
        // ...
    }

    #[get("health")]
    #[set_metadata(SKIP_THROTTLE, true)]
    fn health(&self) -> &'static str {
        "ok"
    }
}
```

- An override changes the named throttler and no other: with `short` (3 per 30 s) and `long` (5 per 60 s),
  `throttle("short").limit(1)` leaves `long` at 5.
- Each option is resolved on its own, handler first and then controller, and the first value present wins. A handler
  that only sets `limit` keeps the controller's `ttl` for that throttler. An option nobody sets falls back to the
  throttler's configuration (and the block duration to `with_block_duration`, then to the effective `ttl`).
- A JSON `null` counts as absent: on a handler it falls through to the controller's value, which can be looser than the
  throttler's own, so do not use it to tighten a route.
- Gateway handlers have no `#[apply_decorators]`; use `#[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), 1)]`.
- The bare `THROTTLE_LIMIT`, `THROTTLE_TTL` and `THROTTLE_BLOCK_DURATION` keys are the `default` throttler's entry,
  like the bare `SKIP_THROTTLE`: `#[set_metadata(THROTTLE_LIMIT, 5)]` is `throttle("default").limit(5)`. When a target
  has both, the per-name entry wins.
- Values are checked. `limit` must be an integer from 0 to 4,294,967,295, `ttl` and block duration non-negative integers
  (milliseconds), and a tracker or key generator the name of a registered function. Anything else (a string such as
  `"5"`, a negative number, a fraction) answers `500` with the standard body and logs `Invalid "limit" for the "short"
  throttler: ...`; the guard does not fall back to a looser value. The builder only takes integers, so this can only
  happen with a hand-written `#[set_metadata]`.
- A name that no throttler has (`throttle("shrot")`), or a bare key when no throttler is named `default`, answers `500`
  on every request of that route and logs one error per route naming the route, the unknown names and the configured
  ones. The original library ignores such overrides; Corvis fails closed because a misspelled name would otherwise leave
  the route on the looser global limit without any sign.

::: warning Behavior change
Before, `THROTTLE_LIMIT`, `THROTTLE_TTL` and `THROTTLE_BLOCK_DURATION` applied to every throttler: a bare
`THROTTLE_LIMIT` of 5 also raised a `short` throttler from 3 to 5, and a bare `THROTTLE_TTL` gave every throttler the
same window. They now apply to the throttler named `default` only. In an app whose throttlers all have other names,
a bare override now makes that route answer `500` (the guard logs one error per such route); replace it with
`throttle("name")` for each throttler it was meant for.
:::

### Trackers and key generators

By default the tracker is the normalized client IP and the counter key combines the controller, the handler and the
tracker. The original library lets you replace both (`getTracker` and `generateKey`), and so does Corvis:

```rust
use corvis::modules::throttler::normalize_ip;
use corvis::prelude::*;

ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)])
        // the original's getTracker: async, gets the ExecutionContext; an Err is returned as is
        .with_tracker(|ctx| async move {
            let request = ctx.request();
            Ok(match request.header("x-api-key") {
                Some(key) => format!("key:{key}"),
                None => normalize_ip(&request.ip(), 64),
            })
        })
        // the original's generateKey(context, tracker, throttlerName): synchronous
        .with_key_generator(|_ctx, tracker, name| format!("{name}:shared:{tracker}")),
)
```

- `with_tracker` and `with_key_generator` set the module-level hooks. They run for every throttler that is not
  skipped, once per request, so keep them cheap and free of side effects.
- A route chooses another function for one throttler by name, because metadata holds JSON and not closures: register
  it with `with_named_tracker("by-email", f)` or `with_named_key_generator("shared", f)` and select it with
  `throttle("login").tracker("by-email")` or `.key_generator("shared")`. An id that is not registered answers `500`
  (the route is not left unlimited). Registering an id twice panics.
- One throttler can have its own hooks, like an entry of the original's throttler list with `getTracker` or
  `generateKey`: `with_tracker_for("login", f)` and `with_key_generator_for("login", f)` take the same functions as the
  module-level ones and apply to that throttler only. They panic when no configured throttler has that name; calling
  one twice for a name keeps the last function.
- For each throttler the guard uses the route's function (handler, then controller), then the throttler's hook, then
  the module hook, then the built-in.
- Keys from a throttler's generator are stored under their own source marker, so they never meet the keys of the
  built-in, module or registered generators.
- A custom tracker replaces the built-in one entirely, including IPv6 grouping: call `normalize_ip` yourself, as in the
  example.
- The throttler name is always part of the stored key. A key generator that ignores the name (`|_, tracker, _|
  tracker.to_string()`) shares a counter between routes that use it, which is its use, but never between throttlers.
  The stored key also records where it came from (built-in, the module generator, or a registered generator by id), so
  keys from different sources never meet: a client cannot craft a tracker that lands on another route's built-in counter. The original
  library shares the reporting window and the block between two throttlers that get the same key, so its
  `X-RateLimit-Reset-long` shows the other throttler's window and a block by `long` makes `short` answer
  `Retry-After-short`; Corvis keeps each throttler's own values. The differential test records this case and checks
  that Corvis differs from it.
- A generator should not return a string that the client fully controls (a header value): put a constant prefix in
  front of it, as in `format!("shared:{tracker}")`. Routes that use the same generator share counters, so a client
  that chooses the tracker chooses which of those counters it hits.

::: warning Client-chosen trackers
For login routes use `login_tracker` and the [recipe below](#protecting-a-login-route) instead of writing the tracker
yourself: it reads the username the strategy reads, so the counter cannot be dodged by moving the field.

A tracker that reads a header or a body field lets the client choose its counter, so a client that rotates the value
gets a fresh limit each time. Each rotated value also creates a counter, so a client that sends many values fills
`max_entries` and evicts the counters of other clients (which resets them). Keep an IP-based throttler next to a
per-account one (the `default` throttler counts by IP, a named throttler counts by account on the login route with
`.tracker(..)`), but note that it only stops this when it is checked first: throttlers are checked in ascending `ttl`
order and the first that rejects stops the check, so the IP throttler needs the shorter `ttl`; otherwise the account
throttler counts the hit before the IP throttler rejects it. Normalize the account identifier (lowercase an email, for
example) so that spelling variants do not get separate counters, and remember that a limit per account can be used to lock
a victim out on purpose: anyone can send the victim's identifier. A tracker that returns one value for every anonymous
request puts all of them in one counter, and one client can exhaust it for everybody.
:::

### Limits that depend on the request

The original library accepts functions for `limit`, `ttl` and `blockDuration`. In Corvis they are builders of
`ThrottlerStorage` named after the throttler, each taking an async function of the `ExecutionContext`:

```rust
use corvis::prelude::*;

ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions { name: "api".into(), ttl: 60_000, limit: 100 }])
        // Result<u32, HttpException>: the limit of this request
        .with_limit_fn("api", |ctx| async move {
            let plan = plan_of(&ctx).await?; // server-side lookup, e.g. the authenticated user
            Ok(if plan == "pro" { 1000 } else { 100 })
        })
        // Result<u64, HttpException> in milliseconds
        .with_block_duration_fn("api", |_ctx| async { Ok(20_000) }),
)
```

- `with_limit_fn(name, f)` returns a `u32`; `with_ttl_fn(name, f)` and `with_block_duration_fn(name, f)` return
  milliseconds as `u64`. The value type is `ThrottlerValueFn<T>`. The results are typed, so no non-numeric or negative
  value can reach the counters.
- For each throttler and option the guard uses the route override (`throttle("api").limit(5)`), else the function, else
  the static value, whatever the order of the builder calls. The function is not called when a route override applies,
  nor for a skipped throttler. The block duration then falls back to `with_block_duration` and then to the effective
  ttl, as before.
- An `Err` is returned by the guard unchanged (a 500 for an internal error, the exception's own status otherwise) and
  no hit is counted for that throttler or the ones after it.
- An `Err` stops the check: the throttlers after it do not count the request, so keep the IP throttler static with
  the shortest ttl so that it is checked first.
- Keep each value stable per key. A key that is blocked stays blocked until its block ends, whatever value a later
  request gets, but the length of a new block follows the request that causes it.
- A ttl function that returns `0` disables that throttler for the request, as a static ttl of `0` does. A warning is
  logged once per throttler; do not map a missing value (a failed lookup) to `0`.
- The builders panic when no configured throttler has that name, while the module is configured.
- Throttlers with a ttl function are checked after the throttlers with a static ttl, in configured order, as in the
  original; their static `ttl` is not used. Keep the IP throttler static with the shortest ttl so that it is checked
  first.
- Function-valued overrides on a route (`@Throttle({ short: { limit: (ctx) => .. } })`) are not supported: route
  metadata holds JSON.
- Derive the values from server-side state, never from a header the client sets (`x-plan: pro`): the client would
  choose its own limit. The functions run on every request for every throttler that is not skipped, so keep them cheap
  and cache lookups. Use a ttl that depends on stable properties of the client: each hit expires after the ttl it was
  counted with.

### Skipping throttlers

Skips are decided per throttler, as in the original library. `SKIP_THROTTLE` only skips the throttler named `default`; other throttlers keep counting the request and sending their headers. Skip them by name with `skip_throttle`, which returns a `CustomDecorator` (`@SkipThrottle({ short: true, long: true })`):

```rust
#[get("status")]
#[apply_decorators(skip_throttle(["short", "long"]))]
fn status(&self) -> &'static str {
    "ok"
}
```

The names must match `ThrottlerOptions::name`; a name no throttler has skips nothing. A skipped throttler does not count the request and sets no `X-RateLimit-*` or `Retry-After` header.

For each throttler the guard looks at the method first and then at the controller, and the first value found wins. A method can undo a controller's skip for one throttler with `false`, using `skip_throttle_key(name)` (`"THROTTLER:SKIP"` followed by the name, as the original stores it); for `default`, `SKIP_THROTTLE` set to `false` works too:

```rust
#[controller("accounts")]
#[apply_decorators(skip_throttle(["short"]))]
pub struct AccountsController;

#[routes]
impl AccountsController {
    #[post("login")]
    #[set_metadata(skip_throttle_key("short"), false)]
    fn login(&self) { /* ... */ }
}
```

Gateway handlers use the same keys with `#[set_metadata(skip_throttle_key("short"), true)]`, since they have no `#[apply_decorators]`.

::: warning Behavior change
Before, `SKIP_THROTTLE` skipped every throttler. An app whose throttlers all have other names (`short`, `long`) and a route with `SKIP_THROTTLE` now throttles that route. The guard logs one warning per such route on its first request, naming the route; replace the skip with `skip_throttle(["short", "long"])` to keep the old behavior.
:::

## Protecting a login route

A login route is where password guessing happens, and the work it asks of the server (a password hash, a session store
write, a request to an OAuth2 provider) is what an attacker spends your resources on. Two things protect it: a limit per
**client** and a limit per **account**, and both must run **before** the authentication.

The [authentication guide](/security/authentication#username-and-password) shows the login route, and `examples/auth-app` runs
this recipe on its token and session logins and limits both routes of its OAuth2 login. The recipe below limits the local strategy's login to 10 attempts a minute per client address and 5 attempts per account
every five minutes, from any number of addresses. `login_tracker` returns the tracker of the second limit: it reads the
username the local strategy will read from the same request, with the same options (the field name, `user[email]` style
paths, the body first and then the query when the strategy reads it, the length limit), so moving the username to another
place cannot dodge the counter.

```rust
use corvis::auth::local::{LocalStrategyOptions, login_tracker};
use corvis::prelude::*;

// The options the strategy of `LocalAuth::options()` returns.
fn local_options() -> LocalStrategyOptions {
    LocalStrategyOptions::default()
}

#[module(
    imports = [ThrottlerModule::for_root_storage(
        ThrottlerStorage::new(vec![
            // Per client. The shorter ttl: the guard checks throttlers by ascending ttl, so a client that is over its limit
            // stops here and does not add to the per-account counter.
            ThrottlerOptions::new(60_000, 100),
            // Per account, five minutes.
            ThrottlerOptions { name: "login".into(), ttl: 300_000, limit: 100 },
        ])
        .with_named_tracker("login-account", login_tracker(local_options()))
        // The tracker alone is the key, so every login route shares one counter per account.
        .with_named_key_generator("account", |_ctx, tracker, _name| tracker.to_owned()),
    )],
    // A global guard registered through the container runs before every `AuthGuard`.
    providers = [Provider::app_guard::<ThrottlerGuard>(), /* strategies */],
    controllers = [AuthController],
)]
pub struct AppModule;

#[controller("auth")]
pub struct AuthController;

#[routes]
impl AuthController {
    #[post("login")]
    #[apply_decorators(throttle("default").limit(10), throttle("login").limit(5).tracker("login-account").key_generator("account"))]
    #[use_guards(AuthGuard::new("local"))]
    fn login(&self, user: User<PublicUser>) -> Value { /* ... */ }
}
```

`crates/corvis/tests/auth_login_throttle.rs` runs this recipe against the whole authentication module.

What to know about it:

- **Order matters.** Guards run global, then controller, then route, and a `ThrottlerGuard` that runs after an `AuthGuard`
  limits nothing: the password was already hashed. Register the throttler as a global guard with `Provider::app_guard`
  (those run before the instance guards of `use_global_guards`) or list it first. When an `AuthGuard` runs before a
  `ThrottlerGuard` on a route, startup logs a warning naming the method and the path; it is a warning because limiting
  signed-in users per account after authenticating them is legitimate. A guard that wraps both and shows neither through
  `as_any` or `inner_guards` hides them from the check.
- **The default key is per route.** A counter is keyed by the throttler, the controller, the handler and the tracker value, so
  two login routes (a token login and a session login, say) would each allow the full number of attempts for one account.
  The recipe registers `with_named_key_generator("account", ..)` that returns the tracker alone and selects it with
  `.key_generator("account")` on every login route: they then share one counter. The named throttler counts every route that
  does not skip it, by client address when there is no username, so keep it on the login routes only: put
  `#[apply_decorators(skip_throttle(["login"]))]` on the other controllers and routes, or give the throttler a tracker of its
  own at the module level. Behind a proxy, set `trust proxy` (`app.set_trust_proxy(..)`), or every client shares the
  address of the proxy and one counter.
- **A throttled request costs nothing.** When the throttler rejects, the response is the `429` and no strategy runs: no key
  derivation, no validator call, no session store write, no request to the provider.
- **What is counted.** Every attempt, successful or not, counts: the counter does not know the outcome. A request without a
  readable username (missing, not a string, longer than the strategy's limit) is counted for the normalized client address,
  so it is still limited and still answered as the strategy answers it. The tracker value is `acct:` and the SHA-256 of the
  username, so usernames never reach the storage and the key has a fixed size. Existing and unknown accounts are counted
  alike, so the headers do not tell them apart.
- **Spelling.** The username is used as submitted. If your application treats `Ana@x.io` and `ana@x.io` as one account,
  wrap the tracker and lowercase first.
- **A per-account limit is a lockout tool for everybody.** Anyone can send a victim's username, so with a limit of 5 an
  attacker keeps that account blocked for the block duration (it defaults to the ttl). Choose limits that annoy an attacker
  more than the owner, keep the per-client throttler next to it, and decide the lockout policy (a captcha, a second
  factor, an e-mail) in your application: the throttler counts, it does not lock accounts.
- **OAuth2 routes.** Login start and callback have no username: limit them per client with the `default` throttler (the
  recipe's `throttle("default").limit(5)` on both routes). With `state_in_session()` every login start writes a session
  record, so an unlimited login route lets a client fill the store.

See the [authentication threat model](/security/authentication-threat-model#login-rate-limiting) for what each limit
covers and what is left.

## Storage

Counters live in memory, in `ThrottlerStorage`. It stores the `sha256` of each key and its hits grouped in at
most 64 buckets of `ttl / 64`: hits whose expiry falls in the same bucket are counted together and expire
together, so a key uses the same memory whatever its limit and however many requests it receives. Keys with no
live hit, an ended window and no block are deleted on every request.

A key that goes over its limit moves to a separate, compact table of blocked keys (its hits are dropped: they
are not used during the block). The two tables have their own caps, so a flood of blocked keys cannot displace
the counters of other clients:

- Counters: at most 100,000 keys by default (`with_max_entries`). The worst case is about 1.8 KB per key (64
  buckets of 24 bytes plus the key's fixed state), roughly 180 MB at the default. When the table is full the
  counters that become idle soonest are discarded, which resets them. Under an attack with more distinct keys
  than the cap, counters of clients that are not blocked can be reset, so size the cap for your traffic.
- Blocked keys: at most as many as `with_max_entries` unless you set `with_max_blocked`. A blocked key takes
  about 300 bytes, about 30 MB at 100,000 keys. When that table is full the block that started first is
  dropped, whatever its length and even if it has a long time left, which lets that key count again; freeing
  someone else's block therefore takes as many new blocks as the table holds. Set `with_max_blocked` to at
  least the number of blocks your application starts during its longest block duration: with 24-hour login
  blocks, count the blocks started in 24 hours.

Together the worst case at the defaults is about 200 MB (about 170 MB of counters plus about 30 MB of blocks). To change the caps:

```rust
use corvis::ThrottlerStorage;

ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)])
        .with_max_entries(500_000)
        .with_max_blocked(200_000),
)
```

Known difference from the original library: when a client sends hits closer together than `ttl / 64` (about
0.9 s for a 60 s window), they share a bucket and expire together, so `X-RateLimit-Remaining` and
`X-RateLimit-Reset` can differ by up to `ttl / 64`. Which requests are rejected, and `Retry-After`, are not
affected. With hits spaced further apart the values are identical.

Two throttlers with the same name share counters and block durations; a warning is logged when the storage is
built.

Being process memory, each instance of the application counts separately. There is no shared
storage (Redis or other) included.

## Limits from another provider

`ThrottlerModule::for_root_async(factory)` builds the limits from a factory that can resolve other providers, such as `ConfigService`. The factory returns a `Vec<ThrottlerOptions>` or a configured `ThrottlerStorage`; the module is global, like `for_root`:

```rust
ThrottlerModule::for_root_async(|ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let limit = config.get_or("THROTTLE_LIMIT", 10u32);
    Ok(vec![ThrottlerOptions::new(60_000, limit)])
})
```

If the factory fails, the application does not start (`Error while instantiating ThrottlerStorage: ...`).

## IPv6 clients

A host or home connection usually gets a whole /64 (2^64 addresses). If every address counted as a different
client, one client could get a fresh limit with each address it rotates through. `ThrottlerGuard` therefore keys
IPv6 clients by prefix, 64 bits by default, as the original throttler library does since 6.7.0 (`ipv6SubnetPrefix`). Clients in
the same /64 share their counters, like IPv4 clients behind one NAT.

Change the length with `ThrottlerStorage::with_ipv6_subnet_prefix`. It applies to every throttler and works with
`for_root_storage` and with `for_root_async` factories that return a storage:

```rust
use corvis::modules::throttler::ThrottlerStorage;

// Larger allocations: many providers delegate a /56 or /48 to one customer.
ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 10)]).with_ipv6_subnet_prefix(56),
)
```

- `128` keeps each IPv6 address apart (the way to opt out of grouping). IPv4-mapped addresses still count as IPv4 and
  `::1` stays alone. Values above 128 count as 128.
- `56` or `48` make rotation harder for endpoints such as login, but clients that share such a prefix also share the
  limit.
- A prefix shorter than 32 bits is accepted, but it puts a large part of the address space in one counter, so a
  warning is logged once when it is configured.

Shared hosting is the case where grouping hurts: if a provider gives each VM a slice of one /64 (often a /128 per VM),
all the VMs, and so all of your clients that live on them, share one counter. If your clients can sit on such
addresses, use `with_ipv6_subnet_prefix(128)`, or key the counters yourself with a guard that calls `record_hit`.

Where Corvis differs from the original library (security over parity), `normalize_ip` returns the client's IPv4
address for addresses that embed it, whatever the prefix:

- NAT64 `64:ff9b::/96`. A stateless translator (NAT46, SIIT-DC) shows every IPv4 client as `64:ff9b::a.b.c.d`; the
  /64 would put all of them in one counter. A deployment with its own translation prefix is not recognized: use
  `with_ipv6_subnet_prefix(128)` or a custom tracker there.
- 6to4 `2002::/16` (the IPv4 address is bits 16 to 47; one IPv4 address owns a whole /48 of them) and Teredo
  `2001::/32` (the client's IPv4 address is the last 32 bits, inverted).
- The deprecated IPv4-compatible form `::a.b.c.d`, except `::` and `::1`.

An address with a port (`203.0.113.5:54321`, `[2001:db8:1:2::1]:443`) or a bracketed IPv6 address
(`[2001:db8:1:2::1]`) counts as the address alone; some proxies write the port in `X-Forwarded-For`, and every
connection would otherwise get its own counter. The rest of the original's output is unchanged and checked against it.

`req.ip()`, `req.ips()` and `#[ip]` still return the full address; only the throttler key changes. A guard of your
own that calls `ThrottlerStorage::record_hit` can build the same client key with
`corvis::modules::throttler::normalize_ip(&req.ip(), prefix)`, a port of the original's `normalizeIp`. Anything that is
not an IP address, with or without a port (see [Behind a proxy](#behind-a-proxy)) is returned unchanged, and a zone identifier (`%eth0`) is
dropped from valid addresses.

## Behind a proxy

The key uses `req.ip()`, which by default is the socket address. Behind a reverse proxy all
requests would come from the proxy's IP and share a counter. Configure `trust proxy` so that
`req.ip()` reads `X-Forwarded-For`:

```rust
app.set_trust_proxy(1); // one proxy in front
```

The setting also applies to gateway messages: a `ThrottlerGuard` on a gateway keys by the handshake
request's `ip()`, which resolves `X-Forwarded-For` the same way (see [Gateways](/websockets/gateways)).

Use `true` (trust all hops) only if every request goes through a proxy that
rewrites `X-Forwarded-For`: otherwise, a client can choose its IP and bypass the limit. See
[HTTP adapters](/http/performance).

The counter uses the normalized form of the client entry that `trust proxy` resolves. Entries that are neither IP
addresses nor an address with a port are kept as they are, so a client that can choose its `X-Forwarded-For` entry still chooses its counter.

## Response headers

Like the original library, `ThrottlerGuard` tells the client where it stands. For every throttler that counts a request and lets it through it sends:

| Header | Value |
|---|---|
| `X-RateLimit-Limit` | The effective limit (after the route's override) |
| `X-RateLimit-Remaining` | The limit minus the hits counted in the last `ttl`, never below 0 |
| `X-RateLimit-Reset` | Seconds until the key's reporting window ends, rounded up |

When a throttler blocks the request, the `429` carries `Retry-After` (seconds until the block ends, at least 1) and no `X-RateLimit-*` header for that throttler. Headers of throttlers checked earlier stay on the `429`. Skipped throttlers get none of them.

For a throttler not named `default` the header names end in `-<name>`: with the `short` and `long` throttlers above a response carries `X-RateLimit-Limit-short`, `X-RateLimit-Remaining-long`, `Retry-After-long` and so on. The throttler name becomes part of a header name, so use a plain token (letters, digits and `-`) that says nothing internal; a name that is not a valid header name makes its headers be dropped.

The headers are on by default and tell every client each limit, its window and the throttler names. To hide them, keep counting and blocking as they are, and turn them off on the storage:

```rust
ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)]).with_headers(false),
)
```

Browsers only let a page read these headers if the CORS configuration exposes them (`Access-Control-Expose-Headers` in `CorsOptions`). `Retry-After` and the `X-RateLimit-*` values follow the original library for the same requests. A WebSocket message is counted but gets no headers. Other guards and interceptors can set headers with `ctx.switch_to_http().set_response_header` (see [Guards](/guards#setting-response-headers)).

## Limitations

- A global `ThrottlerGuard` also counts WebSocket gateway messages, per handshake IP and handler, so a long-lived connection can exhaust its limit. `app.apply_global_enhancers_to_gateways(false)` leaves gateways out. Plain `corvis::ws` listeners are never counted.
