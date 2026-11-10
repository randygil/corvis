# Tasks

Prerequisite: #136 (`e3-strategy-core-and-guard-responses`) is merged into `dev` (and, by the epic's order, #42, #41
and #45). Before starting, compare this change with the API that landed (`corvis::auth`, `Strategy`,
`AuthenticatedUser` and its JSON form, `Authenticator`, `AuthGuard` and its startup check, `Provider::strategy`, the
fixture package, `subtle` if #45 added it) and update the artifacts if it differs; record the adjustments under
"Implementation notes" in `design.md`. The maintainer's answer to Open question 1 (Redis and ORM stores) decides
whether new issues are created before this change starts.

## 1. Feature and dependencies

- [x] 1.1 Add the optional `hmac = "0.12"` and `subtle = "2"` dependencies and the `auth-session` feature (design
  decision 1) to `crates/corvis/Cargo.toml`; check with `cargo tree -d -p corvis --features auth-session` that `hmac`
  shares `digest` 0.10 with `sha2` and no duplicate generation appears, and with `cargo deny check` that licenses
  pass; verify `cargo clippy -p corvis --no-default-features --features auth-session -- -D warnings` passes on empty
  modules
- [x] 1.2 Add `auth-session` to the per-feature loop (`.github/workflows/ci.yml:72`), a
  `cargo test -p corvis --features auth-session --no-fail-fast` step next to the `compression` one (line 114), and
  the feature to the fuzz step (line 165); verify the new steps run in the CI of the pull request

## 2. Signed cookies (`http/signed-cookies`)

- [x] 2.1 `CookieKeys` in `crates/corvis/src/http/signed_cookie.rs`: `new` (1 to 8 secrets, 32-byte minimum,
  `CookieKeysError` without the secret), redacted `Debug`, `sign`; verify with unit tests for "Short secret", "Debug
  output" and a signature computed by hand for one fixed value
- [x] 2.2 `unsign` (prefix, 4096-byte cap before any HMAC, last `.`, every secret, `subtle` comparison of the
  signature strings) and `verified_with_current`, with a `#[cfg(test)]` HMAC counter; verify with unit tests for
  "Tampered value or signature", "Unsigned input", "Oversized cookie", "Value signed with a retired secret" and
  "Secret removed"
- [x] 2.3 `Request::signed_cookie(name, keys)`; verify with tests for "Unsigned cookie with the same name" and a
  cookie whose value needs URI-decoding
- [x] 2.4 Fixture: add `cookie-parser` (exact version) to `crates/corvis/tests/fixtures/auth/package.json`, write
  `signed_cookies.mjs` → `signed_cookies.json` (design decision 11) and `crates/corvis/tests/signed_cookies.rs`;
  re-read `cookie-signature` and correct "The original" in `design.md` if a fact differs; verify the generator is
  deterministic (two runs, no diff) and `cargo test -p corvis --features auth-session --test signed_cookies` passes
  without Node
- [x] 2.5 Property test in `crates/corvis/src/fuzz_tests.rs` for "Arbitrary input" (random strings, mutations of
  valid values, random secrets of valid length); verify no panic and no false acceptance

## 3. Session store

- [x] 3.1 `SessionStore` trait, `SessionStoreError` (no identifiers or data in messages) and the internal clock trait
  in `crates/corvis/src/auth/session/store.rs`; verify with a compile test of a minimal custom store
- [x] 3.2 `MemorySessionStore` (`max_sessions`, `max_total_bytes`, expiry on load, amortized purge on writes, least
  recently used eviction, rate-limited warning, `create` refusing an existing identifier, `update` / `touch`
  returning `false` for a missing one); verify with unit tests for "Count bound", "Byte bound", "Expired records
  leave" (test clock), "Identifier collision", and the eviction warning containing no identifier

## 4. Session middleware

- [x] 4.1 `SessionOptions`, `SessionCookie`, `SameSite` and their validation in `SessionManager` construction (cookie
  name, path and domain with the validators of `crates/corvis/src/http/res.rs:209-244`; prefix rules; `SameSite=None`
  without `Secure`; `max_age` against the absolute timeout; timeout and size ranges; warnings for `secure(false)`,
  `http_only(false)`, `no_absolute_timeout()` and the memory store); verify with tests for every scenario of "Unsafe
  cookie configurations are caught at startup", "Timeout out of range", "Absolute timeout disabled" and "Short secret"
- [x] 4.2 Identifier generation with `rand::rngs::OsRng` and base64url, incoming identifier validation, `Session`
  handle (`id`, `is_new`, `get`, `get_value`, `set`, `remove`, `clear`, `regenerate`, `destroy`, reserved keys,
  redacted `Debug`) and `Request::session()`; verify with unit tests for "Identifier shape", "Reserved key" and the
  `Debug` output
- [x] 4.3 `SessionMiddleware::use_`: cookie read and unsign, load, timeouts from `__corvis` and `cookie.expires`,
  handle insertion, commit with the saving rules of design decision 6 (create / update / touch, `Set-Cookie`
  decision, re-signing, the `Err` path, failed commit → 500, `max_session_bytes`), cookie written with
  `serialize_cookie`; verify with HTTP tests for "Value kept between requests", "Anonymous request", "Tampered
  cookie", "Unsigned identifier", "Signed but unknown identifier", "Defaults" (attributes), "Oversized session",
  "Secure cookie over plain HTTP", "Store failure on load" and every scenario of "Sessions expire by idle time and by
  age" and "Expiry is enforced by the middleware" with the test clock
- [x] 4.4 `SessionModule::for_root` / `for_root_async` (global dynamic module, `SessionManager` provider, middleware
  installed through the module definition's `configure`); verify with tests for "Middleware not installed", "Custom
  store", a session read in a guard, an interceptor and a `#[res]` handler, and that a CORS preflight creates no
  session
- [x] 4.5 Concurrency test for "Destroyed session does not come back": two requests on one session, the second
  destroys it while the first is held in its handler by a channel (no sleeps); verify the old identifier is absent
  from the store at the end

## 5. Login through a session

- [x] 5.1 `SessionSerializer`, `SessionStrategy<S>` (DI of `S`, self-registration with `SessionManager`, second
  registration as a startup error, strategy `"session"`), user restoring in the middleware; verify with tests for
  "Logged-in request", "User deleted after login", "Deserialization error", "Two session strategies", "Anonymous request to a protected route"
  and "Chain with a token"
- [x] 5.2 `Authenticator::log_in` / `log_out` under the feature (regeneration, data dropped or kept, `passport.user`,
  request user, old record destroyed before the new one is created); verify with tests for "Session fixation
  attempt", "Login without a previous session", "Data from before the login", "Logout" and "Login without session
  support"
- [x] 5.3 `AuthGuard::log_in()` and the extension of the startup check of #136 (a `.log_in()` guard also names the
  `"session"` strategy); verify with tests for "Login route", "Guard without log_in", "Failed login" and "log_in
  without a session strategy"

## 6. Differential fixture

- [x] 6.1 Add `express-session` 1.19.0 (exact) to the fixture package and refresh the lockfile; re-read its
  `index.js`, `session/cookie.js` and passport's `sessionmanager.js` and correct "The original" in `design.md` if a
  fact differs (in particular the `Expires` / `Max-Age` attributes and the key order of the `cookie` object); verify
  `npm ci` in the fixture folder
- [x] 6.2 `session.mjs` (header comment: versions, how to run, 127.0.0.1 only, fake secrets of at least 32 bytes)
  with the recording store wrapper and the sequences of design decision 11, identifiers replaced by placeholders,
  divergences marked with their reason; verify two runs produce the same `session.json`
- [x] 6.3 `crates/corvis/tests/auth_session.rs`: the same app with Corvis, every step replayed (status, body,
  `Set-Cookie` presence and attributes, store operations, final record without `__corvis`), "Cookie created by the
  original", "Cookie created by Corvis", "Record shape", divergences asserted from the spec, and the "Captured logs"
  check over the whole run; verify `cargo test -p corvis --features auth-session --test auth_session` passes and that
  editing one recorded attribute makes it fail
- [x] 6.4 Property test in `crates/corvis/src/fuzz_tests.rs` for "Arbitrary cookie headers"; verify no panic, no
  error status caused by the cookie, and no session loaded without a valid signature

## 7. Documentation and example

- [x] 7.1 `docs/http/session.md`: status `documented`; enabling `auth-session`, `SessionModule`, the `Session` API,
  cookie attributes and their defaults with the reason for each, plain-HTTP development (`secure(false)`), secrets
  and the rotation procedure, timeouts, limits table, the memory store's bounds and warnings, writing a store
  (contract: `create` must not overwrite, `update` must not create), saving rules and the `resave` /
  `save_uninitialized` / `rolling` options, cross-reading sessions with a Node app (`connect.sid`, `passport.user`,
  `__corvis`), the divergences table, middleware order; same in `docs/es/http/session.md`
- [x] 7.2 `docs/http/cookies.md`: a "Signed cookies" section replacing the statement of lines 32-33;
  `docs/security/csrf.md`: sessions exist, `SameSite=Lax` is the default, tokens are still not provided;
  `docs/recipes/passport.md`: sessions section (`SessionSerializer`, `SessionStrategy`, `AuthGuard::..log_in()`,
  `AuthGuard::new("session")`, `log_out`, the mapping from `serializeUser` / `deserializeUser` / `req.logIn` /
  `req.logout` / `passport.session()`); a pointer in `docs/security/authentication.md`; same in `docs/es/`; verify
  `npm run docs:check` in `docs/`
- [x] 7.3 `examples/cats-app` (design decision 12); verify the example builds, its existing tests pass, and the new
  test covers login, the changed identifier, the protected route, logout, the old cookie being anonymous, and a
  startup failure with a short secret

## 8. Definition of done

- [x] 8.1 `cargo build --workspace --all-targets` with no warnings
- [x] 8.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml` passes with `auth-session` alone and with `--no-default-features`
- [x] 8.3 `cargo test --workspace` and `cargo test -p corvis --features auth-session` pass, including
  `signed_cookies.rs` and `auth_session.rs` without Node
- [x] 8.4 Regression tests exist for every scenario of `http/signed-cookies` and `auth/sessions`
- [x] 8.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [x] 8.6 corvis-parity check: run it if any framework runtime code compiled without the `auth-session` feature
  changed (for example `AuthGuard`, `Request`, the middleware chain); otherwise record in the archive notes that the
  ports do not enable the feature and nothing else changed; verify the three port apps (consi, facten, houseplan)
  keep 0 apidiff differences when it runs
- [x] 8.7 `cargo deny check` passes with `hmac` and `subtle` as direct dependencies
- [x] 8.8 Security review (round 1 done and fixed, see 9; a second pass is the maintainer's call): run the security-reviewer agent over `corvis::http::signed_cookie`,
  `corvis::auth::session` and the example. Scope: signature forgery and comparison timing, secret handling and
  rotation, identifier entropy and the random source, adoption of client-chosen identifiers, fixation on login and
  logout, resurrection of destroyed sessions under concurrency, cookie attribute defaults and the startup checks,
  idle and absolute expiry (including stores that ignore the time to live and records without timestamps), memory
  store bounds and eviction as a denial of service, session size, store errors failing closed, identifiers, secrets
  and session data in logs, errors and `Debug`, panics reachable from a cookie, the reserved keys, and
  cross-reading records written by the original. Verify every finding is fixed or recorded as a new issue, and list
  the findings in the archive notes

## 9. Security review, round 1 (see design.md, "Security review (round 1)")

- [x] 9.1 Item 1: `SessionStore::destroy` returns `Result<bool, _>`; the commit treats a loaded session whose record is
  gone (destroy found nothing, `update` / `touch` found no record) as destroyed, clearing the cookie; race tests for
  logout and `destroy()` against `regenerate_keeping_data()`, `regenerate()`, a data change and a touch, held with gates
- [x] 9.2 Item 2: the memory store keeps serialized records, accounts the allocation and a per-entry overhead; counting
  allocator test with adversarial shapes
- [x] 9.3 Item 3: a login made by `AuthGuard::log_in()` is committed only when the response status is below 400; tests
  for a second guard, a handler that answers 403, a presented anonymous session, and a handler's own `log_in`
- [x] 9.4 Item 4: `max_sessions_per_user` (default 16, validated) and the eviction order (anonymous, then the user with
  the most sessions, oldest first); unit and HTTP tests
- [x] 9.5 Item 5: remove the default of `SessionStore::touch` and document the requirement on the trait
- [x] 9.6 Item 6: `SameOriginGuard` in the cats-app example on the session login, with tests; docs EN and ES
- [x] 9.7 Item 7: `__Host-` and `__Secure-` checked without regard to case, with tests
- [x] 9.8 Item 8: a stored record over `max_session_bytes` is no session (not loaded, adopted or deleted), with a test
- [x] 9.9 Item 9: docs EN and ES (cookie name not signed, session user visible on every route, cookie tossing, the
  `Secure` sentence)
- [x] 9.10 Specs updated (`auth/sessions`, `http/signed-cookies`); `openspec validate e3-sessions-and-signed-cookies
  --strict` passes

## Verification

Checked on `dev` at 7b3e1d6 (2026-11-10):

| Check | Result |
|---|---|
| CI on `dev` (fmt, clippy with the `auth-session` feature loop, test, fuzz, deny, audit, docs, packaging; Linux and Windows) | all jobs passed |
| `cargo test --workspace` | 1786 passed, 0 failed, 91 ignored |
| `cargo check -p corvis --tests --features <f>` for compression, auth-local, auth-api-key, socket-io, auth-session | no warnings |
| Security review, first round | 2 medium (logout racing a regeneration revived the session; memory store not bounded in real memory), 4 low, notes; all fixed (group 9) |
| Security review, second round | round 1 held (no resurrection in 16 interleavings and 100,000-request runs); 2 medium (a filter kept a guard login; parsing under the store lock), 5 low; all fixed or documented (group 10) |
| Security review, third round | round 2 held; 2 low (rejection mark missed two paths; login over an ended logged-in session), fixed (group 11) |
| Parity, consi | 832 checks equal, 0 different; databases equivalent |
| Parity, facten | 727 checks equal, 0 different; route tables identical; databases equivalent |
| Parity, houseplan | 636 checks equal, 0 different; route tables identical; databases equivalent |

Notes: the first CI run of the merge failed one test on Linux, the contention comparison of the memory store
(`big_sessions_do_not_slow_down_other_users`): on a runner with few cores the ratio measures the CPU the attackers
take, not the lock. It is now a measurement run on request (`--ignored`); 7b3e1d6. The group 11 changes were verified
by their regression tests and the full workspace run, not by a fourth review round. The parity run of facten and
houseplan was on f8814dd, which differs from 7b3e1d6 by that test file only; consi was rerun on 7b3e1d6.

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #44, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).
- If Open question 1 was answered with the split, create the issues for the Redis store and the ORM store under epic
  #39 (maintainer session) and link them from the archive notes.

## 10. Security review, round 2 (see design.md, "Security review (round 2)")

- [x] 10.1 Item 1: `RequestRejected` marked by the router on a failing pipeline, a stored guard response, a panic, a
  pipe failure and a handler error (before filters and interceptors); `commit` discards a guard login when it is set or
  no response exists, whatever the status; docs example EN and ES; tests for route and global filters (redirect, 200
  page), an interceptor that maps the error, handler `Err`, panic, failing pipe, disconnect, and a handler's own redirect
- [x] 10.2 Item 2: the memory store keeps `Arc<str>` records and a separate touch fragment; nothing proportional to a
  record under the lock; contention test with a ratio assertion; the measurement recorded in design.md
- [x] 10.3 Item 3: nesting depth bounded at write time (`MAX_RECORD_DEPTH`), a deeper stored record is no session, an
  unparsable record is destroyed on load, no fallback writes a null record; limits table EN and ES; tests
- [x] 10.4 Item 4: the hash tables of the memory store shrink; the counting-allocator test covers create, destroy and
  refill
- [x] 10.5 Item 5: a login that keeps nothing creates its session even when the previous record is gone; tests for the
  eviction during login and the logout racing a login (default and keeping data)
- [x] 10.6 Item 6: `SameOriginGuard` is an allow-list (`same-origin`, `none`, listed origins); unit tests and the example's
  HTTP test; docs EN and ES
- [x] 10.7 Item 7: docs EN and ES (logout and successor sessions, the stable `serialize_user`, flood by many accounts,
  disconnect during a guard login)
- [x] 10.8 Specs updated (`auth/sessions`); `openspec validate e3-sessions-and-signed-cookies --strict` passes; the
  definition of done (fmt, clippy with the feature loop, tests, docs check, naming, deny) passes

## 11. Security review, round 3 (see design.md, "Security review (round 3)")

- [x] 11.1 Mark the request as rejected when an interceptor fails and when the handler does not end with `Ok` (panic,
  drop); verify with the two new tests of `auth_session_review2.rs`
- [x] 11.2 The origin-gone exception only for a login over a session without a user; verify with
  `a_login_over_a_logged_in_session_that_was_logged_out_meanwhile_stays_logged_out` and the eviction test
- [x] 11.3 Depth-walk unit test without a deep drop; docs (EN, ES) for interceptors, the login race and the
  header-less request

