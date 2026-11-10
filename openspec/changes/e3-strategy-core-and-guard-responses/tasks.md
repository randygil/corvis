# Tasks

## 1. Guard responses (`http/guard-responses`)

- [x] 1.1 Factor the header validation of `set_response_header` (`crates/corvis/src/common/context.rs:188-198`) into
  one private function and add `HttpArgumentsHost::append_response_header` with the `AppendedHeaders` extension and the
  128-value cap (design decision 7); verify with unit tests: invalid name, CR/LF value, reserved header, WebSocket
  context, the 129th value, and that `set_response_header` behaves as before
- [x] 1.2 Merge appended headers wherever `apply_enhancer_headers` runs (`crates/corvis/src/router/mod.rs:711`, 730,
  736) and copy them into `Res` at creation (`router/mod.rs:772-778`) and on later calls; verify with
  `crates/corvis/tests/guard_responses.rs` covering "Cookie set by a guard survives an exception", "Guard cookies and
  handler cookie", "`#[res]` handler adds a cookie", "Two appended values" and "Handler header wins"
- [x] 1.3 `HttpArgumentsHost::respond_with` with the `PendingResponse` extension; check it after each guard in
  `run_pipeline` (`router/mod.rs:759-763`) and send it from `execute_route` without `finalize` or filters, with
  enhancer and appended headers; log a discarded `Ok(false)` / `Err`; verify with tests for "Guard short-circuits with
  a redirect", "Exception filters are not involved", "Global guard", "Guard stores a response and then fails" and
  "Rate limit headers on a redirect", plus CORS headers on a stored response
- [x] 1.4 Regression: run the existing `response_headers.rs`, `enhancer_order.rs`, `enhancers_routing.rs`,
  `res.rs` and `express_parity.rs` suites and add the "Plain guard denies" test; verify they pass unchanged
- [ ] 1.5 Run the corvis-parity check (parity-verifier agent) after group 1, before building on it; verify the three
  port apps keep 0 apidiff differences

## 2. Strategy contract and registry

- [x] 2.1 Create `crates/corvis/src/auth/` with `Strategy`, `StrategyOutcome`, `AuthenticateOptions`,
  `AuthenticatedUser` (serialize once, typed insert closure) and `AuthInfo`; export `corvis::auth` from
  `crates/corvis/src/lib.rs`; verify with unit tests that build each outcome and an `AuthenticatedUser` whose
  serialization fails
- [x] 2.2 `StrategyTag` field on `Provider` (`crates/corvis/src/di/provider.rs:179-210`) and `Provider::strategy::<S>()`
  (cast accepting `S` or `Arc<dyn Strategy>`); verify with a unit test next to the `app_guard` tests
- [x] 2.3 Core `Authenticator` provider in `InternalCoreModule` (`crates/corvis/src/di/container.rs:247-254`) holding
  the `OnceLock<StrategyRegistry>`; fill it in `init_steps` (`crates/corvis/src/application.rs:513-546`) with the
  duplicate-name error; verify with tests for "Strategy found by name" (strategy in one module, route in another,
  injected dependency), "Two strategies of the same type with different names" and "Duplicate name"
- [x] 2.4 `PassportModule::register` / `PassportModuleOptions` (`default_strategy`, `default_strategies`), read in
  `init_steps`; error on two different registrations; verify with the "Default strategy used" test

## 3. Authenticator and AuthGuard

- [x] 3.1 `Authenticator::authenticate` (design decision 4): order, first success, fail continues, error stops, pass,
  redirect through `respond_with` with URL validation, auth info for one and several failures, unknown name as 500;
  verify with unit tests over fake strategies, one per outcome and chain case
- [x] 3.2 `AuthGuard::new`, `chain`, `default`, `.optional()`, `.on_failure(..)`, `.with_options(..)`, user storage
  and removal (design decisions 5 and 6), `controls_access()` false when optional; `User<T>` extractor and
  `Request::auth_info()`; verify with HTTP tests through `corvis::testing::request` for "Handler reads the user",
  "Extractor without a user", "User that does not serialize", "Optional guard without credentials", "Strategy passes",
  "Custom failure exception", "Auth info of a failed chain" and "Optional guard does not protect a route" (route
  posture enforced)
- [x] 3.3 `.skip_public()` over `PUBLIC_METADATA` with `Reflector::get_all_and_override`; verify with "Global guard
  and a public route" and "Handler overrides a public controller"
- [x] 3.4 WebSocket context: deny without running strategies, one warning per guard; verify with a gateway test in
  the style of `crates/corvis/tests/ws_enhancers.rs` that counts strategy calls (zero) and the logged warning (one)
- [x] 3.5 `CanActivate::as_any` (`crates/corvis/src/common/guard.rs:23-40`), `AuthGuard` override, and the startup
  check in `init_steps` next to `posture::check_routes` (`application.rs:573-574`) that binds the `Authenticator` and
  lists missing names with their routes; verify with "Typo in a guard" (also that no init hook ran), "Default guard
  without a default strategy", an empty `chain`, an unregistered default strategy, and "Name resolved only at request
  time"

## 4. Differential fixture

- [x] 4.1 `crates/corvis/tests/fixtures/auth/package.json` with the exact versions of design decision 11,
  `package-lock.json` from `npm install`, and the `node_modules` entry in the repository ignore file next to the other
  fixtures; verify `npm ci` in that folder and that `git status` does not list `node_modules`
- [x] 4.2 `strategies.mjs` (header comment: versions, how to run, 127.0.0.1 only) recording the cases of design
  decision 11 into `strategies.json`; verify by running it twice and diffing the output (deterministic)
- [x] 4.3 `crates/corvis/tests/auth_strategies.rs`: the same routes with Corvis strategies, replaying every case
  (status, listed headers, body) and asserting the spec for cases with `divergence`; verify `cargo test -p corvis
  --test auth_strategies` passes and that editing one recorded status makes it fail

## 5. Documentation

- [x] 5.1 `docs/recipes/passport.md`: status `partial`; the strategy model (`Strategy`, `Provider::strategy`,
  `PassportModule`), `AuthGuard` (single, chain, default, optional, `on_failure`, `skip_public`), `User<T>` and
  `auth_info`, writing a custom strategy, the divergences (startup errors, fail-closed on gateways, typed user not
  removed), a table from the original API (`PassportStrategy`, `AuthGuard('x')`, `req.user`, `handleRequest`) to
  Corvis, and a note that JWT, local, API keys, sessions and OAuth2 come with #42, #41, #45, #44, #43; same in
  `docs/es/recipes/passport.md`
- [x] 5.2 `docs/guards.md`: a section on `respond_with` and `append_response_header` (cookies, order, limits); same in
  `docs/es/guards.md`
- [x] 5.3 Run `npm run docs:check` in `docs/`; verify it passes

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml:69-73` (including `--no-default-features`) passes
- [x] 6.3 `cargo test --workspace` passes, including `guard_responses.rs` and the fixture replay of
  `auth_strategies.rs` (no Node needed)
- [x] 6.4 Regression tests exist for every scenario of `auth/strategies` and `http/guard-responses`
- [x] 6.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 6.6 Framework runtime code changed (router, `init`, `Provider`): run the corvis-parity check; verify the three
  port apps (consi, facten, houseplan) keep 0 apidiff differences
- [ ] 6.7 Run the security-reviewer agent over the router changes and `corvis::auth` (header injection through
  appended headers and redirect URLs, short-circuit bypassing guards, fail-closed paths); verify every finding is fixed
  or recorded as a new issue

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #136, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).
