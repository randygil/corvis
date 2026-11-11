# Proposal

Implements [randygil/corvis#46](https://github.com/randygil/corvis/issues/46) (epic
[#39](https://github.com/randygil/corvis/issues/39)). Planned in the #40 spike
(`openspec/changes/archive/2026-11-08-e3-strategy-module-design/`, task group 7) and re-scoped here against what the
epic became: the strategy core is merged (`openspec/changes/archive/2026-11-10-e3-strategy-core-and-guard-responses/`,
code in `crates/corvis/src/auth/`) and every strategy change (#42 JWT, #41 local, #45 API keys, #44 sessions, #43
OAuth2) already ends with its own adversarial security review of its own module.

**Depends on #42, #41, #45, #44 and #43 being merged into `dev`.** Order of the epic: #136 (merged) → #42 → #41 →
#45 → #44 → #43 → **#46** → #47. Their APIs are described here as designed (their `design.md` with the Review notes),
not as shipped; the first task compares them with the merged code.

## Why

Each change of the epic is reviewed alone, with its own threat list: algorithm confusion for JWT, timing and planted
hashes for passwords, comparison timing and log redaction for API keys, fixation and store races for sessions, state
binding and the provider client for OAuth2. Those reviews found real problems (open redirects, a guard-phase response
issue in the core, lax `iss`/`aud` validation in the older `JwtService`, a wrapper-guard pattern that hid a
misconfiguration until the first request). What none of them can see is what happens **between** the modules, because
no single change contains two of them:

- A session cookie restores a user on every route before the guards run (#44 design decision 10), while a route may be
  guarded by `AuthGuard::new("jwt")`, by an optional guard, by a hand-written roles guard, or by nothing. Which
  identity the handler sees, and whether a cookie-borne identity reaches a route its author thought was token-only,
  is decided by three changes together.
- Chains (`AuthGuard::chain([..])`) fall through on `Fail`, stop on `Error` and let the request through on `Pass`
  (`crates/corvis/src/auth/authenticator.rs:148-152`). Whether a store outage or a provider failure can fall through
  to a weaker strategy, and whether two strategies that read the same place (`Authorization: Bearer` for JWT and for
  the API key `bearer()` source) agree on what they read, is a property of the combination.
- The OAuth2 `state` may live in the session (#43 decision 4) and the callback may log in to the session (#44
  `AuthGuard::..log_in()`), which regenerates it. The order of "take the state", "regenerate", "destroy the old
  record" spans two changes.
- Guards run in the order global, controller, route (`crates/corvis/src/router/mod.rs:806-826`). A throttler that
  runs after an `AuthGuard` limits nothing: the password hash, the store read or the provider request already
  happened. `#[public]` is honored only by a guard built with `.skip_public()`
  (`crates/corvis/src/auth/guard.rs:160`).
- WebSocket upgrades never run HTTP middleware or guards (`crates/corvis/src/server.rs:560-567`), `AuthGuard` denies
  every socket message (`guard.rs:157`), and the documented handshake pattern verifies tokens with `JwtService`, whose
  rules are looser than the JWT strategy's (#42 keeps `JwtService` unchanged). The same token can be refused over
  HTTP and accepted on a socket.
- Defaults were chosen change by change: the 32-byte secret rule, cookie attributes, "credentials in a URL only when
  asked, with a warning", redacted `Debug`, constant-time comparison. Nothing checks that they are the same rule
  everywhere, and nothing fails when one of them is changed later.

The issue text lists OAuth CSRF, session fixation, timing, user enumeration and login rate limiting. The first four
are specified and tested inside #43, #44 and #41; this change verifies them **across** strategies and adds the one
that belongs to no strategy: login rate limiting, which #41 and #43 left to this issue.

## What Changes

- **Cross-cutting review** by the `security-reviewer` agent over the whole `corvis::auth` tree and its touch points
  (router guard loop, `Request` user slots, session middleware, WebSocket handshake, throttler), with a fixed scope
  list (design decision 2) made of interactions only. Per-module threats already reviewed are not repeated; the
  review reads the earlier findings first and checks that their fixes still hold when strategies are combined.
- **Threat model document**: `docs/security/authentication-threat-model.md` (and `docs/es/`): assets, trust
  boundaries, attackers, one table per surface (threat, mitigation, the test that proves it, residual risk), the
  defaults table, the dependency inventory and the findings register. It is the source the security guidance page of
  #47 is written from.
- **Invariants turned into tests** (`auth/security-invariants`): a checklist of properties that must hold for every
  combination of strategies, each with a regression test in `crates/corvis/tests/auth_invariants.rs`: identity comes
  from the route's guard, chains fail closed, one failure response, no authority-carrying cookie on a failed
  request, public routes and guard order, OAuth2 state and session regeneration, socket traffic, consistent defaults
  (a test compares the documented defaults table with the code), no secret in logs, bodies or `Debug`.
- **Fuzz targets** in the style of the existing property fuzzing (`crates/corvis/src/fuzz_tests.rs`, CI job `fuzz`):
  a model-based gate property (random credentials against every guard shape, compared with a reference model), a
  session state machine, the OAuth2 callback against a counting mock provider, redirect targets, and a canary-secret
  property over logs and responses. They run in `cargo test`, in the CI `fuzz` job and in the nightly long run.
- **Login rate limiting** (`auth/login-rate-limiting`): no new throttler; the existing `ThrottlerGuard` with a
  per-route override and a per-account tracker. New: `corvis::auth::local::login_tracker`, a tracker that names the
  same account the local strategy will read (so a username moved from the body to the query cannot dodge the
  counter), a startup warning when an `AuthGuard` runs before the throttler on a route, and tests that a throttled
  request runs no strategy, no key derivation, no store write and no provider request.
- **Dependency audit of the crypto crates** (`infra/auth-dependencies`): an inventory per auth feature, one
  generation of each primitive enforced by `cargo deny`, native TLS stacks banned, no advisory exception for an auth
  crate without a threat-model entry, and a source check that identifiers, state, verifiers and salts come from the
  operating system generator.
- **Triage rule** for findings (design decision 9): fixed in this change when the fix stays inside the auth modules
  and changes no specified behavior of another capability; otherwise recorded as an issue, with the workaround
  written in the threat model. Fixes that change a requirement of an archived change add a `MODIFIED` delta to this
  change before they are implemented.

## Capabilities

### New Capabilities

- `auth/security-invariants`: properties that hold across strategies, guards, sessions and sockets, and the fuzz
  targets that exercise them.
- `auth/login-rate-limiting`: throttling of login and callback routes with the existing throttler: order relative to
  authentication, per-account tracking consistent with the local strategy, what a throttled request must not cost.
- `auth/threat-model`: the threat model document, its traceability to tests, the defaults table and the findings
  register.
- `infra/auth-dependencies`: inventory and policy for the cryptographic crates the auth features pull.

### Modified Capabilities

None when drafted. A finding fixed in this change that alters a requirement of `auth/strategies`,
`auth/jwt-strategy`, `auth/local`, `auth/api-key`, `auth/sessions`, `http/signed-cookies` or `auth/oauth2` adds its
`MODIFIED` delta here (design decision 9).

## Impact

- New tests: `crates/corvis/tests/auth_invariants.rs`, `crates/corvis/tests/auth_login_throttle.rs`,
  `crates/corvis/tests/fuzz_auth.rs`, `crates/corvis/tests/auth_defaults.rs`,
  `crates/corvis/tests/auth_source_checks.rs`, with a shared test application in
  `crates/corvis/tests/auth_common/`.
- `crates/corvis/src/auth/local/`: `login_tracker` (behind `auth-local`).
- `crates/corvis/src/auth/mod.rs` (`check_routes`, lines 94-112): the guard-order warning.
- `crates/corvis/src/modules/throttler.rs`: `ThrottlerGuard` overrides `CanActivate::as_any` (it has no override
  today, line 1181 onwards), so the startup check can recognize it.
- `.github/workflows/ci.yml` (job `fuzz`, lines 143-177; per-feature test steps) and `.github/workflows/nightly.yml`
  (lines 33-69): the new test binaries with the auth features enabled.
- `deny.toml`: banned crates and the single-generation list.
- Docs: `docs/security/authentication-threat-model.md`, `docs/security/rate-limiting.md` (login section), the sidebar,
  and their `docs/es/` copies.
- Fixes for findings: inside `crates/corvis/src/auth/` and the auth-feature modules
  (`crates/corvis/src/http/signed_cookie.rs`). Anything else becomes an issue.
- Port apps: they enable no auth feature. `ThrottlerGuard::as_any` and the startup warning are compiled in every
  build, so the corvis-parity check is required.

## Out of scope

- Repeating the per-module reviews of #136, #42, #41, #45, #44 and #43. Their findings are inputs; a regression of
  one of them found here is a finding of this change.
- Handshake authentication through strategies (running a `Strategy` for a WebSocket or Socket.IO connection): the
  review records the gap between `JwtService` and the JWT strategy at the handshake and files it; possible new issue
  "strategies at the WebSocket handshake".
- Changing `JwtService` / `JwtModule` defaults (short secrets, optional `exp`): specified by `auth/jwt` and used by
  the three port apps. Recorded in the threat model; possible new issue.
- Account lockout, CAPTCHA, credential-stuffing detection, breached-password checks, multi-factor authentication:
  application policy or new issues.
- CSRF tokens (`docs/security/csrf.md`): possible new issue, noted in #44.
- A coverage-guided fuzzer (`cargo fuzz`, libFuzzer): the repository fuzzes with property tests that run on every
  platform in `cargo test`; a coverage-guided harness is a possible new issue.
- An external penetration test or a formal audit of the cryptographic crates' internals.
- The user-facing guide, the migration notes, the security guidance page and the example: #47.
- Socket.IO (`e2-socket-io-*`) internals. Its handshake is reviewed only as an authentication surface, and only if
  those changes are merged when this one starts; otherwise the threat model lists it as pending with an issue.
