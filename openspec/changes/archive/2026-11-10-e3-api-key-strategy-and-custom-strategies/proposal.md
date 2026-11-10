# Proposal

Implements [randygil/corvis#45](https://github.com/randygil/corvis/issues/45) (epic
[#39](https://github.com/randygil/corvis/issues/39)). Design from the #40 spike
(`openspec/changes/archive/2026-11-08-e3-strategy-module-design/`: fact 8, decisions 2, 9 and 10, the `auth/api-key`
delta, task group 4, Review notes 6 and 8: keys in the query string only when explicitly enabled, the prefix compared
literally).

**Depends on #136** (`openspec/changes/e3-strategy-core-and-guard-responses/`): this change is written against the API
#136 proposes (`corvis::auth::Strategy`, `StrategyOutcome`, `AuthenticatedUser`, `Provider::strategy`, `AuthGuard`,
the fixture package `crates/corvis/tests/fixtures/auth/`). #136 is being implemented in another worktree while this
is drafted, so that API is described here as designed, not as shipped. By the epic's order this change is implemented
after #136, #42 and #41; if the API changes, this change is updated before work starts.

## Why

Machine clients (webhooks, internal services, partner integrations) authenticate with an API key, in the original
through `passport-headerapikey` or a hand-written `passport-custom` strategy. Corvis has neither:

- `docs/recipes/passport.md` is `status: unavailable`; the only mention of API keys in the docs is the OpenAPI scheme
  (`docs/openapi/security.md:22`, `add_api_key()`), which documents a scheme the framework cannot enforce.
- There is no constant-time comparison helper in `crates/corvis` (a search for `subtle` and `constant_time` in
  `crates/corvis/src` finds nothing), so a hand-written guard compares keys with `==`, which stops at the first
  different byte.
- The only request logger of the framework prints the full URL with its query string
  (`crates/corvis/src/common/interceptor.rs:88-91`, `LoggingInterceptor`), so a key sent as `?api_key=...` would be
  written to the log.

API keys are long-lived bearer secrets. The mistakes that matter are a comparison that leaks the key through timing,
a key that ends up in a log line or an error body, a key accepted from a place the operator did not intend (a URL),
and two different keys in one request of which a proxy checks one and the app the other. The framework should make
the safe path the default one. The issue also asks for the guide to writing a strategy: the core (#136) gives the
trait; what is missing is the documented pattern, checked against the original's `passport-custom`.

## What Changes

- `corvis::auth::api_key`, behind a new cargo feature `auth-api-key` (off by default, one optional dependency:
  `subtle`): `ApiKeyValidator` (the user's `validate(req, key)`, name `"headerapikey"` overridable with `const NAME`),
  `ApiKeyStrategy<V>` (the adapter registered with `Provider::strategy::<ApiKeyStrategy<V>>()`),
  `ApiKeyStrategyOptions`, `ApiKeySource`, `ApiKey`, `ExpectedApiKey`, `ApiKeySet<T>` and `constant_time_eq`.
- Where the key may come from: only the places configured, in the configured order.
  - `ApiKeySource::header(name)` with an optional literal prefix (`passport-headerapikey` 1.2.2; the default options
    read `X-Api-Key` with no prefix);
  - `ApiKeySource::bearer()`: `Authorization: Bearer <key>`, parsed like the bearer extractor of #42;
  - `ApiKeySource::query(name)`: only when configured, with a startup warning and a redacted URL in request logs.

  The request body and cookies are never a source.
- Two configured sources carrying different keys in one request make the strategy fail without calling the validator
  (divergence: the original has one source).
- Constant-time comparison: `ApiKey::ct_eq`, `ExpectedApiKey::matches`, `ApiKeySet::lookup` and `constant_time_eq`
  compare SHA-256 digests with `subtle`, so neither the matching prefix nor the length of the stored key is revealed
  by timing; `ApiKeySet::lookup` checks every entry. `ApiKey::sha256_hex()` supports the "store only the digest"
  pattern.
- No key material in logs or errors: `ApiKey` and `ExpectedApiKey` have a redacted `Debug` and no `Display` or
  `Serialize`; every failure reason is a fixed string; the framework logs no key; a key taken from the query marks its
  parameter so that the new `Request::redacted_url()` hides the value, and `LoggingInterceptor` uses it.
- Security limits, each with a default, a startup check and a scenario: `max_key_bytes` 512 (1 to 4096); control
  characters refused; an `ExpectedApiKey` or `ApiKeySet` entry shorter than 32 bytes is an error at construction (a
  startup error when built in a provider); empty source names or prefixes and options without sources are startup
  errors.
- Custom strategies: no new API (a user type implements `Strategy` and is registered with `Provider::strategy`, #136).
  This change adds the differential test against `passport-custom` 1.2.1 for the documented pattern, with dependency
  injection and a testing override, and the "Writing a strategy" guide with its security checklist.
- Differential tests: `api_key.mjs` (`passport-headerapikey` 1.2.2, and `passport-custom` 1.2.1 for the query and
  bearer variants) and `custom.mjs` (`passport-custom` 1.2.1) in the fixture package of #136, replayed by
  `crates/corvis/tests/auth_api_key.rs` and `auth_custom.rs` with no Node at test time.
- Docs (EN + ES): "API keys" and "Writing a strategy" sections in `docs/recipes/passport.md`, a pointer in
  `docs/openapi/security.md`, and the redaction note in `docs/interceptors.md`. Example: `examples/cats-app` gets one
  route for machine clients guarded by `AuthGuard::chain(["jwt", "headerapikey"])`.

## Capabilities

### New Capabilities

- `auth/api-key`: API key strategy, key sources, conflicting keys, constant-time comparison helpers, redaction of key
  material, limits, startup checks, differential verification.
- `auth/custom-strategies`: user-written strategies with dependency injection, their mapping from `passport-custom`,
  and their differential verification.

### Modified Capabilities

None. `auth/strategies` (#136) is used as designed. The URL printed by `LoggingInterceptor` has no requirement in
`openspec/specs/` today; the new behavior is specified in `auth/api-key`.

## Impact

- `crates/corvis/Cargo.toml`: optional `subtle = "2"` (2.6.1 is already in `Cargo.lock`, pulled by `digest` and
  `rustls`), feature `auth-api-key = ["dep:subtle"]`. `sha2 = "0.10"` is already a dependency (line 45).
- New `crates/corvis/src/auth/api_key.rs`, `#[cfg(feature = "auth-api-key")]`.
- `crates/corvis/src/http/request.rs`: `Request::redact_query_param(name)` and `Request::redacted_url()` (always
  compiled, no dependency).
- `crates/corvis/src/common/interceptor.rs:88-91`: `LoggingInterceptor` prints `redacted_url()`.
- `.github/workflows/ci.yml`: `auth-api-key` in the per-feature clippy loop (line 72) and a
  `cargo test -p corvis --features auth-api-key` step next to the `compression` one (line 114).
- `crates/corvis/tests/fixtures/auth/`: `passport-headerapikey` 1.2.2 and `passport-custom` 1.2.1 added to
  `package.json` and the lockfile; `api_key.mjs`, `api_key.json`, `custom.mjs`, `custom.json`.
- Tests: `crates/corvis/tests/auth_api_key.rs`, `crates/corvis/tests/auth_custom.rs`, unit tests in the new module, a
  property test in `crates/corvis/src/fuzz_tests.rs`.
- Docs: `docs/recipes/passport.md`, `docs/openapi/security.md`, `docs/interceptors.md` and their `docs/es/` copies.
- Example: `examples/cats-app` (one validator, one route, its tests).
- Port apps: none enables the feature; `LoggingInterceptor` and `Request` change, so the corvis-parity check is
  required.

## Out of scope

- Generating, storing, rotating or revoking API keys, scopes and per-key rate limits: application logic. The docs show
  the digest-storage pattern; a key management module is a possible new issue.
- Slow hashing of API keys (argon2, bcrypt): keys are random and at least 32 bytes, so a SHA-256 digest is the right
  storage form; `PasswordHasher` (#41) is for passwords.
- Request signing (HMAC of the body with a shared secret, timestamps, nonces), as the webhooks of payment providers
  use: possible new issue.
- An API key in the request body or in a cookie: no reference in the original, and cookies bring CSRF; a custom
  strategy can do it.
- Redacting the URL in the `404` message for unmatched routes (`Cannot GET /x?api_key=...`,
  `crates/corvis/src/router/mod.rs:659` and `668`): it mirrors the original, goes only to the sender of the key, and no
  strategy runs for an unmatched route. Noted in the docs; possible new issue for a general redaction setting.
- Redacting the cache key of `CacheInterceptor` (`crates/corvis/src/modules/cache.rs:207`, the URL): it is not
  logged, and separating cached responses by key is the correct behavior.
- Zeroizing key memory on drop (`zeroize`): possible new issue for all secrets of the framework.
- A `#[strategy("name")]` macro and a closure-based strategy builder (`passport-custom`'s callback form): the trait is
  the model (spike decision 1); possible new issue.
- Mutual TLS client certificates as machine credentials: possible new issue under the TLS work.
- `AuthGuard` on WebSocket gateways (fails closed since #136).
- OAuth2 (#43), sessions (#44), the authentication security review (#46) and the overall docs and example (#47).
