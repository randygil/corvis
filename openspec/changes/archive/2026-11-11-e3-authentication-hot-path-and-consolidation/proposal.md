# Proposal

Implements [randygil/corvis#186](https://github.com/randygil/corvis/issues/186) (epic
[#39](https://github.com/randygil/corvis/issues/39)). Source: an engineering-quality review (not a security review) of
the authentication module after #40 to #47, read against `dev` at `db5b4f6`. Every finding was checked against that
code; design.md "Findings as checked" gives the current lines, the two findings dropped and why. Related, and kept as
their own issues: [#181](https://github.com/randygil/corvis/issues/181) (an origin-check guard in the framework) and
[#182](https://github.com/randygil/corvis/issues/182) (building an application from runtime configuration), which the
review's notes on `examples/auth-app` point to.

## Why

The review found no new vulnerability. It found one parity gap, one test that synchronizes with `sleep`, per-request
work proportional to the size of a key or a session, copied fragments that already drift, and comments that contradict
the code:

- **`JwtService::sign` does not follow the original's rules** (`crates/corvis/src/jwt.rs:537-556`): `exp` is counted from
  the clock even when the payload carries `iat`; a payload that already has `exp` or `sub` is overwritten by the option,
  and one that has `iss` or `aud` wins over the module option, where the original refuses all of these. This project's
  promise is verified parity, so the rules are recorded from the original library and replayed (design.md Decision 2).
- **`a_busy_hasher_answers_503_for_every_user`** (`crates/corvis/tests/auth_local.rs:481-482`) assumes a thread took the
  only hashing slot within 30 ms.
- **Keys are parsed per call.** `JwtService` builds its encoding key, its decoding key (PEM and DER for RS/PS/ES) and its
  `Validation` on every `sign` and `verify`; a key that a JWT key provider returns is parsed on every request even when
  the provider hands back the same cached key; the JWT strategy rebuilds its `Validation` and checks the token header
  twice per request.
- **A session costs several passes over its data per request.** It is serialized only to be measured, walked for depth,
  cloned as the baseline for change detection, cloned again into a full record to call `touch` when nothing changed, and
  serialized twice on `update`. A request without a cookie draws an identifier from the operating system even when it
  will save nothing. When the memory store is full, one write may purge every expired record under the store's lock,
  against its own comment.
- **Duplication with diverging rules**: two warning limiters and eight copies of their suffix; the validator call written
  four times and the `503` built two ways with different key orders; two untrusted-JSON visitors; three `Bearer` parsers;
  two empty-user rules; three `__Host-` checks (one case-sensitive); two `valid_cookie_name` with different rules;
  `CookieKeys::sign` used as a MAC by cutting its output apart.
- **Docs that contradict the code**: `User<T>` is documented as reading only what an `AuthGuard` stored, while the session
  middleware and `log_in` store the typed user too, so `User<T>` on a route without a guard returns the session user.
- **Smaller inconsistencies**: the OAuth2 state cookie takes the first of several cookies with one name where the session
  and the JWT cookie extractor refuse duplicates; the API key strategy takes a write lock on every request for a
  redaction the application already registers; the example has a wrong comment and a guard that deserializes the user per
  request because no public accessor of the typed user exists.

There is no benchmark of the authentication paths, so the first task group builds one, and every performance decision
names the number it must move (design.md Decision 1).

## What Changes

- **Benchmark.** `bench/auth/`: a Corvis-only, release-mode benchmark of the JWT strategy, `JwtService` and the session
  middleware, driven by `oha` on `127.0.0.1`, reporting requests per second, latency percentiles and the server's
  allocations and bytes allocated per request. Run before and after; the numbers go to design.md "Measurements".
- **`JwtService::sign` parity.** A new recorder `crates/corvis/tests/fixtures/auth/jwt_sign.mjs` records what the
  original `JwtService.sign` returns or throws for the cases of design.md Decision 2 (payload `iat`; `exp`, `iss`, `aud`,
  `sub`, `jti` together with the matching option; `notBefore`; claim order; string and non-object payloads;
  `mutatePayload`). Corvis replays every case byte for byte and matches it, or the case carries a `divergence` with its
  reason. `JwtSignOptions` gains `not_before` and `jwt_id`, the two options the recorded rules need.
- **JWT keys prepared once.** `JwtService` parses its module keys and builds its `Validation` in `new`; errors keep coming
  from `sign` / `verify` with the same text and type. `JwtKey` shares one parsed key across its clones, so a provider that
  returns a cached key costs no parse. The JWT strategy prepares one `Validation` per key family at startup and checks
  the header once per request.
- **Sessions.** The size limit is checked by counting bytes with a writer that stops past the limit; change detection
  snapshots the data on the first write instead of cloning it on every load; an unchanged session is touched with a
  record that holds only `cookie` and `__corvis`; the identifier of a new session is drawn when it is first needed; the
  memory store purges expired records in batches of 64 when it makes room.
- **Consolidation.** One `WarnLimiter` and one suffix; one 503 body and one helper for running a validator with its
  deadline; one untrusted-JSON visitor; one `Authorization` header tokenizer for the JWT extractor and the API key
  `bearer()` source; one empty-user rule; one cookie-prefix check; `CookieKeys::mac` for the OAuth2 derivations, with
  their outputs unchanged, and HMAC keys prepared once per secret.
- **Fixes.** The OAuth2 state cookie sent twice is no state; the rustdoc of `User<T>` and the migration guide say that the
  session user is a typed user too, and `Request::typed_user::<T>()` reads it from a guard; the API key strategy marks its
  query names only outside an application; `Request::insert`/`get`/`remove` tolerate a poisoned lock; the busy-hasher test
  holds the slot through a hidden `PasswordHasher::hold_slot` instead of sleeping.
- **Nice to have.** Enums instead of booleans, a constant-time `PartialEq` for `OAuth2Token`, the misplaced doc comment,
  the test that cannot fail, test files renamed after what they test, borrowed default strategy names in `AuthGuard`, a
  rate-limited warning in `OAuth2Client::refresh` and the documented one-connection-per-call client.
- **Example.** `examples/auth-app`: the comment that names a variable that does not exist, and `RolesGuard` reading the
  typed user instead of deserializing it.
- **Docs (English and Spanish).** The migration guide's `JwtService::sign` row and its claim rules, `User<T>` in the
  passport recipe, the `touch` row of the session store contract, the `503` body, and the threat model's links to the
  renamed test files.

## Capabilities

### New Capabilities

- `bench/auth-hot-path`: the authentication benchmark (scenarios, what it reports, where it builds, how results are
  recorded).

### Modified Capabilities

- `auth/jwt`: added requirements for the signing rules of `JwtService` (verified against the original) and for keys
  prepared once.
- `auth/jwt-strategy`: added requirement for the work done once per key and once per token.
- `auth/sessions`: the store contract's requirement changes (`touch` receives the timestamps only); added requirements
  for an unchanged session not being copied, an identifier drawn only when needed, and purges in bounded batches.
- `auth/oauth2`: added requirement that a repeated state cookie is no state.
- `auth/strategies`: added requirements for the typed user as guards and handlers read it, and for one `503` body.

## Impact

- New: `bench/auth/{Cargo.toml,src/main.rs,run.sh,README.md}` (workspace member, `publish = false`),
  `crates/corvis/tests/fixtures/auth/jwt_sign.mjs`, `crates/corvis/tests/fixtures/auth/jwt_sign.json`,
  `crates/corvis/src/untrusted_json.rs`, `crates/corvis/src/auth/auth_header.rs`, `crates/corvis/src/auth/outcome.rs`,
  `crates/corvis/tests/auth_hot_path_heap.rs`.
- Edited: `crates/corvis/src/jwt.rs`, `crates/corvis/src/auth/{mod,authenticator,guard,strategy,user,warn_limit}.rs`,
  `crates/corvis/src/auth/jwt/{mod,key,options,extract}.rs`, `crates/corvis/src/auth/local/{hasher,strategy}.rs`,
  `crates/corvis/src/auth/api_key/strategy.rs`, `crates/corvis/src/auth/session/{manager,session,memory,store,login,
  middleware}.rs`, `crates/corvis/src/auth/oauth2/{state,strategy,options,json,secret,client}.rs`,
  `crates/corvis/src/http/{signed_cookie,request,res}.rs`, `crates/corvis/src/lib.rs`,
  `crates/corvis/tests/fixtures/auth/{package.json,package-lock.json}`, `crates/corvis/tests/auth_local.rs`, the renamed
  test files of tasks.md 9.3, `examples/auth-app/src/auth/{jwt,roles}.rs`, `Cargo.toml` (workspace member), and in
  `docs/` and `docs/es/`: `recipes/passport-migration.md`, `recipes/passport.md`, `http/session.md`,
  `security/encryption-and-hashing.md`, `security/authentication-threat-model.md`.
- Public API: additions `JwtSignOptions::{not_before, jwt_id}` (new public fields: a struct literal without
  `..Default::default()` needs them), `Request::typed_user::<T>()`, and the hidden `PasswordHasher::hold_slot`. Nothing
  changes signature.
- Behavior visible to applications and clients: listed in design.md "Behavior visible to applications and clients". The
  ones that matter: `JwtService::sign` refuses the payload-and-option conflicts the original refuses and computes `exp`
  from the payload's `iat`; the busy-hasher and OAuth2 `503` bodies change key order; a callback with two state cookies
  of one name is refused; a custom `SessionStore::touch` receives only `cookie` and `__corvis`.
- No new dependency. The fixture package pins the original JWT module and `jsonwebtoken` it records.
- Two of the port apps (consi, facten) sign their tokens with `JwtService`; the corvis-parity check runs and decides
  (design.md Risks).

## Out of scope

- An origin-check guard in the framework (#181) and building an application from runtime configuration (#182): the
  example keeps its own guard and its task-local settings.
- A deadline for the OAuth2 validator: it is the only validator without one today; adding it is a behavior change of
  its own (design.md Decision 9 keeps OAuth2's call as it is).
- `noTimestamp`, `keyid`, `header` and the other `jsonwebtoken` sign options Corvis does not have; verify-side options of
  the original that `JwtService::verify` lacks.
- Bypassing `jsonwebtoken` to keep `ring` key pairs between calls (design.md Decision 3).
- Making `login_tracker` return a `Result`, and pooling the OAuth2 client's connections (design.md Decision 13).
