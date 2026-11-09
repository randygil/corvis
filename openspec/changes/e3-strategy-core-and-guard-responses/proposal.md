# Proposal

Implements [randygil/corvis#136](https://github.com/randygil/corvis/issues/136) (epic
[#39](https://github.com/randygil/corvis/issues/39)). The design comes from the #40 spike
(`openspec/changes/archive/2026-11-08-e3-strategy-module-design/`, `design.md` decisions 1-7, 10-12 and its Review
notes). This is the first change of the epic: #42 (JWT), #41 (local), #45 (API keys), #44 (sessions) and #43 (OAuth2)
are written against the API it adds.

## Why

Apps built on the original model authenticate with named strategies: `AuthGuard('jwt')` on a route, a strategy class
whose `validate()` turns credentials into a user, `req.user` in the handler, and a fixed
`401 {"message":"Unauthorized","statusCode":401}` when every strategy fails. Corvis has none of it:

- `docs/recipes/passport.md` is `status: unavailable` and the only path is a hand-written guard over `JwtService`
  (`examples/cats-app/src/auth/auth_guard.rs`, `docs/security/authentication.md:133-180`).
- A guard can only answer `true`, `false` or an `HttpException` (`crates/corvis/src/common/guard.rs:23-40`,
  `crates/corvis/src/router/mod.rs:758-763`), so a strategy cannot redirect (OAuth2) and there is no way to add a
  second `Set-Cookie` that survives the handler (`HttpArgumentsHost::set_response_header` replaces the recorded value
  and never adds to a header the response already has, `crates/corvis/src/common/context.rs:172-223`).

Every strategy issue of the epic needs the same core first: the registry, the strategy contract, `AuthGuard` with the
original's semantics, and the two router capabilities guards are missing.

## What Changes

- New module `corvis::auth`, always compiled (no new dependencies): the object-safe `Strategy` trait and its five
  outcomes (success, fail, redirect, pass, error, as passport's), `AuthenticatedUser`, `AuthenticateOptions`,
  `Authenticator` (the port of `passport.authenticate` with the custom callback `AuthGuard` uses), `AuthGuard`,
  `PassportModule` / `PassportModuleOptions` (default strategy), the `User<T>` parameter extractor, and
  `Request::auth_info()`.
- `Provider::strategy::<S>()`: a provider tagged like `Provider::app_guard` (`crates/corvis/src/di/provider.rs:346-353`),
  collected by `CorvisApplication::init()` into the registry held by the core `Authenticator` provider.
- Startup errors, divergences from the original documented in the spec: two strategies with the same name; an
  `AuthGuard` on a route, or the default strategy, naming a strategy that is not registered; `AuthGuard::default()`
  without a configured default.
- `AuthGuard::new(name)`, `AuthGuard::chain([..])`, `AuthGuard::default()`, with `.optional()`, `.on_failure(..)` and
  `.skip_public()` (reads the `isPublic` metadata of #26, `PUBLIC_METADATA` in `crates/corvis/src/lib.rs:251`).
- Guard responses (`http/guard-responses`): `HttpArgumentsHost::respond_with(Response)` lets a guard end the request
  with its own response (a strategy redirect), checked by the router after each guard; and
  `HttpArgumentsHost::append_response_header(name, value)` records header values that accumulate and, for
  `Set-Cookie`, are kept next to the response's own cookies on every path (success, exception, filter, `#[res]`,
  short-circuit).
- `CanActivate::as_any()` (default method, returns `None`), overridden by `AuthGuard`, so `init()` can find the
  strategy names of compiled route guards.
- Differential tests against `@nestjs/passport` 12.0.1 and `passport` 0.7.0: a Node fixture package under
  `crates/corvis/tests/fixtures/auth/` (pinned `package.json` and lockfile, like `fixtures/throttler/`), a generator
  `strategies.mjs` that records the responses of custom `passport-strategy` 1.0.0 strategies driven to each outcome,
  and `crates/corvis/tests/auth_strategies.rs`, which replays every case with no Node at test time.
- Docs (English and Spanish): `docs/recipes/passport.md` (status `partial`: the strategy model, `AuthGuard`, writing a
  strategy; JWT, local and the rest arrive with their issues) and a short section in `docs/guards.md` on ending a
  request from a guard and appending headers.

## Capabilities

### New Capabilities

- `auth/strategies`: named strategy registry, strategy outcomes, `AuthGuard` (single, chained, optional, public
  routes), user and auth info on the request, failure responses, startup checks, differential verification.
- `http/guard-responses`: a guard can end the request with a response and append response headers that survive the
  handler, exception filters and `#[res]`.

### Modified Capabilities

None. The `set_response_header` requirements of `core/enhancers` (`openspec/specs/core/enhancers/spec.md:144-217`)
stay as they are; `append_response_header` is a sibling with its own requirements in `http/guard-responses`.

## Impact

- New `crates/corvis/src/auth/` (`mod.rs`, `strategy.rs`, `authenticator.rs`, `guard.rs`, `user.rs`,
  `module.rs`), exported from `crates/corvis/src/lib.rs` as `corvis::auth` (not in the prelude: apps often have their
  own `AuthGuard`, `docs/guards.md:35`).
- `crates/corvis/src/di/provider.rs`: `strategy` tag field on `Provider` (always present, Review note 9 of the spike)
  and `Provider::strategy::<S>()`.
- `crates/corvis/src/di/container.rs:247-254`: the core `Authenticator` provider in `InternalCoreModule`.
- `crates/corvis/src/application.rs` (`init_steps`, lines 504-596): build the registry, run the strategy-name check
  next to the route posture check.
- `crates/corvis/src/common/guard.rs`: `as_any` default method.
- `crates/corvis/src/common/context.rs`: `respond_with`, `append_response_header`.
- `crates/corvis/src/router/mod.rs`: `run_pipeline` (lines 758-782) checks for a stored response after each guard;
  `execute_route` (696-740) and `apply_enhancer_headers` (744-756) merge appended headers.
- `crates/corvis/src/http/request.rs`: `auth_info()`.
- Fixture package `crates/corvis/tests/fixtures/auth/` and an ignore entry for its `node_modules` in the repository
  ignore file, like the other fixtures (lines 29-39 there).
- Tests: `crates/corvis/tests/guard_responses.rs`, `crates/corvis/tests/auth_strategies.rs`, unit tests in
  `crates/corvis/src/auth/`.
- Docs: `docs/recipes/passport.md`, `docs/guards.md` and `docs/es/` copies.
- Port apps: none uses the new APIs; the router change touches every guarded request, so the corvis-parity check is
  required.

## Out of scope

- The JWT, local, API key, session and OAuth2 strategies (#42, #41, #45, #44, #43), and their cargo features.
- `AuthGuard::log_in()` and the "AuthGuard does not log in to a session by itself" requirement of the spike: there is
  no session yet; both move to #44.
- `AuthGuard` on WebSocket gateways: in a WebSocket context this change makes `AuthGuard` deny the message (fail
  closed, see design decision 9); real handshake authentication through strategies is a possible new issue.
- A `#[strategy("name")]` attribute macro (spike decision 1, option C): possible new issue once the runtime model is in
  use.
- The `property` option, `failureRedirect`, flash messages and `assignProperty` of the original (spike non-goals).
- Letting middleware call `respond_with` / `append_response_header`: middleware already returns or edits the
  response itself (`crates/corvis/src/common/middleware.rs`); possible new issue if a strategy needs it.
- Injecting `AuthGuard` by type (`#[use_guards(AuthGuard)]` resolved through DI as the default-strategy guard):
  possible new issue; this change uses instance guards (`AuthGuard::default()`).
