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
