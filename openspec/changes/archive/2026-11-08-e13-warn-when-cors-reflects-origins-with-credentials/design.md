# Design

## Context

See proposal.md for motivation; the behavior is in the `http/cors` spec delta. Current state, verified in the code:

**CORS options**

- `crates/corvis/src/router/cors.rs:1-13` documents the module as the `cors` package semantics (`origin: true` is
  reflection).
- `CorsOrigin` (`cors.rs:60-77`) has the variants `Any` (default), `Reflect` ("`true`: reflect the request origin",
  `:67-68`), `Disabled`, `Fixed(String)`, `List(Vec<OriginMatcher>)` and `Dynamic(OriginFn)`.
- `CorsOptions` (`cors.rs:93-105`) is a plain struct with public fields, including `origin` and `credentials: bool`
  (`:101`); `Default` sets `credentials: false` (`:107-121`). Builders: `reflect_origin()` (`:163-167`),
  `origin_value(CorsOrigin)` (`:173-177`), `credentials(bool)` (`:190-193`); `origin("*")` maps to `Any`
  (`:136`).
- At request time, `configure_origin` (`cors.rs:219-233`) answers `Reflect` with the request's `Origin` header
  (`:226`), and `headers` (`:237-...`) adds `access-control-allow-credentials: true` whenever `credentials` is set
  (`:247-249`), whatever the origin. `Dynamic` is resolved per request through the user function (`:207-217`), so its
  outcome is unknown before a request arrives.
- Nothing inspects the options when they are set or when the app starts: `grep` finds no `warn` in `cors.rs`.

**Where the options reach the application**

- `CorvisApplicationOptions::cors` (`crates/corvis/src/application.rs:32-38`) is copied into `AppConfig::cors` during
  `bootstrap` (`:129`); `CorvisApplication::enable_cors` (`:198-202`) overwrites the same field.
- `CorvisApplication::init()` (`application.rs:361-424`) returns early when already initialized (`:362-364`), folds the
  provider-registered global enhancers (`:365-398`), compiles the router (`build_router`, `:399`), builds middleware
  and gateways, then clones `config.cors` into `AppShared` (`:408`). After `init()` the options cannot change
  (`enable_cors` after `init()` only edits `config`, which is not read again).
- `listen()` and `listen_in_background()` both go through `bind()`, which calls `init()` first (`:476-477`); the testing
  utilities call `init()` through `CorvisApplication::handle` (`:426-430`).

**Startup checks and logging**

- `Logger` (`crates/corvis/src/logger.rs:200-237`) has `warn(..)`; custom sinks implement
  `LoggerService::log(level, message, context)` (`logger.rs:50-52`) and are installed with
  `CorvisApplication::use_logger` (`application.rs:229-233`), which sets the process-wide logger.
- The route posture change (#26, `openspec/changes/e1-public-metadata-and-startup-check/design.md`, decisions 1 and 8)
  adds a check to `init()` right after `build_router` and before `build_middleware`, logging with
  `Logger::new("RoutePosture")` (`error` in enforce mode, `warn` in warn mode), once per `init()`. It is not
  implemented yet on this branch.
- The compatible vs strict mode spike (#8, `openspec/changes/e0-decide-compatible-vs-strict-mode/design.md`, CORS row
  of decision 2) settled: keep the `cors` semantics and warn at startup for reflect + credentials, in a separate issue
  (this one).

**Tests and apps**

- `crates/corvis/tests/express_parity.rs:230-235` pins the reflected response for `reflect_origin()` (without
  credentials) and `:221-226` a fixed origin with credentials.
- The only example that enables CORS uses the default (`examples/cats-app/src/main.rs:22`). consi
  (`src/main.rs:61`) and facten (`src/app.setup.rs:27`) use `origins(..)` + `credentials(true)`; houseplan does not
  call `enable_cors`.
- Docs: `docs/security/cors.md:53` (the `reflect_origin()` row) and `:99-103` (warning box "`credentials` and open
  origins"); same in `docs/es/security/cors.md:99-103`.

## Goals / Non-Goals

**Goals:**
- One clear warning at startup for `Reflect` + `credentials`, with a pointer to the fix.
- Zero change in HTTP behavior and zero output for the configurations the port apps use.

**Non-Goals:**
- Refusing to start, or a strict mode (#8).
- Detecting other risky CORS configurations (see proposal, Out of scope).

## Decisions

1. **A pure check on `CorsOptions`.** Add `pub(crate) fn startup_warning(&self) -> Option<String>` to `CorsOptions` in
   `cors.rs`: `Some(message)` iff `self.credentials && matches!(self.origin, CorsOrigin::Reflect)`. Being pure, it is
   unit-tested without an application and without the global logger.
   - `Any` + credentials: the headers are `*` plus `Allow-Credentials: true`; browsers reject that pair, so no data is
     exposed. No warning.
   - `Dynamic`: the function may return `Reflect` for some origins on purpose (the docs example does it for
     `*.example.com`); the check cannot tell. No warning; the docs keep the advice.
   - Alternative: a public `CorsOptions::lint()` returning a list of findings. Rejected for now: one finding does not
     need an API; it can grow into one when the other checks (out of scope) arrive.

2. **Run it in `init()`, next to the route posture check.** `init()` calls the check right after `build_router`
   (`application.rs:399`); once #26 lands, right after `posture::check_routes`, so every startup check logs after
   the route map and before any user hook, middleware `configure` or socket bind. `init()` already returns early when
   initialized (`:362-364`), so the warning is logged once per application.
   - Alternative: warn inside `enable_cors`. Rejected: `CorvisApplicationOptions::cors` does not go through it, a
     second `enable_cors` call replaces the first (only the effective options matter), and logging from a builder is
     surprising.
   - Alternative: warn in `bind()`. Rejected: apps tested with the in-memory utilities would never see it.
   - Placing it at the very top of `init()` would also work, but would print it before "Mapped {...} route" lines,
     away from the route posture output.

3. **Logger context and message.** `Logger::new("CorsOptions").warn(message)`, the type the user configured (like
   `RoutePosture` names the feature). Message:
   ```text
   CORS reflects the request origin (reflect_origin()) together with credentials(true): any website can send
   requests with the user's cookies to this application and read the responses. Allow an explicit list of
   origins with origins([...]) instead. See https://corvis.randygil.com/security/cors#credentials-and-open-origins
   ```
   One line in the log (the wrapping above is for reading). The URL is the published docs site
   (`openspec/specs/docs/publishing/spec.md`); the anchor is created in the docs task with an explicit
   `{#credentials-and-open-origins}` so the English and Spanish headings share it.

4. **Warning, never an error.** Parity with `cors` (decision of #8). Users who see it in tests can silence the
   `CorsOptions` context through their custom `LoggerService` or `LoggerOption::Levels`; no dedicated opt-out is added
   (open question).

5. **Tests.** Unit tests in `cors.rs` for the truth table of decision 1 (every `CorsOrigin` variant × credentials).
   A new integration test file `crates/corvis/tests/cors_startup_warning.rs` (its own binary, so the process-wide
   logger does not leak into other test files) installs a capturing `LoggerService` with `use_logger`, serializes its
   tests with a `static Mutex`, and checks the spec scenarios: reflection + credentials through `enable_cors` and
   through `CorvisApplicationOptions::cors`, a second `init()`, explicit list, reflection without credentials,
   `origin_fn`, and the unchanged response headers (the reflected request of `express_parity.rs:230-235` with
   credentials added).

## Risks / Trade-offs

- [Noise for apps that use reflection + credentials on purpose (internal tools, development)] → one line per start;
  the docs explain how to filter it, and an opt-out can be added if users ask (open question).
- [The process-wide logger makes log assertions racy] → the integration tests live in their own test binary and are
  serialized with a mutex; the decision logic itself is covered by pure unit tests.
- [`Dynamic` configurations that reflect everything are not reported] → documented; detecting them would require
  calling user code at startup with made-up origins.
- [The docs URL in the message breaks if the page moves] → `npm run docs:check` does not check Rust strings; the
  integration test asserts the URL, and the docs task adds the explicit anchor.

## Open questions

- Should there be an explicit acknowledgement to silence the warning (for example
  `CorsOptions::allow_credentials_with_any_origin(true)`), or is logger filtering enough?
- Should the message link to the English page only (current choice), or to the language-neutral path?
- When #8's strict mode exists, should this combination become a startup error in strict mode?

## Review notes (2026-11-08)

Decisions taken for the open questions so the change can start; the maintainer can still overrule them in review:

- No option to silence the warning: it is logged once at startup and can be filtered with the logger; an explicit
  opt-out would make the insecure combination look endorsed.
- The link points to the English page; the Spanish page keeps the same anchor id so the docs language switcher works.
- Whether this combination becomes an error in a strict mode belongs to #8 (decide compatible vs strict mode); noted
  there, not here.
- The folder was renamed to `e13-warn-when-cors-reflects-origins-with-credentials` (issue #134).
