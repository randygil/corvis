# Design

## Context

Issue #186 holds the findings of an engineering-quality review of `crates/corvis/src/auth/`, `crates/corvis/src/jwt.rs`
and `crates/corvis/src/http/signed_cookie.rs` after the seven changes of epic #39. Every line below was read on `dev` at
`db5b4f6`; task 1.1 re-checks them before any edit and notes moved lines here.

### Findings as checked

| Id | Finding | Current code | Verdict |
|---|---|---|---|
| M1 | `JwtService::sign` differs from the original | `jwt.rs:527-565`: key chosen and copied (530-536), `iat` kept with `or_insert` but `exp` = clock + `expires_in` (539-545), `iss`/`aud` with `or_insert` (547-552), `sub` with `insert` (553-555) | Confirmed. Fixed from a recorded fixture (Decision 2) |
| M2 | Test synchronized with `sleep` | `tests/auth_local.rs:481-482` (holder thread, 30 ms sleep); the slot count is not observable from a test | Confirmed (Decision 13d) |
| S1 | JWT keys parsed per request | `jwt.rs:530-536,563` (sign: key copied, `EncodingKey` per call), `jwt.rs:579-593` (verify: `Validation` and `DecodingKey` per call); `auth/jwt/mod.rs:234` with `key.rs:59-70,116-125` (a provider's key parsed per request); `check_header` at `auth/jwt/mod.rs:273` and again at `jwt.rs:275`; `validation_for` (`auth/jwt/mod.rs:173-182`) per request at 190 and the `allowed.clone()` at 213. A static strategy key is already parsed once (`auth/jwt/options.rs:286-307`) | Confirmed (Decision 3). Also found: `jsonwebtoken` 9.3.1 parses the DER of an RSA or EC private key on every sign (`crypto/rsa.rs:41`, `crypto/ecdsa.rs:35`); out of reach (Decision 3) |
| S2 | Session passes proportional to its size | `session/manager.rs:163` (serialized to be measured), 168 (`too_deep`), `session/session.rs:117-124` (baseline clone), 112-114 (`data != baseline`); unchanged commit `manager.rs:334-344` builds the full record (`session.rs:154-162`) for `touch`, of which `memory.rs:426-434` keeps two keys; `update` serializes at `manager.rs:197` and `memory.rs:344` | Confirmed (Decisions 4 to 6) |
| S3 | OS identifier on every request without a cookie | `manager.rs:145` → `State::fresh` (`session.rs:91-92`) → `new_id` (`session.rs:42-46`); `save_uninitialized` is off by default (`session/options.rs:184`), so most of these identifiers are thrown away | Confirmed (Decision 7) |
| S4 | Unbounded purge under the store's lock | `memory.rs:389-391` (`purge(now, usize::MAX)` in the room loop) against `memory.rs:21-22` ("no request pays for a large purge"). The comment at `memory.rs:337-339` is about the size of a record, not the number of expired ones | Confirmed against 21-22 (Decision 8) |
| S5 | Duplicated limiter, suffix copied | `local/hasher.rs:326-357` re-implements `auth/warn_limit.rs` without its cap of 64 reasons; the suffix at `authenticator.rs:228-230` (a helper), `session/manager.rs:108,125,170`, `session/memory.rs:417`, `jwt/mod.rs:248`, `api_key/strategy.rs:311`, `local/strategy.rs:359`, and with other words at `oauth2/strategy.rs:288` ("since the last line") and `hasher.rs:858` ("of this kind in the last minute were not logged") | Confirmed (Decision 10) |
| S6 | Validator call written four times, 503 built two ways | `local/strategy.rs:354-377`, `jwt/mod.rs:287-304`, `api_key/strategy.rs:349-366`, `oauth2/strategy.rs:565-583` (no deadline, other rules for `None` and empty users); `HttpException::new((), 503)` (`jwt/mod.rs:151-153`, `local/strategy.rs:365`, `api_key/strategy.rs:353`) gives `{"statusCode":503,"message":"Service Unavailable"}`, `ServiceUnavailableException::empty()` (`hasher.rs:108`, `oauth2/strategy.rs:531,551,559`) gives `{"message":"Service Unavailable","statusCode":503}` | Confirmed (Decision 9). The local strategy answers both forms, depending on the cause |
| S7 | More duplication | untrusted-JSON visitor `jwt.rs:320-379` and `oauth2/json.rs:10-58`; `Bearer` parsers `jwt.rs:631-636`, `auth/jwt/extract.rs:153-169`, `api_key/strategy.rs:208-246`; empty-user rule `session/login.rs:12-21` and `strategy.rs:151-158`; `__Host-` checks `jwt/options.rs:322` (case-sensitive), `oauth2/strategy.rs:211-214`, `session/options.rs:320-322` and `oauth2/options.rs:763` (case-insensitive); `valid_cookie_name` at `http/res.rs:208-211` (cookie 0.7 regex) and `oauth2/options.rs:727-730` (RFC 6265 token, 128 bytes) | Confirmed (Decision 11) |
| S8 | `CookieKeys::sign` used as a MAC | `oauth2/state.rs:53-55,95-97,123-124` (sign, cut at the last `.`, hash); HMAC keyed per call at `signed_cookie.rs:66`, per secret per verification at 129 | Confirmed (Decision 12) |
| S9 | Docs contradict the code on the typed user | `user.rs:12-15`, `strategy.rs:117` and `http/request.rs:285-288` say only `AuthGuard` stores it; `session/middleware.rs:113` and `session/login.rs:122` store it too; `docs/recipes/passport.md:193,199` repeat the wrong rule while `docs/http/session.md:483` states the right one | Confirmed (Decision 13e, Review note 1) |
| S10 | OAuth2 state cookie takes the first of several | `oauth2/state.rs:283` (`req.cookie`) | Confirmed (Decision 13a) |
| S11 | Per-request work for tests | `api_key/strategy.rs:338-344` takes the extensions write lock and allocates on every request although `application.rs:556` registers the names for the whole application (`http/redact.rs:30-37`) | Confirmed (Decision 13b) |
| S12 | Example | `examples/auth-app/src/auth/jwt.rs:45` names `JWT_EXPIRES_IN`, which the example never reads (`app_module.rs:45` hard-codes `"15m"`); `roles.rs:20` deserializes the JSON user per request | Confirmed (Decision 14) |
| N1 | Poisoned lock unwrapped | `http/request.rs:275-283` (`insert`, `get`, `remove` with `unwrap()`); `RejectUnlessDone::drop` (`router/mod.rs:974-982`) calls `insert` while unwinding | Confirmed (Decision 13c) |
| N2 | Authenticator resolved and names cloned per request | `guard.rs:209-212`, `guard.rs:149-150`, `authenticator.rs:150-152` | Partly: the clone is removed, the lookup stays (Decision 13g) |
| N3 | Booleans that should be enums; `OAuth2Token: PartialEq` | `oauth2/strategy.rs:286,324`, `auth/mod.rs:166-185` (`Vec<bool>`); `oauth2/secret.rs:10` derives a short-circuit `PartialEq` | Confirmed (Decision 13f) |
| N4 | Misplaced doc comment, test that cannot fail, test files named after review rounds | `oauth2/options.rs:732-734`; `strategy.rs:226-236` asserts the length of a literal array; `tests/auth_session_review.rs`, `auth_session_review2.rs`, `auth_oauth2_review.rs` | Confirmed (Decision 13h) |
| N5 | `RequestRejected` marked in five places; the mark at `router/mod.rs:717` is redundant | `router/mod.rs:708,717,884,959,980` | **Dropped.** The mark at 717 is the only one for a guard that denies (`Ok(false)`) or fails (`router/mod.rs:829-833` return `Pipeline::Done(Err(..))` without marking) and for a panic outside the handler; the session middleware needs it when a filter turns that error into a response (`session/middleware.rs:90`). Removing it would keep a guard login on a rejected request |
| N6 | OAuth2 client connection per call; `login_tracker` panics; `OAuth2Client::refresh` logs without a limit | `oauth2/client.rs:306` (`connection: close`), `local/strategy.rs:213-214`, `oauth2/client.rs:645-647` | Client documented, refresh limited; **`login_tracker` dropped**: the panic is documented (`# Panics`, `local/strategy.rs:207-210`), happens while the application is composed, for the options that would stop the strategy at startup anyway, and a `Result` would make every caller unwrap (Decision 13i) |

### Security invariants these changes come near

From the "Security review" sections of the archived designs of #42, #41, #45, #44, #43 and #46. Each refactor near one
keeps its existing test and, where the refactor changes the code that enforces it, gains the new test named here.

| Invariant | Enforced at | Existing test | Refactor near it | New pinning test |
|---|---|---|---|---|
| The token header is checked before the key provider and before decoding; `crit` and a non-object header are refused by the strategy and by `JwtService::verify`; no `serde_json::Value` on unverified bytes | `jwt.rs:499-507`, `auth/jwt/mod.rs:270-277`, `jwt.rs:275-279` | `a_header_that_is_not_plain_json_is_refused_before_the_decoder` (`jwt.rs`), `tests/auth_jwt_header.rs` | header checked once (Decision 3), visitor moved (Decision 11) | `verify_still_refuses_crit_after_the_single_header_check` (unit, `jwt.rs`); the provider counter of `a_token_with_a_crit_header_or_the_wrong_shape_never_reaches_the_key_provider` stays at 0 |
| Algorithms are allowed by key family; a provider's key narrows the list; an HMAC token never meets an RSA key | `auth/jwt/key.rs:52-57`, `options.rs:286-307` | `jwt.json` cases "provider returns an RSA key for an HMAC token", "HMAC token signed with the RSA public key" | validations prepared per family (Decision 3) | `prepared_validations_follow_the_key_family` (unit, `auth/jwt/mod.rs`) |
| A provider's HMAC secret under 32 bytes is refused per request | `auth/jwt/mod.rs:230-233` | `tests/auth_jwt_options.rs` | shared parse in `JwtKey` (Decision 3) | `a_cached_short_secret_is_refused_on_every_request` |
| No key material in errors or `Debug` | `key.rs:72-79,116-125`, `jwt.rs:90-126` | `a_key_does_not_print_its_bytes`, `keys_parse_and_report_errors_without_key_material`, `debug_of_every_public_type` (`tests/auth_invariants.rs`) | cached keys in `JwtService` and `JwtKey` | `debug_of_every_public_type` extended to a `JwtKey` whose parse is cached and to `CookieKeys` with prepared keys |
| `JwtService::verify` checks `iss`/`aud` itself and errors stay downcastable to `jsonwebtoken::errors::Error` | `jwt.rs:590-608` | spec `auth/jwt` scenarios, `jwt.rs` unit tests | cached `Validation` and keys | `verify_errors_keep_their_type_with_cached_keys` |
| Callbacks have deadlines; every caller gets the same `503` | `jwt/mod.rs:217-228,287-293`, `local/strategy.rs:354-366`, `api_key/strategy.rs:349-355` | `tests/auth_callback_timeouts.rs`, `tests/auth_api_key_options.rs` | validator helper (Decision 9) | `every_framework_503_has_the_same_bytes` |
| Repeated session cookies are not believed; a signed unknown identifier is never adopted; a record over `max_session_bytes` or too deep is no session | `manager.rs:144-176` | `tests/auth_session_http.rs`, `auth_session_review.rs` | counting writer (Decision 4) | `the_size_limit_holds_at_the_boundary` (record of exactly `max_session_bytes` loads, one byte more does not, on load and on write) |
| A destroyed session is never re-created; `touch` never writes data; a logout racing a slow request wins | `manager.rs:266-353`, `memory.rs:340-404` | `auth_session_review.rs`, `auth_session_review2.rs` (races held with gates) | snapshot on write, timestamps-only touch (Decisions 5, 6) | `touch_receives_only_the_timestamps`; the race suites unchanged |
| A failure of the random generator writes nothing and never yields a weaker identifier | `session.rs:42-46,91-95`, `manager.rs:315-317` | session unit tests of the generator failure | identifier drawn when needed (Decision 7) | `a_failed_draw_at_commit_writes_nothing` |
| The memory store does nothing proportional to a record under its lock, keeps its bounds and evicts anonymous sessions first | `memory.rs:337-404` | `tests/auth_session_memory_heap.rs`, `memory_tests.rs` | purge in batches (Decision 8) | `one_write_purges_a_bounded_number_of_expired_records` |
| Signed values: every secret is tried without stopping, compared in constant time, input over 4096 bytes refused before any HMAC | `signed_cookie.rs:115-137` | `HMAC_RUNS` tests in `signed_cookie.rs`, `tests/signed_cookies.rs` (fixture) | prepared HMAC keys (Decision 12) | the `HMAC_RUNS` tests keep counting one run per secret |
| An OAuth2 flow is bound to its session and to its user; the PKCE verifier is derived, never stored | `oauth2/state.rs:48-125` | `oauth2_cookie_state_is_bound_to_the_session_it_started_under`, `a_flow_is_bound_to_the_user_that_started_it` | `CookieKeys::mac` (Decision 12) | `derivations_are_unchanged` (fixed vectors taken from the code before the change) |
| A token cookie sent twice is no token | `auth/jwt/extract.rs`, `http/request.rs` (`cookie_values`) | `a_jwt_cookie_sent_twice_is_no_token` | the same rule for the state cookie (Decision 13a) | `a_state_cookie_sent_twice_is_no_state` |
| API key query names are redacted in request logs, inside and outside an application | `api_key/strategy.rs:338-344`, `http/redact.rs` | `tests/auth_api_key*.rs` redaction cases | mark skipped inside an application (Decision 13b) | `a_strategy_outside_an_application_still_marks_its_names` |
| The typed user is read only from its own slot; `set_user` drops it | `strategy.rs:117-126`, `http/request.rs:285-294` | spec `auth/strategies` "Foreign extension is not a user" | `Request::typed_user` (Decision 13e) | `typed_user_never_reads_a_foreign_extension` |
| A guard login is discarded when the pipeline rejects the request | `router/mod.rs:708-718,974-982`, `session/middleware.rs:90` | `auth_session_review2.rs` | tolerant locks (Decision 13c) | `a_handler_that_panics_with_a_poisoned_lock_is_a_500` |
| A busy hasher answers the same `503` for every user and runs no derivation for a refused call | `hasher.rs:796-808`, `hasher.rs:106-110` | `a_busy_hasher_answers_503_for_every_user` | `hold_slot`, 503 body (Decisions 9, 13d) | the same test, without a thread or a sleep |

### Port apps and `JwtService`

consi and facten sign their access tokens with `JwtService` (`corvis-tests/corvis/consi-backend/src/auth/auth.service.rs:50`,
`facten-api/src/auth/auth.service.rs:50`): a payload with `sub` and no `iat`, `exp`, `iss`, `aud` or `jti`, a module
`expires_in` and no per-call options. houseplan does not use `JwtService`. Their originals use the original JWT module at
11.0.2 over `jsonwebtoken` 9.0.3 (`corvis-tests/nest/facten-api/package.json`).

## Goals / Non-Goals

**Goals**

- `JwtService::sign` returns what the original returns, byte for byte, or the error it throws, for every case of a
  recorded fixture, except recorded divergences with their reason.
- No key parse, `Validation` build, second header check, identifier draw or copy of session data that a request does not
  need; each removal measured.
- One implementation of each duplicated rule, with the behaviour of every caller kept unless a decision says otherwise.
- Docs and rustdoc that say what the code does.

**Non-Goals**

- A new feature of the authentication module, a new strategy, or a change to a security decision of an earlier review.
- Bypassing `jsonwebtoken` for the cryptography (Decision 3).
- The OAuth2 validator's missing deadline (proposal Out of scope).
- #181 and #182.

## Decisions

### 1. A benchmark of the authentication paths, and the number each decision moves

The synthetic benchmark (`bench/run.sh`, spec `bench/synthetic`) compares the same endpoints across Corvis and Node, and
`bench/port-vs-original` measures whole applications; neither exercises a key provider, `JwtService` with RSA keys or a
large session, and adding Corvis-only routes to the synthetic application would put the session middleware in front of
its cross-framework scenarios. So, as the Socket.IO hot-path change does for its module, this change adds a Corvis-only
benchmark, `bench/auth/`:

- A workspace member `corvis-auth-bench` (`publish = false`, `corvis` with `jwt` and `auth-session`) whose binary serves
  one route per scenario on `127.0.0.1`, installs a counting global allocator and a request counter, and answers
  `GET /__bench/stats` with `{requests, allocations, bytes}`. The session module's cookie path is `/session`, so the JWT
  routes do not run the session middleware.
- `bench/auth/run.sh`: builds with `cargo build --release -p corvis-auth-bench` in the workspace target directory (no
  target directory of its own), starts the server, fetches the tokens and seeds the session cookie through setup
  endpoints, and for each scenario runs a warmup, reads the stats, runs `oha` closed-loop (`CONNS`, `DURATION`), reads the
  stats again, then runs `oha` at a fixed rate for latency. It writes `bench/auth/results/<label>.json` (ignored by git)
  and a table for design.md. `--label before` / `--label after`.

| Scenario | Route | What it exercises |
|---|---|---|
| `jwt-hs256` | `GET /jwt/hs256`, bearer | strategy with a static HMAC key: `Validation` per request, header checked twice |
| `jwt-provider-rs256` | `GET /jwt/provider`, bearer RS256 | key provider returning a clone of one cached `JwtKey`: PEM parse per request |
| `service-verify-rs256` | `GET /service/verify`, bearer RS256 | a guard calling `JwtService::verify_or_unauthorized` (the cats-app pattern): PEM and `Validation` per call |
| `service-sign-hs256`, `service-sign-rs256` | `POST /service/sign`, `/service/sign-rs256` | `JwtService::sign` per request (a login without the password check) |
| `session-none` | `GET /session/none`, no cookie | the session middleware for a request that saves nothing |
| `session-read` | `GET /session/read`, cookie of a 16 KiB session | load, change detection and the unchanged commit |
| `session-write` | `POST /session/write`, same cookie | one key changed: the update path |

The benchmark informs; it is not a test and nothing asserts its timings. What each decision must move, and what asserts it:

| Decision | Number | Asserted by |
|---|---|---|
| 3, `JwtService` keys | key parses per `sign`/`verify` with module keys: 1 → 0; allocations per request in `service-verify-rs256` and `service-sign-rs256` | thread-local parse counter in `jwt.rs` (unit), benchmark |
| 3, `JwtKey` | parses of a key a provider returns again: 1 per request → 0 after the first | parse counter in `auth/jwt/key.rs` (unit), benchmark `jwt-provider-rs256` |
| 3, header and validation | header checks per strategy request: 2 → 1; `Validation` builds per request: 1 → 0 | counters (unit), allocations per request in `jwt-hs256` |
| 4 to 6, session passes | bytes allocated by the middleware for an unchanged session, beyond the store's own parse: about 3 × the record → at most 4 KiB, for records of 16 KiB and 64 KiB | `tests/auth_hot_path_heap.rs` (counting allocator), benchmark `session-read` |
| 4, update | buffers the size of the record allocated by one `update`: 2 → 1 | `tests/auth_hot_path_heap.rs` |
| 7, identifier | identifiers drawn for a request without a cookie that saves nothing: 1 → 0 | thread-local draw counter in `session.rs` (unit), benchmark `session-none` |
| 8, purge | expired records removed by one write under the lock: all of them → at most the records needed for room plus 64 | `memory_tests.rs` |
| 12, HMAC keys | HMAC key setups per signed-cookie verification: one per secret → 0 | benchmark `session-read` |
| 13b, API key | extension writes per request inside an application: 1 → 0 | unit test in `api_key/strategy.rs` |

Each bound of `tests/auth_hot_path_heap.rs` is chosen from the numbers measured before and after, and the task that adds it
checks that it fails on the code before the change. A regression of more than 5% in any scenario between the two runs is
investigated and explained in "Measurements" before archiving.

Alternatives: Corvis-only scenarios in `bench/run.sh` behind a second binary (mixes a cross-framework report with a
single-framework one and changes `bench/synthetic`); a microbenchmark crate such as criterion (a new dependency, and the
repository has none); only the allocation tests (they show the copies, not what a request costs end to end).

### 2. `JwtService::sign` follows a fixture recorded from the original

The issue's description and the reviewer's reading of the original are not trusted: the rules come from recording the
original and replaying it, as every strategy of the epic did.

**Recorder.** `crates/corvis/tests/fixtures/auth/jwt_sign.mjs`, in the existing fixture package (`npm ci && node
jwt_sign.mjs`, output `jwt_sign.json`, committed). It calls the original `JwtService` class directly
(`new JwtService(moduleOptions).sign(payload, callOptions)`), from `@nestjs/jwt` 11.0.2 over `jsonwebtoken` 9.0.3, the
versions the port apps' originals use; both are pinned as direct dependencies in the fixture `package.json` (today
`jsonwebtoken` comes in through `passport-jwt` only), and the task checks that `node jwt.mjs` still writes an identical
`jwt.json` after the lockfile changes. No HTTP server is needed. The clock is fixed: `Date.now` is replaced by
`() => CLOCK * 1000` around each call (`CLOCK = 1767225600`, 2026-01-01T00:00:00Z) and restored after it. Keys: `s1`
(the secret of `jwt.mjs`) and `rsa` (`tests/fixtures/jwt_rsa_private.pem`).

**Format.** `{ versions, clock, cases: [{ name, module, call, payload, result, payloadAfter, divergence }] }` where
`module` and `call` are the original's options with keys by name (`module: { secret: "s1" }` or `{ privateKey: "rsa",
algorithm: "RS256" }`, plus `expiresIn`, `issuer`, `audience`; `call: { expiresIn, subject, jwtid, notBefore,
mutatePayload }` or `null`), `result` is `{ token }` or `{ error }` (the thrown message), `payloadAfter` is the caller's
object after the call, and `divergence` is `null` or `{ reason, result }` with what Corvis returns instead.

**Replay.** A unit test in `crates/corvis/src/jwt.rs`, `sign_matches_the_recorded_original`, reads the file with
`include_str!("../tests/fixtures/auth/jwt_sign.json")` (the file already includes the PEM fixtures this way) and calls a
private `sign_at(payload, opts, now)` that `sign_with` calls with the clock, so every case, time-dependent or not, is
compared exactly: the whole token for a result (HS256 and RS256 with PKCS#1 v1.5 are deterministic), the `to_string()`
of the error for an error. Options map one to one: `secret` → `JwtModuleOptions::new`, `privateKey` + `algorithm` →
`private_key` + `algorithm`, `issuer`, `audience`; `expiresIn` and `notBefore` as strings, a number `n` as `"n"` (a bare
number is seconds in Corvis, the documented reading); `subject`, `jwtid`, `notBefore` per call → `subject`, `jwt_id`,
`not_before` of `JwtSignOptions`. `mutatePayload` has no counterpart: Corvis borrows the payload and cannot change it.

**Cases.** Module `B` has `s1` only; `A` adds `expiresIn: "1h"`; `C` adds `expiresIn: "1h"`, `issuer:
"https://auth.example"` and `audience: "web"`; `R` signs RS256 with `rsa` and `expiresIn: "1h"`.

| # | Module | Call | Payload | Pins |
|---|---|---|---|---|
| 1 | B | none | `{sub:"7", role:"admin"}` | `iat` appended at the clock |
| 2 | A | none | same | `exp` = `iat` + 3600, after `iat` |
| 3 | A | none | `{iat:1700000000, sub:"7"}` | `iat` kept in place; `exp` counted from it |
| 4 | A | none | `{iat:0, sub:"7"}` | an `iat` of 0 |
| 5 | A | none | `{iat:1700000000.5, sub:"7"}` | a fractional `iat` and the rounding of `exp` |
| 6 | B | none | `{iat:"yesterday"}` | a non-numeric `iat` |
| 7 | B | none | `{sub:"7", exp:1900000000}` | a payload `exp` without the option |
| 8 | A | none | same | payload `exp` with the module `expiresIn` |
| 9 | B | `{expiresIn:"10m"}` | `{exp:1900000000}` | payload `exp` with a per-call `expiresIn` |
| 10 | B | none | `{exp:"2030"}` | a non-numeric `exp` |
| 11 | A | none | `{exp:null}` | a `null` `exp` with the option |
| 12 | A | `{expiresIn:"10m"}` | `{sub:"7"}` | the per-call option overrides the module's |
| 13 | B + `expiresIn: 3600` | none | `{iat:1700000000}` | a numeric `expiresIn` |
| 14 | C | none | `{sub:"7"}` | `aud` and `iss` appended after `exp`, in that order |
| 15 | C | none | `{sub:"7", iss:"x"}` | payload `iss` with the module `issuer` |
| 16 | C | none | `{sub:"7", aud:"x"}` | payload `aud` with the module `audience` |
| 17 | B | `{subject:"9"}` | `{role:"admin"}` | `sub` appended |
| 18 | B | `{subject:"9"}` | `{sub:"7"}` | payload `sub` with `subject` |
| 19 | B | `{jwtid:"j1"}` | `{sub:"7"}` | `jti` appended |
| 20 | B | `{jwtid:"j1"}` | `{sub:"7", jti:"x"}` | payload `jti` with `jwtid` |
| 21 | B | `{notBefore:"30s"}` | `{sub:"7"}` | `nbf` = `iat` + 30, before `exp` |
| 22 | A | `{notBefore:"30s"}` | `{iat:1700000000}` | `nbf` counted from the payload's `iat` |
| 23 | B | `{notBefore:"30s"}` | `{nbf:1700000000}` | payload `nbf` with `notBefore` |
| 24 | B | none | `{nbf:"soon"}` | a non-numeric `nbf` |
| 25 | C | `{subject:"9", jwtid:"j1", notBefore:60}` | `{iat:1700000000, role:"admin"}` | the full order `iat`, `nbf`, `exp`, `aud`, `iss`, `sub`, `jti` |
| 26 | C | none | `{iss:"x", exp:1900000000}` | which of two conflicts is reported |
| 27 | B | none | `"hello"` | a string payload: header and payload segment |
| 28 | A | none | `"hello"` | a string payload with a sign option |
| 29 | B | none | `[1, 2]` | an array payload |
| 30 | B | none | `5` | a number payload |
| 31 | B | `{mutatePayload:true}` | `{sub:"7"}` | the caller's object changed by the original: divergence (Corvis borrows the payload) |
| 32 | B | none | `{sub:"7"}` | `payloadAfter` equals the payload (copied by default) |
| 33 | R | none | `{iat:1700000000, sub:"7"}` | RS256 header and signature through the cached key (Decision 3) |
| 34 | B | none | `{iat:1700000000, name:"Zoë \"x\"", n:0.1, big:9007199254740991, nested:{a:[1,"é"]}}` | the payload text: string escapes, non-ASCII, numbers, nesting |

**Rules expected**, read in `jsonwebtoken/sign.js` and the JWT module's `jwt.service.js` to design the cases; where the
recording disagrees, the recording wins and this list is corrected in "Implementation notes": the payload is checked
first (`iat`, `exp`, `nbf` must be numbers; a non-object payload other than a string is refused); a payload `exp` with an
`expiresIn` and a payload `nbf` with a `notBefore` are errors; `iat` is the payload's when it is truthy, the clock
otherwise; `nbf` and `exp` are counted from it (`floor(iat + ms / 1000)` for a timespan, `iat + n` for a number); then
`aud`, `iss`, `sub`, `jti` are appended from `audience`, `issuer`, `subject`, `jwtid`, each an error when the payload
has the claim; a string payload has no `typ` in its header and is signed as it is, and the JWT module refuses it when
any sign option is set. Corvis copies the original's error texts.

**Implementation.** `sign_at` builds the claims in the original's order over a copy of the payload's map
(`preserve_order` is on for `serde_json` in the workspace, `Cargo.toml:31`), with the same checks in the same order, and
writes the header by hand as today (with `typ` only for an object payload). `JwtSignOptions` gains `not_before:
Option<String>` and `jwt_id: Option<String>`; `JwtModuleOptions` keeps `issuer`, `audience` and `expires_in`, which act
as the original's module `signOptions` (and, for `issuer`/`audience`, as its `verifyOptions` too, as today).

Alternatives: fix only the four rules the issue names (leaves the claim order, `iat: 0`, the type checks and string
payloads unverified); record a divergence for the conflicts and keep overwriting (the original refuses them because an
option and a payload claim that disagree are a bug of the caller, and a silent winner hides it).

### 3. JWT keys and validations prepared once

**`JwtService`.** `new` prepares, behind an `Arc` so `Clone` stays cheap: the module encoding key (module secret when
non-empty, else module private key) as `Option<Result<EncodingKey, jsonwebtoken::errors::Error>>`, the decoding key
(module secret, else public key) the same way, and the `Validation` of `verify` (the configured algorithm, no required
claims, leeway 0, `exp`/`nbf`/`aud` off, as at `jwt.rs:584-592`). The order of key choice is unchanged: a per-call secret
or private key is still taken first and parsed for that call (an HMAC secret is a copy). Errors are the same values:
`jsonwebtoken::errors::Error` is `Clone`, so a cached failure is returned as a clone and `downcast_ref` keeps working;
"secretOrPrivateKey must have a value" and "secret or public key must be provided" keep coming from the call. Nothing
fails at startup: the original parses its keys on every call and an application with a bad PEM starts and fails on the
first `sign`; failing earlier would change when such an application breaks (rejected, Review note 7). `JwtService` gets no
`Debug` (neither key type has one, and `JwtModuleOptions` has the safe one).

**`JwtKey`.** It keeps its material and gains `parsed: Arc<OnceLock<Result<DecodingKey, String>>>`; `decoding_key()`
parses once and every clone shares the result, so a provider that returns a clone of a cached `JwtKey` costs no parse. A
provider that builds a new `JwtKey` per request still pays one parse per request; the rustdoc of `JwtKeyProvider`
(`key.rs:128-131`, "cache what it fetches") says to cache the `JwtKey`, not the PEM. `check_secret` still runs per request
(it reads a length). The hand-written `Debug` is unchanged.

**The strategy.** `Resolved` holds the `Validation` of a static key, built when the strategy is built, and for a key
provider one `Option<Validation>` per key family from `allowed_for(family, algorithms)` (`None` when the list is empty,
which answers "invalid algorithm" as today). `verify_claims` stops calling `check_header`: it becomes
`decode_claims(token, key, validation, rules)`, and both callers run `check_header` first: the strategy where it does today
(after the length and the three-segment checks, before the key provider), `JwtService::verify` before `decode_claims`, so
`verify` still refuses `crit` and a malformed header.

**What stays per call.** `jsonwebtoken` 9.3.1 parses an RSA private key's DER on every sign (`RsaKeyPair::from_der`,
`crypto/rsa.rs:41`), an EC key's PKCS#8 on every sign (`crypto/ecdsa.rs:35`), and a public key through
`UnparsedPublicKey` on every verify. Keeping `ring` key pairs would mean signing and verifying with `ring` directly: a new
direct dependency on crypto code the project does not write today, against the dependency policy of #46. Rejected; the
benchmark records what is left.

Alternatives: parse lazily on first use (same cost after the first call, but the first request of each instance pays and
the error moves from construction-time knowledge to a race on a `OnceLock`); a startup warning for a key that does not
parse (useful, but a behaviour of its own; possible follow-up).

### 4. Size and depth of a session checked without a buffer

A private `Measure { written, limit }` implements `std::io::Write`, adds each chunk to `written` and returns an error once
`written > limit`; `fits(record, limit)` is `serde_json::to_writer(&mut Measure, record).is_ok()`. It replaces the
`to_vec` at `manager.rs:163` (load) and `manager.rs:197` (`check_writable`). Serialization stops one chunk past the
limit, so an oversized record costs at most the limit, and the verdict is the same: a record of exactly
`max_session_bytes` passes, one byte more does not. `too_deep` stays: it allocates nothing and stops at the limit.

Alternative: count brackets in the writer to get the depth in the same pass (the writer would have to track JSON string
and escape state, re-doing the serializer's work for a walk that is already cheap).

### 5. Change detection by a snapshot on the first write

`State::baseline` becomes `Option<Map<String, Value>>`, `None` when the state is created or loaded. `data` becomes private
to `session.rs`, read through `State::data()` and written only through `State::data_mut()`, which stores
`Some(data.clone())` the first time it is called. `modified()` is `id_changed || baseline.as_ref().is_some_and(|b| b !=
&data)`. Every write path goes through it: `Session::set`, `remove`, `clear`, `regenerate`, `destroy`, the login and logout
of `session/login.rs`, and `remove_passport_user` of `session/middleware.rs`. A request that only reads its session clones
nothing and compares nothing; one that writes pays the clone it pays today, once. The rule the express-session fixture
pins is unchanged: a value set to what it already was is not a modification.

Alternative: a digest of the serialized data at load, as `express-session` does with a CRC (one serialization pass per
load, and a collision would lose a save).

### 6. An unchanged session is touched with its timestamps only

`State::touch_record(options, now)` builds `{ "cookie": .., "__corvis": .. }`, the two keys `touch` may write
(`store.rs:75-78`, spec `auth/sessions` "Session store contract"). The unchanged branch of `commit` (`manager.rs:342-343`)
passes it instead of the full record. `MemorySessionStore` needs no change (`touch_fragment` keeps the same two keys). The
contract's rustdoc and the store table of `docs/http/session.md` say that the record given to `touch` holds only those
keys. A custom store that read the data in `touch`, which the contract already forbids, finds none.

Alternative: a new trait method with a default (two methods for one operation, and every store keeps implementing the old
one).

### 7. The identifier of a new session is drawn when it is first needed

`State::id` becomes `Option<String>`: `None` for a fresh state, for a regenerated one and after `destroy`. It is drawn by
`Session::id()` (and by the OAuth2 binding through it), by `commit` when it writes a new record, and nowhere else; the
retry after `SessionStoreError::Exists` draws again as today. A failed draw sets `failed` as `State::fresh` does today:
`commit` refuses to write ("the random generator failed") and `Session::id()` returns the empty string, as
`unwrap_or_default` does today. `regenerate` queues the old identifier for destruction exactly as before (it only exists
for a persisted state, which always has one).

### 8. The memory store purges in batches

In the room loop of `MemorySessionStore::write` (`memory.rs:389-397`) `purge(now, usize::MAX)` becomes
`purge(now, PURGE_BATCH)`. The loop already continues while room is short, so expired records still go before any live
record is evicted (the eviction order is unchanged), but a write removes only the expired records it needs plus at most
one partial batch, instead of every expired record in the store. The others go with later writes (64 at the start of
each) and on `load`, as `memory.rs:21-22` and the struct's rustdoc already say.

### 9. One way to call a validator and one `503`

`auth::outcome` (a private module) gets:

- `service_unavailable() -> HttpException`: `HttpException::new((), StatusCode::SERVICE_UNAVAILABLE)`, the body
  `{"statusCode":503,"message":"Service Unavailable"}`. Every `503` the module builds uses it: the deadlines of the JWT key
  provider, the JWT, local and API key validators (unchanged), `PasswordError::Busy` in `into_http_exception` and the three
  "busy" answers of the OAuth2 strategy (key order changes). The status-first form is the one rule that covers every
  status: the API key strategy must answer the plain body of any status of 500 or more (`api_key/strategy.rs:316-325`,
  `HttpException::new((), e.status())`), and `docs/recipes/passport.md:654` documents it.
- `with_deadline(timeout, future, warn) -> Result<T, StrategyOutcome>`: the deadline, the rate-limited warning of the
  strategy and the `503`.
- `user_outcome(result, info) -> StrategyOutcome`: `Err(e)` → `Error(e)`, `Ok(None)` → `Fail` without challenge,
  `Ok(Some(user))` → `AuthenticatedUser::new` → `Success` or `Error`.

The JWT, local and API key strategies use both (the API key strategy passes its error through `uniform` first). The OAuth2
strategy keeps its own call: it has no deadline, answers `Ok(None)` with `refused()` (a redirect to the failure page when
one is configured), treats an empty user apart and stores `return_to`; folding that into the helper would add branches
only it uses.

Alternative: `ServiceUnavailableException::empty()` everywhere (changes the documented API key body and leaves `uniform`
with another form for 502 or 504).

### 10. One limiter and one suffix

The hasher uses `auth::warn_limit::WarnLimiter` (its reasons are five fixed strings, well under the cap of 64).
`WarnLimiter` gains `line(&self, reason, now, message: impl FnOnce() -> String) -> Option<String>`, which builds the
message only when it will be logged and appends ` ({n} more since the last warning)` when `n > 0`. The eight copies and
the `more` helper of `authenticator.rs` use it. The hasher's and the OAuth2 strategy's suffixes take the common words; the
tests that pin those texts change with them. Log lines are not an API.

### 11. One copy of each parser and rule

- **Untrusted JSON.** `crates/corvis/src/untrusted_json.rs` (compiled with `any(feature = "jwt", feature =
  "auth-oauth2")`) holds `PlainJson`, `parse` and `parse_object`; `jwt.rs` and `oauth2/json.rs` use it. `HeaderShape`,
  `Discard` and `IsCrit` stay in `jwt.rs`, the only user.
- **`Authorization` header.** `crates/corvis/src/auth/auth_header.rs`: one byte-level tokenizer for
  `/(\S+)\s+(\S+)/` at its first match over the first header value, with JavaScript whitespace in the latin1 range and an
  optional scan window. The JWT extractor uses it without a window (passport-jwt parity, `jwt.json`); the API key
  `bearer()` source with its window of `max + 128` and keeps its own length and control-character checks (`api_key.json`,
  and the documented difference past 640 bytes of leading whitespace). `extract_bearer_token` (`jwt.rs:631-636`) keeps
  its rules: it is public, applications call it, and its rules (split at the first space, ASCII-case-insensitive scheme,
  trimmed token, visible-ASCII header) differ on inputs such as `Bearer a b`; its rustdoc says it is not the strategy's
  parser and points to `ExtractJwt`.
- **Empty user.** `is_empty_json` moves to `auth/strategy.rs`; `AuthenticatedUser::is_empty` and the session use it.
- **Cookie prefixes.** `http::res::has_cookie_prefix(name, prefix)`, ignoring ASCII case as browsers do, replaces the
  four checks. The JWT cookie extractor's startup warning no longer fires for `__HOST-x` or `__host-x`, which browsers
  treat as prefixed.
- **Cookie names.** The two rules differ on purpose: `http::res::valid_cookie_name` is what `serialize_cookie` accepts
  (cookie 0.7, the parity of `res.cookie`, also used for the session cookie name), the OAuth2 one is the RFC 6265 token of
  at most 128 bytes for a configured name. The OAuth2 function is renamed `is_token_cookie_name`, with a comment on why;
  no rule changes.

### 12. A MAC primitive in `signed_cookie.rs`

`CookieKeys::mac(&self, input: &str) -> [u8; 32]` (crate-private): HMAC-SHA256 of `input` with the first secret. The
three derivations of `oauth2/state.rs` become `SHA256(base64_standard_no_pad(mac(input)))`, which is what they compute
today (the text after the last `.` of `sign(input)` is that base64), so flows started before the upgrade complete after
it. The task records the outputs of the current code for fixed inputs before the change and the new test compares them.

`CookieKeys` keeps one keyed `Hmac<Sha256>` per secret, built in `new`, and clones it for each signature instead of keying
again. `verify` still computes every secret's signature without stopping, compares in constant time and refuses input over
`MAX_SIGNED_BYTES` before any HMAC; the hand-written `Debug` still prints the number of secrets only.

### 13. Smaller fixes

a. **Repeated state cookie.** `StateCookie::read` uses `req.cookie_values(name, 2, limit)` (compiled whenever
   `auth-oauth2` is, through `auth-session`) and reads no flow unless exactly one cookie has the name, as the session and
   the JWT cookie extractor do. A callback that carries two is refused as one without a state; a login start begins from an
   empty set.

b. **API key redaction.** The per-request `redact_query_param` loop runs only when the request has no application
   redaction (`Request::app_redaction`) that holds the strategy's names, which is the case of a strategy used outside an
   application (the guard tests the comment at `api_key/strategy.rs:338-339` mentions). Inside an application nothing is
   written and `redacted_url()` hides the same values.

c. **Poisoned locks.** `Request::insert`, `get` and `remove` recover a poisoned lock (`unwrap_or_else(|e| e.into_inner())`)
   like the rest of `Request`, so `RejectUnlessDone::drop` cannot panic while a panic unwinds.

d. **The busy-hasher test.** `PasswordHasher::hold_slot(&self) -> Result<HeldSlot, PasswordError>`, `#[doc(hidden)]`:
   takes a free slot or fails with `Busy`, and frees it when `HeldSlot` is dropped. The test holds the only slot, sends both
   logins, checks the two `503`s and drops it: no thread, no slow hash, no wait. Alternative: a hidden slot count and a
   loop until the holder thread has it (no sleep, but the holder's hash must still outlast both requests, which is timing
   again).

e. **The typed user.** The code stays as it is (Review note 1). The rustdoc of `User<T>` (`user.rs:12-15`), of
   `AuthUserSlot` (`strategy.rs:117`) and of `Request::set_user` (`http/request.rs:285-288`) say that the slot holds what
   an `AuthGuard` stored or what the session middleware restored (or `log_in` stored), and that `User<T>` on a route
   without an `AuthGuard` returns the session user. `docs/recipes/passport.md:193,199` (English and Spanish) say the same
   and link the threat model's section. New `Request::typed_user::<T: Clone + 'static>() -> Option<T>`: a clone of the
   value in that slot, never another extension, for guards (`User<T>` is a handler parameter).

f. **Enums and equality.** `auth/mod.rs`: `enum GuardKind { Auth, Throttler }` instead of `Vec<bool>`; the OAuth2
   strategy's `note(.., level_warn: bool, ..)` and `fail(.., callback: bool)` take `enum NoteLevel { Warn, Debug }` and
   `enum Phase { Start, Callback }`. `OAuth2Token` keeps `PartialEq + Eq` with a hand-written constant-time comparison
   (`subtle`, already imported in `secret.rs`), so code that compares tokens compiles and no comparison leaks timing.

g. **`AuthGuard` names.** `Authenticator::default_names(&self) -> Option<&[String]>` (crate-private) lets
   `AuthGuard::names` borrow the default list. The `Authenticator` lookup through `ctx.module_ref()` stays per request: an
   `AuthGuard` value can serve more than one application (the tests build several from the same guard), so caching the
   first one it met would be wrong, and the lookup is one map access and an `Arc` clone.

h. **Comments and tests.** The doc comment at `oauth2/options.rs:732-734` moves to `state_cookie_name`. The test
   `every_outcome_can_be_built` (`strategy.rs:226-236`) matches each outcome it builds against its variant. The test files
   take the names of what they test: `auth_session_review.rs` → `auth_session_races.rs`, `auth_session_review2.rs` →
   `auth_session_guard_login.rs`, `auth_oauth2_review.rs` → `auth_oauth2_abuse.rs`; their header comments keep the review
   they come from, and the threat model's links change in both languages (`tests/auth_source_checks.rs` fails on a link to
   a missing file).

i. **OAuth2 client.** The rustdoc of `OAuth2Client` says each call opens its own TLS connection (`connection: close`,
   `client.rs:306`). The warning of a failed `refresh` (`client.rs:645-647`) goes through a `WarnLimiter`, one a minute.
   `login_tracker` keeps its documented panic (finding N6 dropped).

### 14. The example

`examples/auth-app/src/auth/jwt.rs:45`: "The module's `expires_in` (`app_module.rs`, 15 minutes) sets `exp`."
`RolesGuard` reads `request.typed_user::<Account>()`. The example's tests do not change.

### Behavior visible to applications and clients

1. `JwtService::sign`, as recorded: a payload claim together with its option (`exp`/`expires_in`, `nbf`/`not_before`,
   `aud`/`audience`, `iss`/`issuer`, `sub`/`subject`, `jti`/`jwt_id`) is an error with the original's text, where Corvis
   overwrote `exp` and `sub` and let the payload win for `iss` and `aud`; `exp` (and `nbf`) are counted from a payload
   `iat`; whatever else the recording settles (an `iat` of 0, non-numeric time claims, the claim order, string and
   non-object payloads). A token signed with the module's `issuer` and `audience` changes bytes (`aud` now precedes `iss`).
   An application that relied on an overwrite gets an error from `sign`.
2. New `JwtSignOptions` fields `not_before` and `jwt_id`; new `Request::typed_user`; hidden `PasswordHasher::hold_slot`.
3. The `503` of `PasswordError::into_http_exception` and of the OAuth2 strategy's busy answers is
   `{"statusCode":503,"message":"Service Unavailable"}` instead of `{"message":"Service Unavailable","statusCode":503}`.
4. An OAuth2 callback that carries two state cookies of the strategy's name is refused.
5. A custom `SessionStore::touch` receives a record with `cookie` and `__corvis` only.
6. A JWT cookie named with the `__Host-` prefix in another letter case no longer logs the startup warning.
7. The suffix of the hasher's and the OAuth2 strategy's rate-limited log lines changes words.
8. `Request::insert`, `get` and `remove` no longer panic after a panic poisoned the request's extensions.

Nothing else is visible: session saving rules, cookies, identifiers' shape, eviction order and every strategy's
responses stay as the fixtures record them.

## Risks / Trade-offs

- **`sign` turns overwrites into errors.** consi and facten sign a payload with `sub` and no time, issuer, audience or
  token-id claim, with a module `expires_in` and no per-call options, so none of the new errors applies to them. The
  corvis-parity check decides: if it shows a difference in a behaviour a port relies on, that behaviour is kept, the case
  in `jwt_sign.json` gets a `divergence` with the reason, and design.md records it. The tokens themselves are not compared
  by apidiff (it maps tokens), so the check catches errors and status changes, not byte changes.
- **The fixture may miss a branch of the original.** The case list follows every branch of `sign.js` and of the module's
  `sign`; a branch found later is a new case, recorded the same way.
- **A write path that skips `data_mut`** would make a change go unsaved. `data` is private to `session.rs`, so the compiler
  finds every writer; the express-session saving fixture and the race suites run unchanged.
- **The identifier drawn later** could be read before it exists by code that reaches into the state; the field is private
  and read through `Session::id()`, which draws.
- **The derived OAuth2 values must not change**: fixed vectors taken before the change.
- **Prepared HMAC keys** hold two hash states per secret (at most 8 secrets); `CookieKeys` is cloned through an `Arc` as
  today.
- **Benchmark noise**: the throughput numbers inform and are not asserted; the deterministic counters and the allocation
  bounds are the acceptance.
- **The `503` key order** changes for two causes; clients that parse JSON see nothing, byte comparisons in tests change.

## Open questions

1. `User<T>` on a route without an `AuthGuard` returns the session user while the rustdoc says it answers `401`. Change the
   code (the session restores only the JSON user, and `User<T>` reflects only an `AuthGuard`) or the docs?
2. Add `not_before` and `jwt_id` to `JwtSignOptions`, or record the `notBefore` and `jwtid` cases as divergences ("no such
   option")?
3. Which `503` body for the module: status first or message first?
4. Where the benchmark lives: a new `bench/auth/` or scenarios of `bench/run.sh`?
5. The busy-hasher test: a hidden `hold_slot` or a hidden slot count?
6. Should `extract_bearer_token` adopt the strategy's parser?
7. Should `JwtService` fail at startup for a key that does not parse, as the issue suggests ("parse at construction and
   fail at startup")?
8. String and other non-object payloads of `sign`: match the original or refuse them?

## Review notes

These settle the open questions and override the design where they differ. Notes 1, 2 and 3 change what applications
observe or add public API and are listed for the maintainer to confirm.

1. **Keep the behaviour, fix the docs (to confirm).** The session middleware keeps storing the typed user, so `User<T>`
   and the new `Request::typed_user` see the session user on a route without an `AuthGuard`; the rustdoc and the passport
   recipe are corrected. Reasons: it is what applications observe today and what the security guidance page
   (`docs/security/authentication-security.md:134-140`), the threat model (`authentication-threat-model.md:224-240`),
   `docs/http/session.md:483` and the spec requirement "Routes without an AuthGuard are documented as seeing the session
   user" (`auth/security-invariants`) already state; the review of #46 decided it with its reasons (it adds no authority,
   parity with `passport.session()` setting `req.user` everywhere, and the stricter rule does not remove the real risk); and
   changing it would make `User<T>` and `Request::user()` disagree on the same route (the JSON user would stay), which is
   what the typed slot was built to prevent. No safety reason calls for the change: every `AuthGuard` replaces or clears
   the restored user.
2. **Add the two options (to confirm).** `not_before` and `jwt_id` are added to `JwtSignOptions`, so the recorded rules for
   `notBefore` and `jwtid` are reproduced instead of marked; a migrated application that uses them has a direct
   counterpart. They are new public fields: code that builds `JwtSignOptions` with a struct literal and no
   `..Default::default()` must add them (every use in the repository and in the port apps uses `..Default::default()` or
   none).
3. **Status-first `503` (to confirm).** One body, `{"statusCode":503,"message":"Service Unavailable"}`, for every `503` the
   module builds, because the API key strategy's documented plain bodies use that form for every status (Decision 9).
4. **`bench/auth/`**, a Corvis-only benchmark, as Decision 1 describes; the synthetic benchmark and its spec are untouched.
5. **`hold_slot`**, hidden: it removes the timing assumption altogether.
6. **`extract_bearer_token` keeps its rules** and says so in its rustdoc; changing a public helper's parsing is not a
   quality fix.
7. **No startup failure for `JwtService` keys.** The keys are prepared in `new` and a failure is returned by each call, as
   the original fails per call; a startup warning for an unparsable key is a possible follow-up.
8. **String and non-object payloads follow the recording**: Corvis matches what the original returns or throws for cases
   27 to 30, unless matching one would need a payload type Corvis cannot express, which is then a recorded divergence.

## Measurements

Filled by tasks 1.4 and 10.1: `bash bench/auth/run.sh --label before` on `dev` before any framework edit, `--label after`
on the finished branch, same machine, same `CONNS`, `DURATION` and rate.

Machine: Intel Core i9-14900KF, Windows 11 Pro 10.0.26300, rustc 1.99.0, oha 1.16.0. Parameters: `CONNS=50`,
`DURATION=10`, `WARMUP=3`, `RATE=1000`, `OHA_THREADS=4` (the defaults of `run.sh`). Before: `d46758a` (the `dev` this
change starts from, with only `bench/auth/` added), 2026-11-11.

| Scenario | req/s before | req/s after | p99 before | p99 after | allocations / request before → after | bytes / request before → after |
|---|---|---|---|---|---|---|
| `jwt-hs256` | 67345 | | 1.519 ms | | 93 → | 24776 → |
| `jwt-provider-rs256` | 64499 | | 2.16 ms | | 125 → | 31495 → |
| `service-verify-rs256` | 72429 | | 1.719 ms | | 107 → | 29330 → |
| `service-sign-hs256` | 67799 | | 1.192 ms | | 81 → | 24515 → |
| `service-sign-rs256` | 13538 | | 6.355 ms | | 135 → | 41580 → |
| `session-none` | 70323 | | 1.075 ms | | 54 → | 22035 → |
| `session-read` | 26258 | | 4.477 ms | | 4665 → | 573709 → |
| `session-write` | 23175 | | 4.594 ms | | 4642 → | 651137 → |

## Implementation notes

### Findings re-checked (task 1.1)

Every line of "Findings as checked" and of the invariants table was read again on `d46758a` (the `dev` this change
starts from, two commits after `db5b4f6`; neither touched the authentication module). The facts hold; the lines that
moved by a few:

- M1, S1: unchanged lines in `jwt.rs`.
- S6: the local strategy's validator call is `local/strategy.rs:354-378`; `login_tracker`'s panic (N6) is at
  `local/strategy.rs:214` with its `# Panics` section at 207-210.
- S11: the application registers the names at `application.rs:558` (not 556); the request keeps them as
  `Request::app_redaction` (`http/request.rs:111`).
- N6: the refresh warning is at `oauth2/client.rs:646`.
- Everything else is at the lines given.

`cargo test -p corvis --features "auth-local auth-api-key auth-session auth-oauth2"` passed before any edit: 1598
passed, 0 failed.

### The benchmark (tasks 1.2 to 1.4)

- The server counts allocations in 64 shards (one per thread, each on its own cache lines) summed by
  `/__bench/stats`. A first version with one pair of global counters made `session-read` and `session-write` measure
  the counters (4.7k req/s, against 26k with the shards: about 4 600 allocations per request on 32 threads contend on
  one cache line); the "before" numbers above come from the sharded version, run before any framework edit.
- The latency run uses the same `CONNS` as the closed loop run, at `RATE` requests per second.
- The seeded session holds 128 small objects (about 16 KiB of JSON) rather than one long string, so its parse, its
  copies and its comparison cost what a real session's do.

### The `sign` recording (tasks 2.1 to 2.5)

- The fixture package pins `@nestjs/jwt` 11.0.2 and `jsonwebtoken` 9.0.3. The JWT module declares a peer dependency on
  `@nestjs/common` up to 11 while the package uses 12.4.0 (the version the other recorders need), so `package.json`
  carries an `overrides` entry that satisfies the peer with the package's own `@nestjs/common`; `npm ci` installs it as
  it is. `jsonwebtoken` was already 9.0.3 through `passport-jwt`, so the lockfile only adds the JWT module and its types,
  and `node jwt.mjs` writes a `jwt.json` identical to the committed one.
- Two runs of `node jwt_sign.mjs` write identical files.
- Cases added to the 34 of the table, for branches of the original the table does not cover (35 to 40): a string payload
  with an asymmetric module (the module's `algorithm` counts as a sign option), a number payload with a sign option, a
  boolean payload, a `null` payload, a negative payload `iat`, and a payload claim set to `null` together with its option.
- Recording against "Rules expected": one difference. A non-object payload other than a string is not always refused:
  an array or `null` is (`Expected "payload" to be a plain object.`), but a number or a boolean is signed as its JSON
  text with a header without `typ`, unless an option that only applies to objects is set (`invalid expiresIn option for
  number payload`). Everything else holds as written: the checks and their order, `iat` truthy or the clock (an `iat` of
  0 is replaced in place by the clock), `floor(iat + ms / 1000)`, the conflicts with the original's texts (two forms:
  `Bad "options.expiresIn" option the payload already has an "exp" property.` for `exp`/`nbf`, `Bad "options.issuer"
  option. The payload already has an "iss" property.` for the four appended claims), the order `iat`, `nbf`, `exp`,
  `aud`, `iss`, `sub`, `jti`, and `exp` reported before `iss` when both conflict (case 26).
- The replay on the signing code of `dev` failed 32 of the 40 cases: every case with a payload `iat`, every conflict,
  every non-numeric time claim, `aud`/`iss` order, `jti`, `nbf`, every non-object payload and RS256 (whose payload has an
  `iat`). It passed 1, 2, 7, 12, 17, 31, 32 and 34.
- After the change every case matches; the only divergence is case 31 (`mutatePayload`), whose token is the same and
  whose changed caller object Corvis cannot reproduce (it borrows the payload).
- The string-payload check lists the sign options as the module merges them: the module's (`algorithm` when it is not
  HS256, `expiresIn`, `issuer`, `audience`), then the call's (`expiresIn` when the module has none, `subject`, `jwtid`,
  `notBefore`). Corvis cannot tell an explicit `algorithm: HS256` from the default, so only another algorithm counts.
- `sign_at` builds the claims over the payload's map in the original's order of checks; `JwtSignOptions` gained
  `not_before` and `jwt_id`, with `Debug` listing both.
- Every caller of `sign` and `sign_with` in the repository (`examples/`, `crates/corvis/tests/`,
  `crates/corvis/src/fuzz_tests.rs`, `bench/auth/`) signs a payload without `iat`, `exp`, `nbf`, `iss`, `aud`, `sub` or
  `jti` claims that meet their option.

### Keys and validations prepared once (group 3)

- `JwtService::new` parses the module encoding key and decoding key and builds the `Validation` of `verify` behind an
  `Arc`. The per-call keys keep their place in the order of choice (per-call secret, module secret, per-call private
  key, module private key) and are parsed for their call. A cached parse error is returned as a clone of the
  `jsonwebtoken::errors::Error`; "secretOrPrivateKey must have a value" and "secret or public key must be provided" come
  from the call, as before.
- `verify_claims` became `decode_claims` (no header check); `JwtService::verify` checks the header right after choosing
  the key, where `verify_claims` did, so the order of its errors is unchanged.
- The strategy's `Resolved` lost its `algorithms` field: a static key holds its `Validation`, a provider one
  `Option<Validation>` per key family (`FamilyValidations`). The `Validation` builder moved to `jwt.rs`
  (`signature_validation`) and serves both.
- `JwtKey::decoding_key` now returns a reference into the shared `OnceLock`, so neither the strategy nor a provider's
  clone copies the parsed key; the static key of the strategy clones it once at startup.
- Counters: `KEY_PARSES` (`jwt.rs` and `auth/jwt/key.rs`), `HEADER_CHECKS` (`jwt.rs`), `VALIDATION_BUILDS`
  (`auth/jwt/options.rs`), all `#[cfg(test)]` thread-locals. The strategy's unit tests are in `auth/jwt/tests.rs` and
  drive `JwtStrategy::authenticate` directly with an `ExecutionContext`.

### Sessions (group 4)

- `State::id` is `Option<String>` and private, with `ensure_id` (draws when missing), `current_id` and `forget_id`;
  `Session::regenerate`, `destroy` and the logins leave it `None`. `data` and `baseline` are private; `data_mut` takes
  the baseline on the first write. `Session::remove` and `clear` write only when there is something to remove, and
  `remove_passport_user` only when there is a user, so a read path never copies the data.
- The draw counter (`ID_DRAWS`) and a switch that makes draws fail (`FAIL_DRAWS`) are `#[cfg(test)]` thread-locals in
  `session.rs`; the design's "session unit tests of the generator failure" did not exist, so
  `a_failed_draw_at_commit_writes_nothing` is the first. The manager's unit tests are in `session/manager_tests.rs`.
- `tests/auth_hot_path_heap.rs` measures, with a thread-local counting allocator, a request to a session route minus
  the same request to a route outside the cookie path, minus what the store's own `load` and `touch` allocate (measured
  inside a wrapping store). Before (the session files of `dev`): 359 395 bytes for a 16 KiB session (16 592 bytes of
  record). After: 3 287 bytes for 16 KiB and for 64 KiB alike. The bound is 4 KiB, as the spec says. The store's
  `touch` is taken out with its `load`: what it allocates (the small `cookie`/`__corvis` fragment) is the store's work.
- Record-sized buffers (allocations of at least the record's length) of one `update` request: before 4, of which 2 were
  the middleware's measuring buffer; after 2, both the store's (its text and the `Arc<str>` copy of it). The test
  asserts none outside the store. Both assertions fail on the session files of `dev`.
- `one_write_purges_a_bounded_number_of_expired_records` covers the spec scenario (count bound: 9 936 of 10 000 expired
  records left) and a byte-bound case that reaches the room loop (a record 150 times the size of the expired ones): on
  the store of `dev` that write dropped all 2 000 expired records, now at most the ~152 it needs plus two batches. The
  count-bound scenario of the spec cannot reach the loop: after the first batch the store is always below
  `max_sessions`, so it passes on the old code too and is kept as written.

### Consolidation (group 5) and fixes (group 6)

- `WarnLimiter::line` builds the message only when it is written. The hasher's own limiter is gone; its keys were
  `&'static str`, the shared one takes `&str` (its five reasons stay far under the cap of 64).
- `auth::outcome` holds `service_unavailable` (compiled with any of `jwt`, `auth-local`, `auth-api-key`,
  `auth-oauth2`) and `with_deadline` / `user_outcome` (with any of the first three, the only users), so no feature
  combination leaves dead code. The JWT key provider's deadline keeps its own `timeout` (it maps to `Denied`, not to an
  outcome) and builds its `503` with `service_unavailable`.
- `every_framework_503_has_the_same_bytes` (`tests/auth_local.rs`) compares the text of the busy hasher's and the slow
  validator's `503` on one route; the OAuth2 strategy at its limit is compared with the same text in
  `too_many_requests_in_flight_answer_503_without_contacting_the_provider` (`tests/auth_oauth2.rs`), where the mock
  provider that holds the slot already lives. The JWT, local and API key deadlines already used this body
  (`tests/auth_callback_timeouts.rs`, `tests/auth_api_key_options.rs`).
- `untrusted_json.rs` is compiled with `jwt` or `auth-oauth2`; `parse_object` only with `auth-oauth2`, its only user.
- `auth::auth_header` parses the scheme and finds where the token starts; the token is read by `Credentials::token`,
  with or without a length limit, so the API key source still refuses an over-long key while scanning it. One edge of
  the API key source changes: a value whose leading space and scheme reach exactly the end of the `max + 128` window
  with `bearer` and continue with a non-space byte (the scheme cut by the window) was read as a key starting there, with
  no separator; it is now no key, which is what `passport-jwt`'s expression gives (no separator, no second group). The
  scheme length cap of 17 bytes of the old scanner is gone: the window bounds the scan, and a scheme longer than six
  bytes was never `bearer`. The `jwt.json` and `api_key.json` replays and the extractor tests pass unchanged.
- `has_cookie_prefix` is compiled with `jwt` or `auth-session`.
- `derivations_are_unchanged` holds the outputs of `derive_verifier`, `session_binding` (a session loaded under a fixed
  identifier) and `user_binding` (keyed and plain) recorded from the code before the change; they match after it.
- `CookieKeys` keeps one keyed `Hmac<Sha256>` per secret (no copy of the secret otherwise); `mac` is compiled with
  `auth-oauth2`, its only user. `HMAC_RUNS` still counts one run per secret.
- `StateCookie::read` decodes at most two values of the name, each up to three times `MAX_SIGNED_BYTES` raw, as the
  session does; `a_state_cookie_sent_twice_is_no_state` (`tests/auth_oauth2_review.rs`) plants the second cookie
  before and after the real one and sends the real one twice.
- The busy-hasher test holds the slot with `hold_slot` (`HeldSlot` frees it on drop) and shares the slow hasher with
  `every_framework_503_has_the_same_bytes`, so the two take turns through a `tokio::sync::Mutex`; 20 runs in a row
  passed.
- `Request::typed_user` reads `AuthUserSlot` only. Its integration tests are in `tests/auth_session_http.rs`, where a
  session user exists for "Route without an AuthGuard"; "A guard reads the typed user" uses the harness's `header`
  strategy instead of `jwt` (the slot is the same whatever strategy fills it), and "Foreign extension" a guard that
  inserts an `Account`, with a unit test of the same rule in `http/request.rs`.
- The API key strategy's mark moved to `Resolved::mark_query_names`, skipped for a name the application's redaction
  holds (`Request::app_redacts`, compiled with `auth-api-key`).
- Poisoned locks: besides `insert`, `get` and `remove`, `RequestRejected::is_marked` reads a poisoned lock too.
  Otherwise the mark the router writes while a panic unwinds (now possible through the tolerant `insert`) would be
  invisible to the session middleware, which keeps a guard login unless it sees the mark or an `Err`. With the old
  `insert`, `a_handler_that_panics_with_a_poisoned_lock_is_a_500` aborts the test process (exit 0xc0000409, a panic
  while panicking); now it is a 500 with no login.

### Nice to have (group 7)

- `GuardKind`, `NoteLevel` and `Phase` as designed; `OAuth2Token` compares with `ct_eq`.
- `AuthGuard::names` returns `&[String]` (both kinds of list borrowed) through `Authenticator::default_names`; the
  public `default_strategies` keeps returning a `Vec`.
- `OAuth2Client` keeps a `WarnLimiter`; `a_failed_refresh_logs_one_line_a_minute` tests the line it writes.
