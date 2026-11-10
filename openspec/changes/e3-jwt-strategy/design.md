# Design

## Context

This change builds on #136 (`openspec/changes/e3-strategy-core-and-guard-responses/`), which adds `corvis::auth`
with `Strategy` (`authenticate(&ExecutionContext, &AuthenticateOptions) -> StrategyOutcome`), `AuthenticatedUser`,
`Provider::strategy::<S>()`, the registry, `AuthGuard` and the fixture package `crates/corvis/tests/fixtures/auth/`.
Everything below assumes that API; it is implemented after #136 lands. Line numbers were checked on `origin/dev` at
`6f5ff4d`.

### Corvis today

- `JwtModuleOptions` (`crates/corvis/src/jwt.rs:26-43`): one `algorithm` (default HS256), `secret`, optional PEM
  `public_key`, one `issuer`, one `audience`.
- `JwtService::verify` (`jwt.rs:251-293`): picks the first non-empty key of secret and public key (254-258), builds a
  `jsonwebtoken::Validation` for the one configured algorithm with no required claims, leeway 0, `exp`/`nbf` turned off
  in the crate (259-264) and `iss`/`aud` from the module options (265-271), decodes (272), then checks with
  node-jsonwebtoken's rules that `iat`, `nbf`, `exp` are numbers when present, `nbf > now` (not active) and
  `now >= exp` (expired) (274-291). `verify_or_unauthorized` maps every error to the empty 401 (301-303).
- `decoding_key` (`jwt.rs:172-182`) maps an algorithm to a `DecodingKey`: HMAC from the bytes, RSA/PS from PEM, EC
  only for ES256/ES384 (179), Ed from PEM.
- `extract_bearer_token` (`jwt.rs:315-319`) uses `req.header("authorization")` and `split_once(' ')`.
  `Request::header` returns `None` for any value that is not visible ASCII (`crates/corvis/src/http/request.rs:135-137`,
  `to_str()`), while `Request::cookies` decodes header bytes as latin1 (`request.rs:223-227`, `body_parser::latin1`),
  which is how Node exposes header strings.
- Spec `openspec/specs/auth/jwt/spec.md`: forged tokens (`alg` `none` in any case, other HMAC sizes, asymmetric, empty),
  tampered tokens and malformed input are rejected without a panic.
- The `jwt` feature (`crates/corvis/Cargo.toml:58`) enables `jsonwebtoken = "9"` (line 37); `Cargo.lock` has 9.3.1.
  It is a default feature (line 57).

### `jsonwebtoken` 9.3.1 (Rust, read in the local registry copy)

`decoding.rs:207-237` (`verify_signature`): refuses an empty algorithm list (212-214), refuses when any listed
algorithm's family differs from the key's family (216-221), refuses a header `alg` not in the list (228-230), then
verifies the signature. `Algorithm` has no `none` variant, so a `none` header does not parse. There is no token length
limit.

### The original (`passport-jwt` 4.0.1, `jsonwebtoken` 9.0.3)

- passport-jwt (spike fact 4): `jwtFromRequest` required; `secretOrKey` xor `secretOrKeyProvider(req, rawJwt, done)`,
  both or neither throw at construction; no token → `fail("No auth token")`; key provider error or verification error
  → `fail`; `validate` falsy → `fail`; options `issuer`, `audience`, `algorithms`, `ignoreExpiration` and
  `jsonWebTokenOptions` passed to `jsonwebtoken.verify`. The header parser is `/(\S+)\s+(\S+)/`
  (`lib/auth_header.js`), scheme compared lowercased.
- node-jsonwebtoken 9.0.3 `verify.js` (read in `corvis-tests/nest/facten-api/node_modules/jsonwebtoken`): `now` is
  `Math.floor(Date.now() / 1000)` (line 57); a token without signature needs `"none"` listed and no key (106-118);
  default algorithms by key type: secret → HS256/384/512, RSA → RS*/PS*, EC → ES256/384/512, other → RS*/PS* (11-18,
  132-142); header `alg` must be listed (144-146); HMAC with a non-secret key and RS/PS/ES with a non-public key are
  refused (148-152); `nbf > now + clockTolerance` → not active (180-181); `exp` must be a number and
  `now >= exp + clockTolerance` → expired unless `ignoreExpiration` (185-190); audience: any of the token's `aud`
  values equals any configured one (194-206); issuer: string equality or list membership (209-215); `iat` is checked
  only with `maxAge` (238-239). There is no EdDSA in its algorithm lists.

### Example to migrate

`examples/cats-app/src/auth/auth_guard.rs` is a global guard that skips `isPublic` routes, reads the bearer token with
`extract_bearer_token`, verifies it with `JwtService::verify_or_unauthorized` and calls `request.set_user`. Its module
signs tokens with `expires_in` (`examples/cats-app/src/auth/auth_module.rs:16`), so they carry `exp`.

## Goals / Non-Goals

**Goals**

- `AuthGuard::new("jwt")` with a validator class, as in the original, with the passport-jwt extraction and
  verification semantics, verified by a recorded fixture.
- Safe defaults: allow-list by key family, no `none`, `exp` required, bounded skew and token length, extraction only
  where configured; every default documented and testable.
- No behavior change for `JwtService` and `extract_bearer_token`.

**Non-Goals**

- A new `jsonwebtoken` major, ES512, JWKS, `maxAge` and the other `jsonWebTokenOptions` (see proposal Out of scope).
- Signing: `JwtService::sign` stays the way to issue tokens.

## Decisions

### 1. Under the existing `jwt` feature

The spike planned an `auth-jwt` feature (`auth` + `jwt`). #136 compiles `corvis::auth` unconditionally (its decision
1), so `auth-jwt` would equal `jwt`. The strategy lives in `corvis::auth::jwt` behind `#[cfg(feature = "jwt")]`; no
new dependency (the header parser is hand-written, not a `regex`).

### 2. Validator trait and options

```rust
#[async_trait]
pub trait JwtValidator: Injectable {
    const NAME: &'static str = "jwt";
    type User: Serialize + Clone + Send + Sync + 'static;
    fn options(&self) -> JwtStrategyOptions;
    async fn validate(&self, req: &Request, payload: Value) -> Result<Option<Self::User>, HttpException>;
    fn info(&self, _user: &Self::User) -> Option<Value> { None }
}

JwtStrategyOptions::new()
    .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
    .secret_or_key(JwtKey::secret(config.get("JWT_SECRET")?))   // or .secret_or_key_provider(provider)
    .issuer(["https://auth.example"]).audience(["web", "mobile"])
    .algorithms([JwtAlgorithm::HS256])
    .clock_tolerance(Duration::from_secs(5))
    .max_token_length(8192)
    // .ignore_expiration(true) / .allow_missing_exp()   (both warn at startup)
```

`JwtStrategy<V>` implements `Injectable` (resolves `V` with `ResolveCtx::get_or_construct::<V>()`,
`crates/corvis/src/di/injectable.rs:177`, calls `options()` once and validates it; an invalid configuration is a
`DiError` naming the strategy, so the application does not start) and `Strategy` (name `V::NAME`). Option names follow
passport-jwt so a migration maps one to one. `passReqToCallback` does not exist: the request is always passed (spike
decision 2).

`JwtKey`: `secret(bytes)`, `rsa_pem(pem)`, `ec_pem(pem)`, `ed_pem(pem)`; a static key is parsed when the strategy is
built. There is no auto-detection of a PEM inside a "secret" string (node tries `createPublicKey` on any string first,
verify.js:120-130): the explicit constructor removes that ambiguity.

`JwtKeyProvider`: `#[async_trait] trait JwtKeyProvider: Send + Sync { async fn key(&self, req: &Request, raw_token:
&str) -> anyhow::Result<JwtKey>; }`, given as `Arc<dyn JwtKeyProvider>`; the validator's `options()` can clone its own
injected services into it. Docs warn that the raw token is unverified (use its `iss`/`kid` only to choose a key).

### 3. Extractors

`ExtractJwt` builds an `Extractor` (an enum plus a `Custom(Arc<dyn Fn(&Request) -> Option<String> + Send + Sync>)`):

| Corvis | passport-jwt | Rule |
|---|---|---|
| `from_auth_header_as_bearer_token()` | `fromAuthHeaderAsBearerToken()` | scheme `bearer` |
| `from_auth_header_with_scheme(s)` | `fromAuthHeaderWithScheme(s)` | first `(\S+)\s+(\S+)` match on the latin1 value, JS whitespace (`\t \x0b \x0c \x20 \xa0` in the latin1 range), scheme equal ignoring ASCII case (no latin1 character lowercases to ASCII, so this equals JS `toLowerCase` here) |
| `from_header(n)` | `fromHeader(n)` | all values of `n` joined with `", "` (Node's join); name lowercased at build time (divergence for uppercase names, which never match in Node) |
| `from_url_query_parameter(n)` | `fromUrlQueryParameter(n)` | `req.query()[n]` if a string |
| `from_body_field(n)` | `fromBodyField(n)` | `req.body()[n]` if a string |
| `from_cookie(n)` | hand-written `req.cookies?.[n]` | `Request::cookie(n)` |
| `from_extractors([..])` | `fromExtractors([..])` | first non-empty |
| `custom(f)` | any function | `f(req)` |

An empty string counts as no token (JS falsy). Header values go through latin1 decoding, not `Request::header`,
because a token next to a non-ASCII byte must behave as in Node. The `Authorization` header uses the first value only:
Node discards duplicates of `authorization`.

### 4. Verification core shared with `JwtService`

The body of `JwtService::verify` (`jwt.rs:259-291`) moves to `pub(crate) fn verify_claims(token, key: &DecodingKey,
validation: jsonwebtoken::Validation, time: TimeRules) -> crate::Result<Value>` where `TimeRules { tolerance: u64,
ignore_expiration: bool, require_exp: bool }`. `JwtService::verify` calls it with its current `Validation` (including
the crate's `iss`/`aud` settings) and `TimeRules { 0, false, false }`, so its behavior and the `auth/jwt` spec do not
change. The strategy calls it with `validate_aud = false`, no crate issuer, and checks issuer and audience itself with
the node-jsonwebtoken rules (verify.js:194-215), which the spec pins (lists, `aud` arrays, non-string claims).

Alternative: re-implement verification in the strategy. Rejected: two copies of the time and numeric-claim rules would
drift.

### 5. Algorithm allow-list per request

For a static key, the allow-list is computed at startup: explicit `algorithms`, which must all be in the key's family
(else startup error; the crate would refuse at decode time anyway, `decoding.rs:216-221`, but per request), or the
default list of the key family (HMAC: HS256/384/512; RSA: RS256/384/512, PS256/384/512; EC: ES256/ES384; Ed25519:
none, explicit `algorithms([EdDSA])` required). With a key provider, the configured list (or every default list) is
filtered per request to the returned key's family; an empty result fails. Mixing families in the configured list is
allowed only with a key provider. So an HMAC token can never be checked against an RSA key, and `none` cannot be
expressed (no variant).

### 6. Limits and their defaults

| Limit | Default | Bound | Original |
|---|---|---|---|
| `max_token_length` | 8192 bytes | > 0 | none (Node's 16 KiB header limit for headers only) |
| `clock_tolerance` | 0 s | ≤ 300 s | `clockTolerance`, default 0, unbounded |
| `exp` | required | `allow_missing_exp()` opt-out, warns | optional |
| `ignore_expiration` | off | warns | `ignoreExpiration` |
| HMAC secret length | none enforced | warning under 32 bytes (RFC 7518 3.2 asks for at least the hash size) | none |
| Query extractor | not configured | warns when configured | none |

The length check runs on the extracted string before the key provider and before any base64 or JSON decoding, so an
oversized token costs nothing but the comparison.

### 7. Failure reasons and auth info

Every failure is `StrategyOutcome::Fail { challenge: Some({"message": reason}), status: None }`, with node-jsonwebtoken's
messages where one exists (`jwt expired`, `jwt not active`, `invalid signature`, `invalid algorithm`, `jwt malformed`,
`jwt audience invalid. expected: ...`, `jwt issuer invalid. expected: ...`) and `No auth token`, `token too long`, `exp
claim required`, `key provider error` otherwise. The original's `req.authInfo` is a serialized `Error` object whose
shape varies (`{}` for a plain `Error`), so the fixture does not compare auth info for JWT cases; it compares status,
headers and body.

### 8. Fixture `jwt.mjs`

Added to `crates/corvis/tests/fixtures/auth/` (package of #136): `passport-jwt` 4.0.1 pinned, `jsonwebtoken` resolved
by the lockfile (9.0.x). The generator declares strategies equivalent to the Rust test's (bearer; scheme `JWT`;
`fromHeader('x-token')` and `fromHeader('X-Token')`; cookie through `cookie-parser`; query; body; `fromExtractors`;
issuer and audience lists; `secretOrKeyProvider` by unverified `iss`, and one that errors; RS256 with
`tests/fixtures/jwt_rsa_public.pem`; ES256 with a new P-256 pair; `ignoreExpiration`; `clockTolerance: 30`). Cases
that do not depend on time use fixed tokens (signed once, `exp` in 2099 or in 2000). Cases that depend on time are
recorded as a token template (claims with offsets from now, the key name) plus the outcome; the Rust test signs a fresh
token from the template, with margins of at least 20 seconds so a slow machine cannot change the result. Marked
divergences: token without `exp`, non-numeric `iat`, string payload, uppercase `fromHeader` name, oversized token.

### 9. Example

`examples/cats-app`: `JwtAuth` validator (`JwtValidator`, `type User = JwtPayload`), registered with
`Provider::strategy::<JwtStrategy<JwtAuth>>()`, and the global guard becomes
`app.use_global_guards(AuthGuard::new("jwt").skip_public())` (or the existing `Provider::app_guard` with a thin
wrapper, whichever keeps the example's tests unchanged); its `posture.rs` and `startup.rs` tests keep passing.

## Risks / Trade-offs

- **Required `exp` breaks apps whose tokens have none.** Mitigation: clear 401 reason in the auth info, the startup
  option, and a migration note in the docs. `JwtService::sign` adds `exp` whenever `expires_in` is set.
- **Refactor of `JwtService::verify`** could change port-app behavior. Mitigation: the existing unit tests and the
  `auth/jwt` spec tests, plus the corvis-parity check.
- **Latin1 header handling** differs from `Request::header`; a dedicated proptest covers random bytes.
- **Key provider latency** is the app's: it runs per request; docs recommend caching keys.
- **The fixture's time-relative cases** rely on margins, not on timing assertions.

## Open questions

1. Should `allow_missing_exp()` exist at all? The design keeps it (with a warning) so apps with long-lived service
   tokens can migrate; the maintainer may prefer no opt-out.
2. Should the HMAC secret minimum be an error instead of a warning? A warning keeps migrations with short secrets
   working; an error would be safer for new apps.
3. Default `max_token_length` of 8192 bytes: large enough for identity-provider tokens with group claims seen in
   practice; confirm.

## Review notes (2026-11-09, maintainer)

1. `allow_missing_exp()` stays as an explicit opt-out with a startup warning; the default requires `exp`.
2. An HMAC secret shorter than 32 bytes is a startup error, not a warning (Corvis targets new projects; security over
   parity, RFC 7518 section 3.2). Test fixtures use secrets of at least 32 bytes. `JwtService` keeps its current
   behavior; only the strategy enforces this.
3. `max_token_length` 8192 bytes is confirmed.
4. Implemented after #136 lands: start by comparing the design with the API #136 shipped, and record the adjustments
   in Implementation notes.

## Implementation notes

Written while implementing, after comparing the design with what #136 shipped (`corvis::auth`, archived change
`2026-11-10-e3-strategy-core-and-guard-responses`).

### Adjustments to the API of #136

1. **`Strategy::name` is a method, not a constant.** `JwtStrategy<V>` returns `V::NAME` from it, so a validator still
   chooses the name with `const NAME`. `Strategy::authenticate(&ExecutionContext, &AuthenticateOptions)` is as the
   design assumed, `AuthenticatedUser::new` returns a `Result` (a user that does not serialize is an `Error` outcome)
   and `Provider::strategy::<S>()` needs `S: Injectable + Strategy`, which `JwtStrategy<V>` implements by hand
   (`ResolveCtx::get_or_construct::<V>()`).
2. **The typed user lives in a private slot** read only by `User<T>`: the validator's `User` type needs no
   registration, and handlers read it with `User<T>` (or `request.user_as::<T>()` for the JSON form), never with
   `Request::get::<T>()`.
3. **Failures carry the reason as the challenge** (`{"message": ..}`), which `AuthGuard` keeps as `request.auth_info()`
   and never sends; no `Fail` carries a status.
4. **No `auth-jwt` feature** (decision 1 of the design stands): the module is `#[cfg(feature = "jwt")]`, so the CI
   feature loop needed no change. The three new integration test files are `#![cfg(feature = "jwt")]`.
5. **Warnings run in `on_module_init`**, not in the constructor, so a logger installed with `use_logger` after
   `CorvisFactory::create` sees them. Startup errors are `DiError::Factory` errors raised by the constructor
   (`Error while instantiating JwtStrategy "jwt" (<validator type>): invalid JWT strategy options: <problem>`), so
   they surface from `CorvisFactory::create`.

### Decisions

6. **HMAC secrets under 32 bytes are a startup error** (Review note 2), also for a secret a key provider returns: that
   request fails with `key provider error` and the reason goes to the debug log. The spec requirements were rewritten
   accordingly (the warning scenario became "Short secret" under the startup checks; the warnings requirement keeps
   `ignore_expiration`, `allow_missing_exp` and the query extractor).
7. **Shape of the verification core** (tasks 1.1 and 1.2): `verify_claims(token, &DecodingKey, &Validation,
   &TimeRules)` returns `VerifyError` (`Decode(jsonwebtoken::errors::Error)` or `Claim(&'static str)`) so the strategy
   maps reasons without parsing text; the time rules are the pure function `check_time_claims(&Value, &TimeRules,
   now)`, which unit tests call with a fixed `now`. `JwtService::verify` passes `TimeRules::default()` (tolerance 0,
   `exp` optional, not ignored) and keeps its messages unchanged.
8. **`JwtKey` keeps the key material and parses it on demand.** A static key is parsed once, when the strategy is
   built; a key a provider returns is parsed per request. The `Debug` of `JwtKey`, `JwtStrategyOptions` and
   `JwtExtractor` prints kinds and sizes, never values, and parse errors never include key text.
9. **`issuer` and `audience` take one value or a list** through `StringList` (`From<&str>`, `String`, `[&str; N]`,
   `Vec<String>`, `Vec<&str>`).
10. **`from_header` mirrors Node beyond `", "`**: the headers Node keeps once (`authorization`, `user-agent`, ...) give
    their first value and `cookie` joins with `"; "`. `from_cookie` uses `Request::cookie`, which parses like
    cookie-parser.
11. **The `Authorization` header parser** takes the first two runs of non-space characters, which is what
    `/(\S+)\s+(\S+)/` yields at its first match; JavaScript whitespace in the latin1 range is `\t \n \v \f \r`, space
    and `\xa0` (the design list omitted the line breaks, which a header cannot contain anyway).
12. **Token and key never reach a log or a message.** Failure reasons are fixed strings; `jwt audience invalid` and
    `jwt issuer invalid` do not repeat the configured values (node's do), and the key provider's own error text is
    logged at debug level only. `tests/auth_jwt_log.rs` captures every log level while bad tokens are sent.

### Fixture and tests

13. **Fixture shape.** Cases carry a token spec instead of fixed tokens (`{alg, key, claims}`, `{payloadText}`, `{pad}`
    or `{raw}`; a claim `{"$now": n}` is `now + n`), signed by `jws` in the generator and again by `jsonwebtoken` in
    the Rust test. Every case, time-dependent or not, is replayed this way. `jwt.json` has 65 cases and two runs of
    `jwt.mjs` give identical files. The time cases keep at least 20 s between the claim and the boundary that
    would flip the result (offsets of 10, 60 and 3600 s against a tolerance of 0 or 30 s); no assertion measures
    elapsed time.
14. **Repeated custom header** is recorded as one header `x-token: <t>, <t>` (what Node's `fetch` sends and Node's
    server joins); two real headers, the no-break space and non-UTF-8 bytes are unit tests of the extractor, because
    `TestRequest` sends neither.
15. **Divergent cases (5):** uppercase `from_header` name, oversized token, token without `exp`, string `iat`, string
    payload. Each records the status Corvis answers (`divergence.status`); the replay asserts it.
16. **Extra test files:** `tests/auth_jwt_options.rs` (startup errors, opt-outs, limits, validator results, a proptest
    of random credentials through HTTP) and `tests/auth_jwt_log.rs` (its own binary: the logger is process-wide).
17. **Example.** `examples/cats-app` keeps a global guard named `AuthGuard` that skips `#[public]` targets and delegates
    to `corvis::auth::AuthGuard::new("jwt")`: the framework's guard denies every WebSocket message, even with
    `skip_public`, and the chat gateway is public. The wrapper does not override `CanActivate::as_any`, so its strategy
    name is not checked at startup (and the gateway warning of `corvis::auth` does not fire for a guard that lets
    public targets through). `JwtAuth` reads `JWT_SECRET` like `JwtModule` does; the existing startup tests still see
    the `JWT_SECRET` error first because `JwtService` resolves before the strategy. A new `tests/jwt_auth.rs` covers
    login, profile, bad tokens and a public route.
