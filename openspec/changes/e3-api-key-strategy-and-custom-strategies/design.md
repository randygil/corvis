# Design

## Context

This change builds on #136 (`openspec/changes/e3-strategy-core-and-guard-responses/`), which adds `corvis::auth` with
`Strategy` (`authenticate(&ExecutionContext, &AuthenticateOptions) -> StrategyOutcome`), `AuthenticatedUser`,
`Provider::strategy::<S>()`, the registry, `AuthGuard` and the fixture package `crates/corvis/tests/fixtures/auth/`.
#136 is being implemented in another worktree while this is drafted: its API is described as designed (its
`design.md` decisions 2-5 and 11, Review notes 1-5), not as shipped. The validator-plus-adapter shape follows #42
(`JwtValidator` / `JwtStrategy<V>`) and #41 (`LocalValidator` / `LocalStrategy<V>`), both approved and not yet
implemented. Line numbers were checked on `origin/dev` at `6f58ee1`.

### Corvis today

- There is no `crates/corvis/src/auth/` on `dev` yet, and no API key code anywhere in `crates/corvis/src`.
- `Request::header(name)` returns `None` for a value that is not visible ASCII
  (`crates/corvis/src/http/request.rs:135-137`, `to_str()`); `Request::cookies` decodes header bytes as latin1
  (`request.rs:223-227`, `body_parser::latin1`), which is how Node exposes header strings. #42 reads the
  `Authorization` header through latin1 for that reason (its design decision 3).
- `Request::query()` is the parsed query (`request.rs:161-163`, a JSON map: a repeated parameter is an array, a
  bracketed one an object); `Request::query_param(name)` returns the value only when it is a string (164-166).
- `Request::url()` is the path with the query string (`request.rs:122-124`). It is used by
  `LoggingInterceptor` to build the log line `GET /cats?x=1 200 - 3ms` (`crates/corvis/src/common/interceptor.rs:88-91`;
  the comment above it already avoids logging the handshake URL of a gateway "whose URL may carry a token in its
  query"), by the `404` of an unmatched route (`crates/corvis/src/router/mod.rs:659` and `668`) and by the cache key
  of `CacheInterceptor` (`crates/corvis/src/modules/cache.rs:207`). Interceptors run after guards
  (`router/mod.rs:758-782`), so `LoggingInterceptor` logs only requests that passed the guards.
- `Request::insert` / `get` store typed extensions shared by all clones (`request.rs:233-242`).
- Dependencies: `sha2 = "0.10"` is a dependency of `corvis` (`crates/corvis/Cargo.toml:45`); `subtle` 2.6.1 is in
  `Cargo.lock` as a transitive dependency (of `digest` and `rustls`) but is not a direct dependency of `corvis`.
  Features today: `jwt`, `websockets`, `tls`, `health-tls`, `compression` (`Cargo.toml:56-62`); the CI checks each one
  alone (`.github/workflows/ci.yml:69-74`).
- `examples/cats-app` has a JWT guard and no machine-client route (`examples/cats-app/src/auth/`).

### The original

Facts from the spike (its fact 8 and the versions list; the implementer re-reads the package sources when writing the
generators and corrects this section if they differ):

- `passport-headerapikey` 1.2.2 (`src/Strategy.ts`): strategy name `headerapikey`; options `{ header, prefix }`,
  header `X-Api-Key` by default, looked up lowercased; missing header → `fail(BadRequestError('Missing API Key'))`;
  the prefix is removed with an unescaped `RegExp('^' + prefix)` and a value that does not match it fails; the verify
  callback receives the key; a falsy result → `fail`. It reads headers only.
- `passport-custom` 1.2.1: a strategy is a callback `(req, done)`; `done(null, user, info)` is success, a falsy user
  is `fail`, `done(err)` is `error`. It cannot reach redirect or pass.
- Through `AuthGuard` of `@nestjs/passport` 12.0.1 every `fail` is `401 {"message":"Unauthorized","statusCode":401}`
  whatever the challenge or status (spike fact 1), and an `error` passes through unchanged.
- Node joins repeated values of a header that is not in its single-value list with `", "`, so two `X-Api-Key` headers
  reach the strategy as one string `"k1, k2"`.

## Goals / Non-Goals

**Goals**

- `AuthGuard::new("headerapikey")` with a validator class, with the header behavior of `passport-headerapikey`,
  verified by a recorded fixture.
- Safe by default: keys read only where configured, never from a URL unless asked, compared in constant time,
  bounded, and absent from every log line and error the framework produces.
- A documented and tested way to write a strategy, equivalent to `passport-custom`.

**Non-Goals**

- Key management (generation, storage, rotation, scopes), request signing, slow hashing of keys.
- New core API for custom strategies.

## Decisions

### 1. Feature `auth-api-key` with `subtle`

`auth-api-key = ["dep:subtle"]`, off by default, module `corvis::auth::api_key`. The convention of the epic: the core
has no feature and no new dependency (#136 decision 1), and each strategy with extra dependencies gets one feature
(`auth-local` in #41). `subtle` is the reference crate for constant-time comparison and is already in the lockfile, so
the feature adds no new crate to the build of an app that already uses `corvis-orm` or TLS.

Alternatives: a hand-written XOR loop in the core with no feature (a compiler may turn it into an early exit;
rejected); `subtle` as a required dependency of `corvis` (breaks the "core adds no dependency" decision for one
comparison); the spike's `auth` feature (not created, #136 decision 1). See Open question 1.

### 2. Validator trait, options and the adapter

```rust
#[async_trait]
pub trait ApiKeyValidator: Injectable {
    const NAME: &'static str = "headerapikey";
    type User: Serialize + Clone + Send + Sync + 'static;
    fn options(&self) -> ApiKeyStrategyOptions { ApiKeyStrategyOptions::default() }
    async fn validate(&self, req: &Request, key: &ApiKey) -> Result<Option<Self::User>, HttpException>;
    fn info(&self, _user: &Self::User) -> Option<Value> { None }
}

ApiKeyStrategyOptions::default()                       // one source: header "X-Api-Key", no prefix
ApiKeyStrategyOptions::new()                           // no source: must add at least one
    .source(ApiKeySource::header("X-Api-Key"))
    .source(ApiKeySource::header("Authorization").prefix("Api-Key "))
    .source(ApiKeySource::bearer())
    .source(ApiKeySource::query("api_key"))            // startup warning
    .max_key_bytes(512)
```

`ApiKeyStrategy<V>` implements `Injectable` (resolves `V` through DI, calls `options()` once and validates it; an
invalid configuration is a `DiError` naming the strategy, so the application does not start, as `JwtStrategy<V>` in
#42) and `Strategy` (name `V::NAME`). Outcomes: `Ok(Some(user))` → `Success` with `info`; `Ok(None)` → `Fail` with no
challenge; `Err(e)` → `Error(e)`. `passReqToCallback` does not exist: the request is always passed.

The name stays `"headerapikey"` (spike decision 2 and the `auth/strategies` wording) so that
`AuthGuard('headerapikey')` migrates one to one, although Corvis can also read other places.

### 3. Sources

| Source | Reads | Reference |
|---|---|---|
| `header(name)` | every value of the header, latin1-decoded, joined with `", "`; name compared case-insensitively | `passport-headerapikey` 1.2.2 |
| `header(name).prefix(p)` | the same, and the value must start with `p` (byte comparison, case-sensitive), which is removed | the same, with the literal-prefix divergence (spike decision 10) |
| `bearer()` | the first `Authorization` value, parsed with the rule of #42's `from_auth_header_as_bearer_token` (`(\S+)\s+(\S+)`, scheme `bearer` ignoring ASCII case) | `passport-custom` 1.2.1 with `passport-jwt`'s header parser |
| `query(name)` | `req.query()[name]` when it is a string | `passport-custom` 1.2.1 reading `req.query[name]` |

A source that finds nothing (absent header, wrong prefix, empty string, a query value that is an array or an object)
yields no key. `bearer()` reuses the header parser #42 adds; if #42 keeps it private to `auth::jwt` (which is behind
the `jwt` feature), the implementer moves the parser to a `pub(crate)` function of `corvis::auth` that both use.

Sources are evaluated in the configured order, all of them:

- no source yields a key → `Fail { challenge: {"message":"Missing API Key"} }` (the original's message);
- every source that yields a key yields the same bytes → that key;
- two sources yield different keys → `Fail { challenge: {"message":"Conflicting API keys"} }`, validator not called.

The conflict rule is a divergence with no original (it has one source). It closes the case where a proxy or a WAF
authenticates or rate-limits by one location and the app trusts another. Alternative: first source wins, as
`fromExtractors` of `passport-jwt`: simpler, but it accepts exactly that case. The comparison of the candidates uses
the constant-time helper (they are secrets).

There is no default for query or bearer: `ApiKeyStrategyOptions::default()` reads only `X-Api-Key`.

### 4. `ApiKey` and the comparison helpers

```rust
pub struct ApiKey(/* private */);
impl ApiKey {
    pub fn expose(&self) -> &str;                         // the key itself: do not log it
    pub fn ct_eq(&self, expected: impl AsRef<[u8]>) -> bool;
    pub fn sha256(&self) -> [u8; 32];
    pub fn sha256_hex(&self) -> String;                   // lowercase hex, to look a key up by its digest
}
impl fmt::Debug for ApiKey { /* "ApiKey(<redacted>)" */ }   // no Display, no Serialize, no PartialEq

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool;

pub struct ExpectedApiKey(/* SHA-256 digest only */);
impl ExpectedApiKey {
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self, ApiKeyConfigError>;   // < 32 bytes is an error
    pub fn from_sha256_hex(digest: &str) -> Result<Self, ApiKeyConfigError>;   // 64 hex characters
    pub fn matches(&self, key: &ApiKey) -> bool;
}

pub struct ApiKeySet<T>(/* Vec<(ExpectedApiKey, T)> */);
impl<T> ApiKeySet<T> {
    pub fn new() -> Self;
    pub fn insert(&mut self, key: ExpectedApiKey, value: T);
    pub fn lookup(&self, key: &ApiKey) -> Option<&T>;     // compares with every entry, no early exit
}
```

`constant_time_eq(a, b)` hashes both inputs with SHA-256 and compares the two digests with
`subtle::ConstantTimeEq`. A plain `ct_eq` on slices of different lengths answers at once and so reveals the length of
the stored key; comparing fixed-size digests removes that, at the cost of one hash of an input the attacker already
knows the length of. `ApiKey::ct_eq`, `ExpectedApiKey::matches` and `ApiKeySet::lookup` are built on the same
comparison. `ApiKeySet::lookup` folds over all entries with `subtle::Choice` (no `break`), so its time depends on the
number of configured keys, not on which one matched; it is meant for a handful of keys from configuration.

Many keys in a database: store `sha256_hex` of each key in an indexed column and look the presented key's digest up.
The lookup is not constant-time, but what it can leak is a prefix of the digest of a 32-byte random secret, which does
not help to find a key. The docs give both patterns and say when each applies.

`ExpectedApiKey` keeps only the digest, so a configured key does not stay in memory in clear after startup, and its
`Debug` prints `ExpectedApiKey(<redacted>)`. The 32-byte minimum follows the rule settled for the epic (secrets
shorter than 32 bytes are a startup error, #42 Review note 2): the framework can only enforce it where it sees the
secret, which is here. `ApiKeyConfigError` messages never contain the secret (`"API key secret must be at least 32
bytes (got 12)"`).

Alternatives: `PartialEq` on `ApiKey` implemented in constant time (an `==` in user code would look like the unsafe
comparison and reviewers could not tell; rejected: no `PartialEq`); returning `String` to the validator (the redacted
`Debug` is lost the moment it is a `String`; `expose()` makes the dangerous step visible).

### 5. No key material in logs or errors

- The strategy logs nothing per request. Failure challenges are the fixed strings `Missing API Key`,
  `Conflicting API keys`, `API key too long` and `Invalid API key characters`; none includes request data, and
  through `AuthGuard` the client sees the plain `401` anyway.
- Startup: the query-source warning names the strategy and the parameter (`ApiKeyStrategy "headerapikey" accepts keys
  in the query parameter "api_key": URLs are written to proxy and access logs`), never a key.
- `ApiKey`, `ExpectedApiKey` and `ApiKeySet` have redacted `Debug`; a `#[derive(Debug)]` on a struct holding them is
  safe.
- Query keys and request logs: `Request::redact_query_param(&self, name: &str)` records a parameter name in a request
  extension (shared by clones, at most 16 names per request), and `Request::redacted_url(&self) -> String` returns
  `url()` with the value of every recorded parameter replaced by `REDACTED` (raw query split on `&`, name compared
  after percent-decoding, every occurrence replaced, bracketed forms `name[...]` included). With nothing recorded it
  returns `url()` unchanged. The strategy calls `redact_query_param` for each configured query source before it
  reads anything, so the mark is there on success and on failure. `LoggingInterceptor` prints `redacted_url()`. Both
  methods are public so that a custom strategy and an app's own logging middleware can use them.

A middleware that logs `req.url()` before the guards run still logs the key, as does any proxy in front: that is why
the query source needs an explicit opt-in and a warning (spike Review note 6). Alternative: a global list of redacted
parameters on the application (`app.redact_query_params([..])`) applied to `url()` itself: changes what `url()`
returns to handlers and to the `404` body (parity with `req.originalUrl`); rejected for this change, noted as a
possible new issue.

### 6. Limits and their defaults

| Limit | Default | Bound | Original |
|---|---|---|---|
| `max_key_bytes` | 512 | 1 to 4096, else startup error | none (Node's header size limit) |
| Key characters | bytes `0x00-0x1F` and `0x7F` refused | fixed | none |
| Query source | not configured | startup warning when configured | not available |
| Source names, prefix | must not be empty; a header name must be a valid header name | startup error | none |
| Sources | at least one, at most 8 | startup error | one |
| `ExpectedApiKey` secret | at least 32 bytes | error at construction | none |
| Redacted names per request | 16 | fixed | none |

The length and character checks run on each candidate before the conflict comparison and the validator, so an
oversized key costs one length comparison. Keys are bytes of the header as latin1 or of the decoded query value;
control characters cannot arrive in a header (the parser refuses them) but can arrive percent-encoded in a query.

### 7. Custom strategies

No new API. The pattern the guide documents and the fixture checks:

```rust
#[injectable]
pub struct DeviceStrategy { devices: DevicesService }

#[async_trait]
impl Strategy for DeviceStrategy {
    fn name(&self) -> &str { "device" }
    async fn authenticate(&self, ctx: &ExecutionContext, _opts: &AuthenticateOptions) -> StrategyOutcome {
        let req = ctx.request();
        let Some(id) = req.header("x-device-id") else {
            return StrategyOutcome::Fail { challenge: None, status: None };
        };
        match self.devices.find(id).await {
            Ok(Some(device)) => match AuthenticatedUser::new(device) {
                Ok(user) => StrategyOutcome::Success { user, info: None },
                Err(e) => StrategyOutcome::Error(e),
            },
            Ok(None) => StrategyOutcome::Fail { challenge: None, status: None },
            Err(e) => StrategyOutcome::Error(e),
        }
    }
}
// providers = [DevicesService, Provider::strategy::<DeviceStrategy>()]
```

Mapping for a migration: `done(null, user, info)` → `Success`; `done(null, false)` → `Fail`; `done(err)` → `Error`;
`this.redirect(url)` and `this.pass()` of a `passport-strategy` subclass → `Redirect` and `Pass` (already recorded by
`strategies.mjs` of #136). This change records the three `passport-custom` outcomes with a strategy that injects a
service, and tests replacing a strategy in a testing module (the `Arc<dyn Strategy>` cast of `Provider::strategy`,
#136 decision 3). If #136 ships a different override mechanism, the test and the guide follow it.

The guide's security checklist: fail closed (`Fail`, never `Pass`, when credentials are absent or wrong; `Pass` lets
the request through without a user); compare secrets with `constant_time_eq`; never put request data in an
`Error` message or a log line; bound what is read from the request; do not read credentials from the URL without
calling `redact_query_param`; a `Redirect` target must not come from request data.

### 8. Fixtures

Added to `crates/corvis/tests/fixtures/auth/` (package of #136): `passport-headerapikey` 1.2.2 and `passport-custom`
1.2.1, exact versions.

- `api_key.mjs` → `api_key.json`. Routes: default header; custom header name; header with prefix `Api-Key `; header
  with prefix `Key+` (the divergence case); a `passport-custom` strategy reading `req.query.api_key`; a
  `passport-custom` strategy reading the bearer token with `passport-jwt`'s header parser; a chain
  `['jwt-like', 'headerapikey']`. Cases: valid key; wrong key; missing header; empty header; header name in another
  case; two header values; prefix present, absent and in another case; the `Key+` prefix against `Key+abc` and
  `Keyyabc` (marked divergence: the original accepts the second); query key present, absent, repeated
  (`?api_key=a&api_key=b`) and bracketed; bearer key with one and several spaces; validator returning a user with
  info, `null`, and throwing `ForbiddenException`; non-ASCII byte in the header. Marked divergences, asserted from the
  spec: oversized key, control character in a query key, conflicting keys in two sources.
- `custom.mjs` → `custom.json`: a `passport-custom` strategy with an injected service, driven to success (with info),
  fail and error (`ForbiddenException`, plain `Error`), alone and second in a chain.

Fixture keys are fixed test strings of at least 32 bytes, obviously fake (`test-key-000000000000000000000000000001`).
`crates/corvis/tests/auth_api_key.rs` and `auth_custom.rs` declare the same routes and replay every case (status, the
headers listed in the case, body); auth info is not compared (the original's is a serialized `Error`, as in #42
decision 7).

### 9. Example

`examples/cats-app`: enables `auth-api-key`; `ServiceKeyAuth` (`ApiKeyValidator`, one `ExpectedApiKey` built from
the `SERVICE_API_KEY` configuration value with a clear startup error when it is shorter than 32 bytes); one read-only
route for machine clients guarded by `AuthGuard::chain(["jwt", "headerapikey"])`; a test that a key works, a wrong
key gets the `401`, and the application does not start with a short key. Its `posture.rs` test keeps passing.

## Risks / Trade-offs

- **`LoggingInterceptor` output changes** only for requests that carry a marked parameter; every other line is
  byte-identical. Covered by a regression test and the corvis-parity check.
- **The conflict rule can surprise** a client that sends an old key in one place and a new one in another during a
  rotation. The failure reason is in the auth info and the docs say to send one.
- **`ct_eq` hashes on every comparison.** Two SHA-256 computations over at most 4096 bytes per request; negligible
  next to the validator's own lookup.
- **The 32-byte minimum only covers keys given to `ExpectedApiKey`.** Keys an app stores itself are its own; the docs
  say to generate at least 32 random bytes.
- **Redaction is best effort for other loggers**: a middleware, a proxy or a browser history still sees a key in the
  URL. Documented next to the option.
- **Latin1 header handling** differs from `Request::header`; the property test covers random bytes.
- **Depends on unreleased designs** (#136, #42): names may move; the first task re-checks them.

## Open questions

1. Feature or core for the constant-time helpers? The design puts everything behind `auth-api-key` (convention: the
   core adds no dependency). The maintainer may prefer `constant_time_eq` in the always-compiled core (with `subtle`
   as a required dependency) so that custom strategies get it without a feature.
2. Conflicting keys: fail (this design) or first configured source wins (simpler, the `fromExtractors` behavior)?
3. Should the default name be `"headerapikey"` (migration, spike wording) or `"api-key"` (clearer now that bearer and
   query are sources)? The design keeps `"headerapikey"`; `const NAME` overrides it.
4. `max_key_bytes` default 512: confirm (opaque keys are usually 32 to 128 characters; signed keys can be longer).
5. Is `Request::redacted_url()` as public API acceptable in this change, or should request-log redaction be its own
   issue with a general setting?

## Review notes

These settle the open questions and override the design where they differ.

1. The constant-time helpers stay behind `auth-api-key`; the core gets no new mandatory dependency.
2. Conflicting keys from two sources fail the strategy without calling the validator, as designed.
3. The default strategy name is `"api-key"`, not `"headerapikey"`. This overrides the design; the docs mention the
   original name once, for people who port a configuration.
4. `max_key_bytes` 512 is confirmed.
5. `Request::redact_query_param` and `redacted_url()` are part of this change, and `LoggingInterceptor` uses the
   redacted URL. The corvis-parity check is mandatory because of it.
6. Implemented after #136 lands: the first task compares this design with the code #136 merged (the typed user now
   lives in a private slot that `User<T>` reads, `respond_with` only works in the guard phase, redirects are 3xx
   only) and records the adjustments in Implementation notes.
7. The security review is mandatory before the merge.

## Implementation notes

Written after comparing this design with what the auth core (#136), the JWT strategy (#42) and the local strategy (#41)
shipped, including the "Security review" sections of the archived designs.

### Adjustments to the merged code

1. **API as merged matches the design** except as listed here: `Strategy::name` is a method (the validator adapter
   returns `V::NAME`), `AuthenticatedUser::new` returns a `Result`, the typed user lives in a private slot read by
   `User<T>`, `Provider::strategy::<S>()` needs `S: Injectable + Strategy`, and `ApiKeyStrategy<V>` implements
   `Injectable` by hand like `JwtStrategy<V>` and `LocalStrategy<V>` (`get_or_construct::<V>()`, options validated in
   `construct`, a `DiError::Factory` that names the strategy and the validator type).
2. **Warnings run in `on_module_init`** (as the JWT strategy), so a logger installed after `create` sees them.
3. **The bearer parser is not shared with `auth::jwt`.** It is private there and behind the `jwt` feature, and the API
   key strategy needs a bounded variant (it must not scan an unbounded header), so `api_key` has its own byte-level
   parser with the same grammar (`(\S+)\s+(\S+)`, JavaScript whitespace in latin1, first `Authorization` value only).
4. **No override mechanism existed.** `Provider::strategy` stores a tag that already accepts `Arc<dyn Strategy>`, but a
   provider override dropped the tag (the strategy left the registry) and the token of a strategy provider is a named
   one, so `override_provider::<S>()` never matched. `Container::add_provider` now keeps `strategy` like it keeps
   `enhancer`, and `TestingModuleBuilder::override_strategy::<S>(stub)` was added.
5. **Name `"api-key"`** (Review note 3), so the spec scenarios and the guide say `api-key`; the docs mention
   `headerapikey` once, for configurations that are ported.
6. **Facts about the original corrected after reading the sources** (`passport-headerapikey` 1.2.2 `Strategy.js`): the
   constructor is `Strategy(header, passReqToCallback, verify)`; the prefix check is `_.startsWith` (literal), and only
   the removal uses the unescaped `RegExp('^' + prefix)`. So the divergence is `Key+<key>` (the original's `verify`
   receives `+<key>`, Corvis's the key), while `Keyyabc` fails in both; the design said the original accepts `Keyyabc`.
   A prefix mismatch is `fail(BadRequestError('Invalid API Key prefix...'))` and a missing header
   `fail(BadRequestError('Missing API Key'))`; through the original guard both are the plain 401.
   The `PassportStrategy` mixin of the original integration passes `done` as the last callback argument, which is
   incompatible with `passReqToCallback: true` of this package, so the generator registers the strategies with
   `passport.use(name, strategy)` directly, as `strategies.mjs` does.

### Decisions beyond the design (hardening, from the review brief)

7. **A present but unusable source fails the whole attempt** (design: "yields no key"). Each source reads as `Absent`
   (no value, empty, other scheme, value without the prefix), `Key` or `Invalid` (too long, control character, repeated
   or, with the extended query parser, bracketed query parameter: reason `Ambiguous API key`, a fifth fixed string).
   Any `Invalid` fails with its reason before the other sources are consulted, so one place cannot be made to look
   empty so that another decides. A wrong prefix stays `Absent` so that `header("Authorization").prefix(..)` and
   `bearer()` can read the same header. The candidates that remain are compared without an early exit.
8. **Bounds before work.** The joined header value is never held past `prefix + max_key_bytes` bytes (the loop over
   repeated values stops at the first one that crosses it), the bearer scan reads at most `max_key_bytes + 128` bytes
   of leading space, scheme and separator and at most `max_key_bytes + 1` of key, the query value is checked by length
   before its characters, and only then is the key hashed. Header prefixes are printable ASCII (1 to 64 bytes) so that
   the raw and the decoded length agree; a `Cookie` header source is refused at startup.
9. **Validator failures** (design: `Err(e)` passes through). A 4xx passes unchanged (the recorded `ForbiddenException`
   case needs it); a status of 500 or above, or below 400, becomes the plain exception of that status (the generic
   internal-error body for 500), so a database error that quotes the key or differs per key never reaches the client.
   New option `validator_timeout` (default 5 s, 1 ms to 300 s): a validator that is slower is a plain 503. Panics are
   caught by the core (note 10). The warnings are rate limited (one per reason per minute) with the status only, never
   the message.
10. **The core contains a panic in any strategy.** `Authenticator::authenticate` catches a panic inside
    `Strategy::authenticate` and returns an internal error (500, fixed body), which ends the chain and is not a failure,
    so an optional guard cannot let the request through. The payload is never read; a warning per strategy name per
    minute (a new `auth::warn_limit::WarnLimiter`, a copy of the one in `auth/local/hasher.rs` that keys by string and
    is bounded to 64 reasons; the hasher keeps its own). A strategy name that is empty, over 128 bytes or holds a control
    character stops startup. The names of guards are checked through `CanActivate::inner_guards` since #42; a test
    covers a custom strategy behind a wrapper.
11. **Lookup helpers lead to hashed storage.** Added beyond the design: `ApiKey::id(delimiter)` (the public id of
    `<id><delimiter><secret>`, validated so it is safe to log and to use as a database key),
    `ExpectedApiKey::matches_or_dummy(Option<&ExpectedApiKey>, &ApiKey)` (an unknown id costs the same comparison),
    and `ApiKeySet::lookup` picks the entry with `subtle` selection instead of a branch. `ApiKey` has no `Clone`.
    `ApiKey::new` is public so that validators can be tested. `ExpectedApiKey::new` also refuses secrets over 4096
    bytes. The docs state how to store keys (digest plus public id) and how to generate them.
12. **`Request::redact_query_param` keeps the query parser.** `Request` now records the `query parser` setting it was
    built with, and `redacted_url` splits the raw query on `&` and parses each pair with that parser, so a pair is hidden
    exactly when the application files it under a marked name (percent escapes in any case, `[name]` and `name[x]` under
    the extended parser). Past 16 names, or a name over 256 bytes, every value is hidden instead of leaving a marked
    one visible (the design silently dropped the extra name). A bracketed or oddly encoded name is not echoed.
13. **Not done, as the design said:** no body or cookie source, no zeroizing, no redaction of the `404` body or the cache
    key.

### Fixtures and tests

14. **`api_key.json`** has 55 cases over 8 routes (the design listed fewer; the header-only cases were added for the
    ignored key places: query, `Authorization`, cookie). Five are divergences, each asserted from the specification: an
    oversized key in a header and in the bearer source, a control character in a query key, conflicting keys in two
    places (the original's `two-places` strategy lets the header win) and the literal prefix. The non-ASCII header byte
    case is a unit test (`header_bytes_are_read_as_latin1`) because `TestRequest` cannot send raw bytes. A header
    repeated twice is recorded as the joined value `a, b`, as Node's `fetch` sends it; real repeated headers are unit
    tests (`repeated_values_are_joined_like_node_does`, a 5000-value case for the bound). `custom.json` has 15 cases.
    Both generators give identical output on two runs.
15. **Test files:** `tests/auth_api_key.rs` (replay and the spec scenarios), `tests/auth_api_key_options.rs` (startup
    checks, limits, configured keys, uniform answers, validator failures, timeouts and panics), `tests/auth_api_key_log.rs`
    (its own binary because the logger is process-wide: startup warnings, the redacted request log, no key in any log
    line, rate-limited warnings), `tests/auth_custom.rs` (no feature gate; builds and passes with
    `--no-default-features`). Unit tests live next to `key.rs`, `options.rs` and `strategy.rs`; the fuzz properties are
    `fuzz_random_bytes_never_panic_and_stay_within_the_limits` (needs the feature; CI and nightly fuzz jobs got a step
    for it) and `redacted_urls_never_hold_a_marked_value` (always compiled).
16. **Example.** `cats-app` enables `auth-api-key`; `GET /api/v1/cats/export` is `#[public]` (it only keeps the global
    JWT guard out of the way) and guarded by `corvis::auth::AuthGuard::chain(["jwt", "api-key"])`. `SERVICE_API_KEY` is
    optional (without it no key works); a value under 32 bytes stops startup with a message that does not repeat it.
    Tests: `service_key.rs` and `startup_short_service_key.rs`.
17. **Left to the maintainer's session:** the corvis-parity check (task 7.6), the security review (7.8), archiving and the
    CI run of the pull request (task 1.2's last verification).

## Security review (round 1)

The first security review found six items. The maintainer decided each one as below; the notes continue the numbering
of "Implementation notes".

18. **An empty user never authenticates (Medium).** `ApiKeyStrategy` turned every `Ok(Some(user))` into a `Success`, and
    the core accepted a `Success` whose user serialized to `false`, `null`, `0` or `""`: a validator written as
    `type User = bool; Ok(Some(expected.matches(key)))` or `type User = Option<Service>; Ok(Some(repo.find(id)))`
    authenticated every request. The originals fail closed twice (`if (!user) return self.fail()` in the strategies,
    and `handleRequest` throws a 401 for `!user`). The fix is in the core, so it covers JWT, local, API key and custom
    strategies: `AuthenticatedUser::is_empty()` is true for JSON `null`, `false`, the number `0` (integer, float,
    `-0`) and `""` (not for `"0"`, `{}`, `[]`, which are truthy in JavaScript); `Authenticator::authenticate` turns such
    a `Success` into a `Fail` of that strategy (the chain goes on; the challenge, hence the auth info, is
    `{"message":"The strategy returned an empty user"}`) and logs a warning through a second `WarnLimiter` (one per
    strategy name per minute, with the suppressed count) that says the validator should return `None`. Defense in depth:
    `AuthenticatedUser::install` (so `AuthResult::store_user`) refuses an empty user and `AuthGuard` treats a
    `Success` that does not store as a failure, so even an `AuthResult` built by hand cannot reach the handler. Other
    paths: the WebSocket path never runs a strategy (the guard denies), an optional guard clears the user on a failure,
    and `Request::set_user` is the application's own guard writing its own user, which no `AuthGuard` reads. A user
    that is legitimately `0` or `""` is wrapped in a struct; the docs say so in a `danger` block. The `custom` fixture
    gained nine cases (`done(null, false | 0 | '' | null | undefined | NaN)`, a real user as the control, and the
    `falsy` strategy before `device` in a chain): 24 cases in all, same output on two runs.
19. **Query keys are redacted application-wide (Low).** The mark was set inside `authenticate`, so a chain whose earlier
    strategy succeeded, or a route without the query strategy, logged `?api_key=<key>`. `Strategy` has a new method
    `credential_query_params() -> Vec<String>` (empty by default). After the registry is built, `init()` collects the
    names of every strategy into an `AppRedaction` stored in `BodyParserOptions` (so every `Request` made by the server,
    the gateways and the testing client carries an `Arc` to it, `None` when nothing is registered) and
    `redacted_url()` applies it in addition to the per-request marks. The API key strategy, the JWT strategy
    (`from_url_query_parameter`) and the local strategy (`allow_query_credentials`) register their names; the per
    request mark in `authenticate` stays for a strategy used without an application. There is no global state: two
    applications in one process do not share names. With nothing registered or marked, `redacted_url()` is `url()` with
    no parsing. The docs sentence now says exactly what is hidden (the log line and `redacted_url()`, on every request)
    and what is not (reverse-proxy and CDN logs, browser history, `Referer`, anything that prints `url()`), and
    recommends against query keys.
20. **`redacted_url()` is cheap (Low).** It used to call the query parser once per pair (14 to 27 times the cost of the
    request for a query full of pairs, reachable by an unauthenticated client). Now it scans the pairs without
    allocating: the NAME is percent-decoded on the fly and compared byte by byte with the registered names, the value
    is never looked at, and the output is one `String`. The application's parser is called only for a pair whose name
    holds `[`, `]`, `%5B` or `%5D` and only when the extended parser is active (the simple parser keeps brackets
    literal, so the comparison is exact). Above 1000 pairs, or when more than 16 request names (64 application names) or
    a name over 256 bytes were registered, every value is hidden without parsing, and a pair whose name has brackets is
    printed as `param=REDACTED` rather than echoed. `LoggingInterceptor` does not build the line when the `log` level is
    off (`logger::level_enabled`). Tests: the output equals the previous implementation's (kept in the test module as
    the reference) over 3000 random cases and three parsers, and `tests/redact_allocations.rs` bounds the work with a
    counting allocator (at most 20 allocations for 20 000 pairs, and no growth with the number of pairs).
21. **The prefix is checked first (Low).** For a prefixed header source, a value that does not start with the prefix is
    `Absent` whatever its length or content; `joined_starts_with` compares at most `prefix.len()` bytes of the joined
    value (it follows the same `", "` joining) and copies nothing. A valid `X-Api-Key` next to
    `Authorization: Bearer <600 bytes>` or a value with a tab is no longer failed by that header.
22. **Docs.** The panic paragraph says the default panic hook of the standard library prints the message to stderr and
    that a strategy must not put credentials in panic messages or errors; the checklist warns that
    `constant_time_eq(b"", b"")` is `true`, so a strategy refuses an empty or unset expected secret before comparing.
23. **The panic line is rate limited (Info).** The warning of the authenticator was already limited per strategy, but
    the 500 it returns is an internal error and the default filter wrote an `ExceptionsHandler` line for each request.
    `HttpException::already_logged()` (crate-private) marks an exception whose raiser logged it, and
    `BaseExceptionFilter` skips the line for it. The panic payload is still never read.

## Security review (round 2)

The second pass could not get around any fix of round 1 (a differential fuzz of 600,000 query strings against what
the strategies really read found no value left in `redacted_url()`), and judged the change ready to merge. What it
noted, and what was done:

24. A query parameter name that is not valid percent-encoding (`k%FF`) reads differently in the extended parser and in
    the log, so its value reached the validator and was still printed. The names the strategies declare are now
    checked when the application starts: a name with `%` stops the start, for every strategy. Test:
    `a_query_source_whose_name_holds_a_percent_sign_stops_the_start`.
25. `Debug` of `Request` printed the full URI, so a `{:?}` in an error report or a tracing span printed the key. It
    prints `redacted_url()`. Test: `the_debug_form_of_a_request_hides_the_key_of_a_query_source`.
26. A strategy that reads the query in its own code (`ExtractJwt::custom`, a custom strategy that does not override
    `credential_query_params()`) registers no name. Documented, with the two ways to register one. A declarative
    option for the JWT strategy is a possible follow-up.
27. Accepted, not changed: `redacted_url()` is still several times the plain request for two shapes that only matter
    with `LoggingInterceptor` and a query-reading strategy (999 deeply bracketed pairs under the extended parser,
    3.6 ms against 1.0 ms; 32 KiB of `&` under the simple one, 0.3 ms against 0.02 ms). The header limit bounds it.
    Candidate follow-up: compute the first-level key without the parser and print a fixed marker above 1000 pairs.
28. `request.set_user(false)` from application code stays what the application wrote; it is not an authentication
    path. A user that serializes to `[]` or `{}` authenticates, as in the original (`Ok(Some(vec![]))` included).
