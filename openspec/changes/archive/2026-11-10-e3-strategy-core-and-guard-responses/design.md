# Design

## Context

The #40 spike (`openspec/changes/archive/2026-11-08-e3-strategy-module-design/design.md`) chose the model: a registry
of named strategies behind an object-safe trait, typed validators with adapter strategies, `AuthGuard` as a port of
`passport/lib/middleware/authenticate.js` plus `@nestjs/passport`'s `auth.guard.ts`, and a router prerequisite for
redirects and cookies. Its Review notes fixed: `PassportModule` as the module name (note 1), this issue done first
(note 3), `skip_public()` over the `isPublic` key (note 4), a `package.json` for the Node fixtures (note 5), the
strategy tag on `Provider` always present (note 9). This change implements that core. Line numbers below were checked
on `origin/dev` at `6f5ff4d`; several moved since the spike.

### Guards today

- `CanActivate` (`crates/corvis/src/common/guard.rs:23-40`) has `can_activate` and two default methods used by route
  posture, `controls_access()` (true by default) and `guard_name()` (the type name).
- `run_pipeline` (`crates/corvis/src/router/mod.rs:758-782`) runs `route.guards` in order (lines 759-763): `Ok(false)`
  becomes `ForbiddenException::new("Forbidden resource")`, an `Err` is returned as is. Then, for `#[res]` routes, it
  creates the `Res` and copies the recorded enhancer headers into it (764-780), then calls the interceptors.
- `route.guards` already holds global, controller and route guards in that order (`CompiledRoute`,
  `router/mod.rs:38-62`, fields `guards`, `global_guard_count`, `controller_guard_count`), so one check after each
  guard covers all three levels.
- `execute_route` (`router/mod.rs:696-740`) builds the `ExecutionContext` with the route's `ModuleRef` (line 700),
  runs the pipeline, then: without `#[res]`, `finalize` (route status, `#[header]`, route redirect;
  `router/mod.rs:869-894`) on success or `handle_exception` (897-909) on error, and `apply_enhancer_headers`
  (744-756) on both; with `#[res]`, an ended `Res` is returned as is (716-723) and otherwise merged (724-739).
- Instance guards: `#[use_guards(expr)]` turns any expression that is not a type path into `guard_instance(expr)`
  (`crates/corvis-macros/src/utils.rs:105-119`, `crates/corvis/src/common/enhancer.rs:61-64`), so
  `#[use_guards(AuthGuard::new("jwt"))]` needs no macro change. `app.use_global_guards(guard)` takes an instance
  (`crates/corvis/src/application.rs:226-227`).
- WebSocket handlers run the same guards (`crates/corvis/src/websockets.rs:782-786`, `WsException("Forbidden
  resource")`), and global guards apply to gateways by default (`gateway_global_enhancers: true`,
  `router/mod.rs:328`; switch in `application.rs:209-214`).

### Response headers from enhancers today

`HttpArgumentsHost::set_response_header` (`crates/corvis/src/common/context.rs:172-223`) validates the header (Node's
character rules, reserved framing headers in `RESERVED_RESPONSE_HEADERS`, lines 153-155), records it in the
`EnhancerHeaders` extension (160-170) with `HeaderMap::insert` (one value per name, at most `MAX_ENHANCER_HEADERS` =
128 names, line 158), returns `false` in a WebSocket context, and writes it to an existing `Res`.
`apply_enhancer_headers` adds a recorded header only when the response has no header with that name. Requirements:
`openspec/specs/core/enhancers/spec.md:144-217`. This is the base the spike's `append_response_header` builds on: it
did not exist when the spike was written. Its two limits for cookies: a second `Set-Cookie` replaces the first, and a
handler or filter that sets its own `Set-Cookie` drops the guard's.

### DI and startup

- `Provider` (`crates/corvis/src/di/provider.rs:179-190`) carries `enhancer: Option<GlobalEnhancer>` (enum at
  131-136); `Provider::app_guard::<T>()` (346-353) is a `use_class` provider with a `Guard` cast that also accepts
  the trait object a testing override stores.
- `init_steps` (`application.rs:504-596`) walks `container.all_providers()` and installs the tagged enhancers
  (513-546), builds the router (553), compiles gateways, runs the route posture report and check (573-574, after the
  comment at 559-560: "a failing check must not have run user code"), then module middleware `configure` (575),
  gateways and the init hooks (594).
- The container has a global internal module with core providers: `Reflector` and `HttpAdapterHost`
  (`crates/corvis/src/di/container.rs:247-254`). `HttpAdapterHost` is filled after the scan, which is the model for
  a registry filled in `init()`.
- `ModuleRef::get` looks in the module context, then application-wide (`crates/corvis/src/di/module_ref.rs:21-36`);
  `ExecutionContext::module_ref()` (`context.rs:139-141`) gives it to guards.

### Request and responses

- `Request::insert` / `get` / `remove` store typed extensions shared by all clones
  (`crates/corvis/src/http/request.rs:232-242`); `set_user` stores JSON in the private `RequestUser` and silently
  ignores a serialization failure (245-249); `user()` / `user_as()` read it (250-255).
- `FromContext` (`crates/corvis/src/common/param.rs:29-32`) is the extractor trait for a `User<T>` parameter.
- `UnauthorizedException::empty()` gives `{"message":"Unauthorized","statusCode":401}`
  (`crates/corvis/src/http/exception.rs:110-115` for the empty body, 375-379 for the type).
- `Response::redirect` percent-encodes the URL like `res.redirect` (`crates/corvis/src/http/response.rs:112-120`);
  passport's `strategy.redirect` does not encode, it calls `res.setHeader('Location', url)`.
- `PUBLIC_METADATA = "isPublic"` (`crates/corvis/src/lib.rs:251`, documented at 136-138), resolved with
  `Reflector::get_all_and_override` (`crates/corvis/src/common/reflector.rs:21`), the pattern of the hand-written
  guard in `examples/cats-app/src/auth/auth_guard.rs`.

### Differential fixtures today

Fixtures with their own Node package already exist: `crates/corvis/tests/fixtures/throttler/` has `package.json`
with exact versions (`@nestjs/*` 12.4.0), `package-lock.json`, `generate.mjs` (header comment with how to run it and
"servers bind to 127.0.0.1 only") and the recorded JSON replayed by `tests/throttler_headers.rs:878`; its
`node_modules` is ignored in the repository ignore file (lines 29-39 list the fixture folders).

## Goals / Non-Goals

**Goals**

- The registry, `Strategy`, `Authenticator`, `AuthGuard` and `PassportModule`, with the HTTP behavior of the original
  `AuthGuard` in every outcome, pinned by a recorded fixture.
- Guards that can redirect and append cookies on every response path, without changing any existing guard.
- Startup errors for configuration mistakes the original only reports per request or never.
- An API #41-#45 can build on without changing it.

**Non-Goals**

- Any concrete strategy kind or its cargo feature.
- Sessions and `log_in` (#44), WebSocket handshake authentication, a strategy macro.
- The `property` option, flash messages, `failureRedirect`.

## Decisions

### 1. `corvis::auth` is always compiled

The spike planned an `auth` cargo feature (decision 9) and, in Review note 9, a strategy tag on `Provider` that is
always present. The tag must name the `Strategy` trait, so the trait cannot sit behind a feature without a second,
type-erased tag. The core needs no new dependency (it uses `async-trait`, `serde_json` and `http`, already in
`crates/corvis/Cargo.toml`), so this change compiles `corvis::auth` unconditionally, like guards and `Reflector`.
Strategy kinds that pull crates keep their own features (#41 `auth-local` with the hashing crates; #42 decides
whether the JWT strategy lives under the existing `jwt` feature).

Alternatives: an `auth` feature on by default plus a type-erased tag (`fn(&Instance) -> Option<Box<dyn Any>>`
downcast in the auth module): more code for no dependency saving; an `auth` feature off by default: contradicts
Review note 9. The `auth` feature name is not created.

### 2. The `Strategy` trait receives the execution context

```rust
#[async_trait]
pub trait Strategy: Send + Sync + 'static {
    fn name(&self) -> &str;
    async fn authenticate(&self, ctx: &ExecutionContext, opts: &AuthenticateOptions) -> StrategyOutcome;
}

pub enum StrategyOutcome {
    Success { user: AuthenticatedUser, info: Option<Value> },
    Fail { challenge: Option<Value>, status: Option<u16> },
    Redirect { url: String, status: Option<u16> },
    Pass,
    Error(HttpException),
}
```

The spike passed `&Request`. The context is passed instead because `append_response_header` and `respond_with` live
on `HttpArgumentsHost` (decision 7) and because a strategy may read route metadata; `ctx.request()` gives the
request. Validator traits of #41/#42 keep receiving `&Request` (spike decision 2). `AuthenticateOptions` is a
`#[non_exhaustive]` struct with a JSON map of strategy-specific options (`AuthGuard::with_options(..)`), the
equivalent of the options object passed to `strategy.authenticate(req, options)`; the core reads none of them.

`AuthenticatedUser::new<T: Serialize + Clone + Send + Sync + 'static>(user: T) -> Result<Self, HttpException>`
serializes the JSON form once (an error is an internal 500, see the spec) and keeps a type-erased
`Arc<dyn Fn(&Request) + Send + Sync>` that inserts the typed value (spike decision 5).

### 3. Registration and the registry

- `Provider` gets `pub(crate) strategy: Option<StrategyTag>`, always present, where
  `StrategyTag { cast: fn(&Instance) -> Option<Arc<dyn Strategy>> }`; `Provider::strategy::<S: Injectable +
  Strategy>()` is `use_class::<S>(Token::named(format!("STRATEGY:{}", type_name::<S>())))` with a cast that, like
  `app_guard`, accepts `S` or an `Arc<dyn Strategy>` (testing overrides through `use_value_arc`).
- `Authenticator` is a core provider of `InternalCoreModule` (next to `HttpAdapterHost`, `container.rs:253-254`), so
  every module can inject it and `ModuleRef::get::<Authenticator>()` finds it. It holds a
  `OnceLock<StrategyRegistry>` (`by_name: HashMap<String, Arc<dyn Strategy>>` plus the default names).
- `init_steps` collects the tagged providers in the loop that installs global enhancers (`application.rs:513-546`),
  fails on a duplicate name naming both provider tokens, and sets the registry before `build_router`.
- `PassportModule::register(PassportModuleOptions::new().default_strategy("jwt"))` (or `.default_strategies([..])`)
  provides a `PassportModuleOptions` value; `init_steps` reads it with `instances_of::<PassportModuleOptions>()` (the
  way `ServeStaticOptions` is read, `application.rs:567-572`); two registrations with different defaults are a startup
  error.

Alternative: keep the registry in `AppShared`. Rejected: user guards and strategies need it through DI.

### 4. `Authenticator` is the port of `authenticate.js` with a callback

`Authenticator::authenticate(&self, ctx, names: &[String], opts) -> Result<AuthResult, HttpException>`:

- unknown name: `Err(internal 500)`, the original's `next(new Error('Unknown authentication strategy "x"'))`;
- for each name in order: `Success` returns `AuthResult::Success { user, info }` and sets the auth info to `info`;
  `Fail` records `AuthFailure { strategy, challenge, status }` and continues; `Error(e)` returns `Err(e)`; `Pass`
  returns `AuthResult::Pass` without touching the auth info (passport calls `next()`, so `createPassportContext`
  never assigns `authInfo`); `Redirect` validates the URL as a header value, builds the response (status, `Location`
  as given, `Content-Length: 0`, empty body), calls `respond_with` and returns `AuthResult::Redirected` (an invalid
  URL returns `Err(internal 500)`);
- all failed: the auth info is the single challenge (one strategy) or the array of challenges (several), as
  `allFailed` passes them to the callback; returns `AuthResult::Failed { failures }`.

`Authenticator` never stores the user: `@nestjs/passport` assigns `req.user` in the guard after `handleRequest`. A
user's guard does what it wants with the `AuthResult` (`AuthResult::store_user(req)` is provided for the success
case).

### 5. `AuthGuard`

```rust
AuthGuard::new("jwt")                       // one strategy
AuthGuard::chain(["jwt", "headerapikey"])   // tried in order
AuthGuard::default()                        // PassportModule default(s)
    .optional()                             // no user instead of 401
    .on_failure(|failures| UnauthorizedException::new("Token required"))
    .skip_public()                          // honor #[public]
    .with_options(json!({ "scope": ["email"] }))
```

`can_activate`: WebSocket context → decision 9; `skip_public()` and the route is public → `Ok(true)`; then
`Authenticator::authenticate` with the `Authenticator` found through `ctx.module_ref()` (bound once by the startup
check of decision 8, so the lookup is not repeated per request). `Success` → insert the typed user, `set_user`
equivalent with the pre-serialized JSON, `Ok(true)`; `Pass` → remove the JSON user, `Ok(true)`; `Failed` → optional:
remove the JSON user, `Ok(true)`; `on_failure`: `Err(f(&failures))`; otherwise `Err(UnauthorizedException::empty())`;
`Redirected` → `Ok(true)` (the router sends the stored response); `Err(e)` → `Err(e)`. `controls_access()` is `false`
for `.optional()`; `guard_name()` stays the type name (it returns `&'static str`). `AuthGuard` is in `corvis::auth`,
not in the prelude.

`User<T>`: `FromContext` reading the private typed-user slot (see Implementation notes 11), else
`Err(UnauthorizedException::empty())`.
`Request::auth_info() -> Option<Value>` reads a private `AuthInfo(Value)` extension.

### 6. Typed user removal

The original assigns `req.user = undefined` (pass) or `null` (optional failure). Corvis removes the JSON user, which
is what `Request::user()` reads. (Superseded by the security review fixes: the typed user is kept in a private slot
and removed with the JSON one; see Implementation notes 11.)

### 7. Guard responses on `HttpArgumentsHost`

The spike put `respond_with` and `append_response_header` on `Request`. Since then `set_response_header` landed on
`HttpArgumentsHost` with the WebSocket rule and the merge points; the new methods sit next to it so the three share
validation, storage and merging, and both return `false` in a WebSocket context.

- `respond_with(Response) -> bool`: stores `PendingResponse(Arc<Mutex<Option<Response>>>)` in the request
  extensions. `run_pipeline` checks it after each guard (`router/mod.rs:759-763`); if present it returns a new
  internal signal (an `Ok` variant or a private wrapper, decided in the implementation) so that `execute_route`
  skips `finalize` and the filters, applies `apply_enhancer_headers` and the appended headers, and returns it.
  If the guard returned `Ok(false)` or `Err` after storing, the stored response still wins and the discarded result is
  logged with the `ExceptionsHandler` logger, like `router/mod.rs:717-720`.
- `append_response_header(name, value) -> bool`: same validation as `set_response_header` (factor the checks of
  `context.rs:188-198` into one function); values go to a new `AppendedHeaders(Arc<Mutex<Vec<(HeaderName,
  HeaderValue)>>>)` extension, capped at 128 values per request (the existing constant). Merge, in a function called
  wherever `apply_enhancer_headers` is called: `Set-Cookie` values are always added, before the response's own
  (remove the response's values, append the recorded ones, re-append the response's); other names only when the
  response has no header with that name. For `#[res]` routes the recorded values are copied into the `Res` when it is
  created (`router/mod.rs:772-778`) and later calls write to it while headers are not sent, like
  `set_response_header` (`context.rs:215-221`).

Alternatives: change `CanActivate` to return an enum (breaking; rejected by the spike); give `HttpException` headers
(a catch-all filter would turn redirects into error bodies; rejected by the spike); make `set_response_header`
append for `Set-Cookie` (changes a specified behavior, `core/enhancers` "never replace", and leaves no way to append
other headers).

### 8. Startup check for strategy names via `CanActivate::as_any`

`CanActivate` gains `fn as_any(&self) -> Option<&dyn Any> { None }`, overridden by `AuthGuard`. After `build_router`
and next to the posture check (`application.rs:573-574`, before middleware `configure`), `init_steps` walks every
compiled route's guards, downcasts `AuthGuard`s, resolves their names (default ones through
`PassportModuleOptions`), binds the `Authenticator` into the guard (`OnceLock`), and collects every missing name with
its route (method and path) into one error. A user guard that wraps an `AuthGuard` can override `as_any` to return
the inner guard and get the same check; one that calls `Authenticator` directly gets the original's 500 at request
time.

Alternative: a generic `fn check_at_startup(&self, ..) -> Result<(), String>` hook on `CanActivate`. More general,
but it needs a startup-context type in the public API; `as_any` is the spike's choice and is enough here.

### 9. `AuthGuard` fails closed in a WebSocket context

Global guards apply to gateway messages by default, so a global `AuthGuard` reaches them. The original `AuthGuard`
does not work there either (its `getRequest` reads the HTTP request). Running strategies against the shared handshake
request would store a user on state shared by every later message of the connection, and redirects make no sense on
a socket. So `AuthGuard` returns `Ok(false)` (`WsException("Forbidden resource")`) without running a strategy and
logs one warning per guard. Apps keep authenticating sockets in `handle_connection` (as `examples/tertulia` does) or
call `apply_global_enhancers_to_gateways(false)`. Gateway support is out of scope.

### 10. Route posture

`AuthGuard` controls access; `.optional()` does not (`controls_access()` returns `false`), because it lets every
request through, which is the rule `guard.rs:27-31` states for that method.

### 11. Fixture package and the strategies fixture

Following `fixtures/throttler/`: `crates/corvis/tests/fixtures/auth/package.json` (private, `"type": "module"`, exact
versions: `@nestjs/common`, `@nestjs/core`, `@nestjs/platform-express` 12.4.0, `@nestjs/passport` 12.0.1, `passport`
0.7.0, `passport-strategy` 1.0.0, `reflect-metadata` 0.2.2, `rxjs` 7.8.2; later issues add their strategy packages),
`package-lock.json`, and `strategies.mjs`, which starts one app on `127.0.0.1` with custom `passport-strategy`
subclasses driven by an `x-outcome` header and writes `strategies.json`:

```json
{ "versions": { "@nestjs/passport": "12.0.1", "passport": "0.7.0" },
  "cases": [ { "name": "fail with status 400", "request": { "method": "GET", "path": "/single",
               "headers": { "x-outcome": "fail-400" } },
               "response": { "status": 401, "headers": { "www-authenticate": null }, "body": { ... } },
               "divergence": null } ] }
```

Cases: success with info; fail without arguments; fail with challenge and 400; error with `ForbiddenException`; error
with `Error`; redirect with default status and with 301; redirect URL with CR/LF (the original's 500); pass; chain
fail-then-success, error-then-success, all failed (auth info array, read through an optional guard); optional guard;
custom `handleRequest` message (for `on_failure`); global filter catching the 401; unknown strategy in a guard
(recorded 500, marked divergence: Corvis fails at startup, asserted in a separate test); `isPublic` skip in a global
guard written the usual way (`Reflector` check before `super.canActivate`). `crates/corvis/tests/auth_strategies.rs`
declares the same routes with equivalent Corvis strategies and replays every case.

## Risks / Trade-offs

- **Router change on every guarded request.** One extension lookup after each guard and one merge call per response.
  Mitigation: regression tests for unchanged guards, the corvis-parity check, and the existing
  `crates/corvis/tests/response_headers.rs` / `enhancer_order.rs` suites.
- **`Set-Cookie` reordering** in the merge touches a response a handler built. Only when cookies were appended; covered
  by the scenario tests.
- **Fail-closed `AuthGuard` on gateways** can surprise an app that adds a global `AuthGuard` and has gateways. The
  warning names the guard and the two ways out; the docs page states it.
- **`as_any` is public API** on `CanActivate`, with a default. It may be superseded by a richer introspection later.
- **Type-erased user** costs a closure and a JSON value per authenticated request (spike risk); acceptable.
- **Stale fixtures** when a Node package releases: versions are exact in `package.json` and the lockfile.

## Open questions

1. Should `AuthGuard` be injectable by type (`#[use_guards(AuthGuard)]` as the default-strategy guard)? Left out of
   scope; `AuthGuard::default()` covers it.
2. Fail-closed on gateways (decision 9) versus running strategies against the handshake: the maintainer may prefer a
   startup error when a global `AuthGuard` reaches gateways. The design picks the runtime denial plus warning because
   a startup error would block apps whose gateways authenticate in `handle_connection`.
3. Spike decision 9 is refined (decision 1): no `auth` feature. Confirm that the core in every build is acceptable.

## Review notes (2026-11-09, maintainer)

1. No `AuthGuard` injectable by type in this change; `AuthGuard::default()` covers the default strategy.
2. Gateways: the runtime denial plus a warning (decision 9) is accepted. The warning is logged once per gateway at
   startup, not per connection, and names the gateway and the guard.
3. The auth core compiles in every build (no `auth` feature): it adds no dependencies, and the strategy tag in
   `Provider` must always exist. Accepted.
4. Line references were taken at 6f5ff4d; check them against the current `dev` before editing.
5. The router changes (guard redirects and extra headers on every path) need the corvis-parity check before
   archiving, and a `security-reviewer` round: header injection through `append_response_header`, the 128 `Set-Cookie`
   bound, redirect targets built from request data.

## Implementation notes

Decisions and deviations recorded while implementing (line references of the design were checked against `2d63992`;
the router, `context.rs` and `application.rs` had moved by a few lines only).

1. **`Authenticator` is not bound into `AuthGuard`.** Decision 8 planned a `OnceLock` bound by the startup check. An
   instance guard written in `#[use_guards(..)]` can be shared by several applications of one process (tests build
   many), so a cached binding could point to another application's registry. The guard instead looks the core
   `Authenticator` up through `ctx.module_ref()` on each request (one map lookup). The startup check reads the registry
   from the container and does not mutate the guard.
2. **Gateway warning at startup** (Review note 2): `corvis::auth::warn_gateways` logs one `AuthGuard` warning per
   gateway when the guards of any of its handlers include an `AuthGuard` (found through `CanActivate::as_any`), after
   the gateway posture is computed. Nothing is logged per message. The spec requirement was updated to say so.
3. **Router signal for stored responses.** `run_pipeline` returns a private `Pipeline` enum (`Done(HandlerResult)` or
   `Stored(Response)`); `execute_route` returns a stored response after `apply_enhancer_headers(.., true)`, with no
   `finalize`, filters or `#[res]` merge. A guard that panics after storing a response answers the panic's 500: the
   panic is raised before the router can read the stored response.
4. **`apply_enhancer_headers` gained a flag.** On `#[res]` routes the appended values are copied into the `Res` when it
   is created (and written through by later calls), and `merge_into` already puts the `Res` cookies before the
   response's own; running the append merge again would duplicate them, so those paths pass `appended = false`.
   Every other path (success, exception, filter, stored response) passes `true`. Without appended headers the function
   behaves byte for byte as before.
5. **Later `append_response_header` calls on a `Res`** (an interceptor after the handler started) write `Set-Cookie`
   always, and any other name when the `Res` has no header with it or the name was recorded earlier in the request, so
   two appended `Link` values from a guard and an interceptor both reach a `#[res]` response.
6. **Optional guards in the fixture** override `handleRequest` with `user || null`, not `user ?? null`: a failed
   attempt hands the Nest callback `user === false`, which `??` keeps as `req.user = false`. Corvis leaves no user, the
   `null` of the fixture. The spec scenario names `user ?? null`; the observable behavior (no user) is the same.
7. **`chain` with one name.** The original passes an array for `AuthGuard(['a'])` and so reports an array auth info for
   one failed strategy; Corvis reports the single challenge when one name is given, whatever constructor built the
   guard. No fixture case covers it.
8. **`StrategyOutcome` is not `#[non_exhaustive]`** (the five outcomes are the contract, mirrored from passport).
9. **Scenarios implemented at different places than listed in tasks:** the "Two strategies of the same type with
   different names" scenario uses two types (a type reports one fixed name), covered in `tests/auth_guard.rs`. The
   discarded-result log test lives in its own binary (`tests/guard_responses_log.rs`) because the logger is
   process-wide.
10. **Left to the maintainer's session:** the corvis-parity check (tasks 1.5 and 6.6) and the `security-reviewer`
    round (6.7), as the implementation brief says.
11. **Security review fixes.** The review found nothing above low; each point is fixed with a regression test
    (`tests/auth_hardening.rs` and unit tests):
    - `respond_with` is valid only in the guard phase. The router installs a private `GuardPhase` flag on the request
      and turns it on around each `can_activate`; outside it (interceptor, handler, hand-made context) the call stores
      nothing and returns `false`, which the `Authenticator` turns into a 500 for a redirecting strategy, so an
      `AuthGuard` run from an interceptor can no longer let the handler run anonymously.
    - The typed user is kept in a private `AuthUserSlot` and read only by `User<T>`; `clear_user()` removes it with
      the JSON user. A foreign request extension of type `T` no longer satisfies `User<T>`, and the typed and JSON
      users agree after a `Pass` or an optional failure. This removes the divergence of decision 6 and means
      `Request::get::<T>()` no longer returns the typed user (use `User<T>`).
    - A redirect status must be 3xx; anything else is the 500 path.
    - `Authenticator` lost its public `Default` (construction is crate-private). The gateway warning now says that
      `apply_global_enhancers_to_gateways(false)` removes every global enhancer from gateways. `Debug` of
      `AuthenticatedUser`, `AuthResult` and `StrategyOutcome` prints the variant only, never the user, info or
      challenge.
    - Docs (English and Spanish): open-redirect warning, a line that appended headers are also sent on a later
      401/403, and the guard-phase rule of `respond_with`.
12. **Re-review follow-ups.** `Request::set_user` drops the private typed-user slot, so `request.user()` and `User<T>`
    never disagree after a later guard replaces the user (`User<T>` then answers 401 unless that guard is an
    `AuthGuard`; `AuthGuard` itself fills the slot and the JSON user through a crate-private path that does not clear
    it). The guard-phase flag is cleared by an RAII scope (`GuardPhase::enter`), so a panicking guard does not leave it
    on for exception filters. The return-to rule in the docs also excludes backslashes (a browser reads `/\evil.com`
    as `//evil.com`).
