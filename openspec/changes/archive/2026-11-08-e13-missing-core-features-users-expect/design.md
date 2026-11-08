# Design

## Context

Issue #23 is a spike. The documentation review listed six features that users of the original framework expect and
that Corvis does not have; the docs already say so, each with a workaround. This change investigates each one in the
code, decides a scope, and splits the work into one issue per feature (`new-issues.md`). It changes no code, docs or
specs.

Facts that hold for several features:

- The three port apps (consi, facten, houseplan) and their Node originals use none of these six features: no throttler,
  cache module, request scope, gateway, compression or TLS. consi only uses `ConfigModule::for_root`
  (`consi-backend/src/app.module.rs:29`). So none of the follow-up changes should move apidiff as long as new
  behavior is opt-in or only affects features the ports don't use.
- Global enhancers live in `RouterConfig` (`crates/corvis/src/router/mod.rs:238-246`: `global_guards`,
  `global_interceptors`, `global_pipes`, `global_filters`). `CorvisApplication::init` passes it to both
  `build_router` and `websockets::build_gateways` (`crates/corvis/src/application.rs:400-402`), but the gateway
  builder ignores it (`_cfg: &RouterConfig`, `crates/corvis/src/websockets.rs:544-547`).
- Responses get their Express defaults (`Content-Type`, `ETag`, `Content-Length`, 304) in
  `http::express::finalize` (`crates/corvis/src/http/express.rs:235`), called after the whole dispatch in
  `server::handle` (`crates/corvis/src/server.rs:358`) and in the test client (`crates/corvis/src/testing.rs:391`).
  Middleware runs before that step and still sees `Body::Json` / `Body::Text`.
- A guard cannot add response headers today: `HttpArgumentsHost::get_response` only returns a `Res` when the handler
  declares `#[res]` (`crates/corvis/src/common/context.rs:160-162`), and `run_pipeline` only creates that `Res` after
  the guards have run (`crates/corvis/src/router/mod.rs:613-627`). `HttpException` has no headers either
  (`crates/corvis/src/http/exception.rs:20-28`).

The sections below give, per feature: current state, proposed scope, size, dependencies and risks. Sizes follow
ROADMAP.md (S about one session, M two or three, L more).

## Goals / Non-Goals

**Goals:**
- One ready-to-file issue per feature, each small enough to become one OpenSpec change (M at most, except where noted).
- For each feature, a recommended scope with the main design choice made, so the follow-up `/opsx:propose` starts from
  a decision instead of a blank page.
- Record the dependencies between these issues and the rest of the roadmap (#14, #17, #25–#27, #8, #106).

**Non-Goals:**
- Implementing any of the features, or writing their spec deltas: each follow-up change writes its own.
- Features the docs review did not list (see "Out of scope" in proposal.md).

## Decisions

1. **One issue per feature, six issues, all children of epic #13.** The issue text lists six items; each touches a
   different part of the code (DI container, gateway runtime, server accept loop, response pipeline, three dynamic
   modules, throttler guard) and can be reviewed and verified alone. Alternative considered: group the "server" items
   (TLS + compression) and the "module" items (async variants + throttler headers) into two issues. Rejected: TLS and
   compression share no code, and the throttler headers need a new response-header mechanism that the async variants
   don't.
2. **No spec delta in this change.** There is no requirement common to all six features; each follow-up change
   writes the deltas for its capability (`core/di`, `ws/gateways`, `http/server`, `http/compression`, `core/modules`,
   `security/rate-limiting`). The spike is archived without spec updates (see Open questions).
3. **Opt-in or parity-neutral defaults.** Every proposed default either keeps today's behavior (TLS, compression,
   request scope, async variants) or changes it only for features the ports don't use (gateway enhancers, throttler
   headers). This keeps the corvis-parity check at 0 differences without special cases.
4. **Differential tests where an original library exists.** Compression is checked against the `compression`
   middleware on Express, the throttler headers against `@nestjs/throttler`, gateway filters and global guards against
   the original framework's WebSocket gateway behavior. TLS and the async variants have no observable output to diff
   beyond what normal tests cover.

### Feature 1 — REQUEST scope

**Current state.**
- `Scope` has two variants, `Default` and `Transient` (`crates/corvis/src/di/provider.rs:30-38`); its doc comment
  still mentions a `Request` scope (already reported in #14).
- `#[injectable(scope = "request")]` is a compile error: `parse_injectable_args` rejects any argument containing
  `request` with "request-scoped providers are not supported; use transient or default scope"
  (`crates/corvis-macros/src/injectable.rs:158-173`).
- The container caches every non-transient instance by `(module, token)` (`crates/corvis/src/di/container.rs:329-372`)
  and instantiates all non-transient providers and every controller at startup (`container.rs:402-420`).
- Routes bind their handlers to the controller instance once, at startup: `build_router` takes the stored instance and
  calls `(def.routes)(&instance)` (`crates/corvis/src/router/mod.rs:285-296`). Enhancers are resolved once per route.
- Docs: `docs/fundamentals/injection-scopes.md` (status `partial`) and `docs/recipes/async-local-storage.md` (status
  `unavailable`) document the absence and the workaround (request extensions `req.insert` / `req.get`, `FromContext`
  parameters); `docs/migration-guide.md:58` and `docs/faq/common-errors.md:81` repeat it.

**Proposed scope (recommended: explicit request scope, no implicit bubbling).**
- Add `Scope::Request` and accept `#[injectable(scope = "request")]` and `Provider::...scope(Scope::Request)`.
- A request-scoped provider is built at most once per HTTP request, on first use, and cached in the request's
  extension store; its own dependencies may be singletons, transients or other request-scoped providers of the same
  request.
- The current request is injectable into a request-scoped provider (a `Request` field resolved from the request being
  served, the equivalent of the original `REQUEST` token).
- Consumption is explicit: a handler, guard, interceptor or pipe receives one through a `FromContext` parameter type
  (for example `Scoped<T>`) or through `ModuleRef::resolve_for(&request)`. Injecting a request-scoped provider into a
  singleton or a controller field is a **startup error** naming both providers, instead of the original framework's
  silent scope bubbling.
- Not available in gateways, standalone contexts, lifecycle hooks or scheduled jobs (there is no request): resolving
  outside a request is an error.

**Alternative considered: full bubbling, as in the original.** Any provider or controller that depends on a
request-scoped provider becomes request-scoped; controllers whose graph contains one are rebuilt per request, which
means moving `(def.routes)(&instance)` from `build_router` to request time for those controllers, threading a
per-request cache through `ResolveCtx` (`crates/corvis/src/di/injectable.rs:58-64`), and skipping them in
`instantiate_all`. Closer to the original and easier for a mechanical migration, but size L, a hidden per-request
allocation cost on whole controller graphs, and a deep change to the container right after #15 adds its tests.
Recommended as a later step only if ports need constructor injection of request-scoped services.

**Size.** M (explicit). L (bubbling).

**Dependencies.** After #14 (fixes the `Scope` doc comment and the DI messages) and #15 (DI test coverage, so the
container change has a safety net). Related to #9 (typed API) for the extractor's shape.

**Risks.** Mis-caching an instance across requests would leak one user's data to another: the per-request cache must
live in the request and never in the container; a regression test runs concurrent requests and asserts distinct
instances. Users migrating code that relies on bubbling get a startup error and must move the dependency to a handler
parameter; the error message must say how.

### Feature 2 — Exception filters and global guards on gateways

**Current state.**
- Gateway enhancers come only from the gateway attribute and the handler attributes. `WsEnhancers` has `guards`,
  `interceptors`, `pipes` and `metadata`, no filters (`crates/corvis/src/discovery.rs:28-35`). `build_gateways`
  resolves them per handler (`crates/corvis/src/websockets.rs:544-580`) and never reads the global ones it receives.
- `#[use_filters]` below `#[websocket_gateway]` is a compile error, "#[use_filters] is not supported on WebSocket
  gateways" (`crates/corvis-macros/src/gateway.rs:60-61`). On a `#[subscribe_message]` method it is not one of the
  attributes `#[handlers]` recognizes (`crates/corvis-macros/src/handlers.rs:92-94`), so it is left on the method.
- Errors from guards, interceptors, pipes or handlers become an `exception` frame through a fixed mapping
  (`exception_payload`, `websockets.rs:127`; sent in `dispatch`, `websockets.rs:780-822`). Panics are caught and
  reported as an internal error.
- `run_handler` runs guards → interceptors → pipes → handler (`websockets.rs:697-730`); a guard returning `false`
  yields `WsException::new("Forbidden resource")`.
- Docs: `docs/websockets/exception-filters.md:7` (status `partial`), `docs/websockets/gateways.md:285-287`,
  `docs/websockets/guards.md:30`, `docs/websockets/pipes.md:19`, `docs/websockets/interceptors.md:7`,
  `docs/migration-guide.md:62`, `docs/deployment.md:174` and `docs/security/rate-limiting.md:123` all state that
  global enhancers and filters do not reach gateways.
- `examples/cats-app` registers `ThrottlerGuard`, `AuthGuard` and `RolesGuard` as global guards
  (`examples/cats-app/src/app_module.rs:31-33`) and also has a public `/chat` gateway (`examples/cats-app/src/chat.rs:7`)
  whose doc comment tells users to connect with `websocat` and no token.

**Proposed scope.**
- Exception filters on gateways: allow `#[use_filters]` below `#[websocket_gateway]` and on `#[subscribe_message]`
  methods, using the same `ExceptionFilter` trait as HTTP (`crates/corvis/src/common/filter.rs:32-37`). Order:
  handler filters, then gateway filters, then global filters, then the built-in mapping, with `catches()` deciding as
  in HTTP (`handle_exception`, `router/mod.rs:745-761`). The `Response` a filter returns is translated to the frame:
  its JSON body becomes the `data` of the `exception` frame (the `id` of the incoming frame is still echoed); an
  `Empty` body means the filter already answered (for example with `client.emit`) and nothing else is sent.
- Global guards and global filters (`use_global_guards`, `use_global_filters`, `Provider::app_guard`,
  `Provider::app_filter`) apply to gateway handlers, before the gateway's own guards, as in the original framework
  in a non-hybrid application. Guards can tell transports apart with `ctx.get_type()`.
- An application switch to keep today's behavior (global enhancers HTTP-only), so existing apps can migrate gradually.

**Alternatives considered.**
- A separate `WsExceptionFilter` trait whose `catch` returns nothing and emits on the client. Closer to how filters are
  written in the original, but it splits filters in two and HTTP-only global filters (such as `OrmExceptionFilter`)
  would not work on gateways at all. Rejected in favor of translating the returned `Response`.
- Keep global guards HTTP-only by default and add an opt-in. Safer for existing apps but leaves the default insecure
  (a global auth guard silently does not protect messages), which conflicts with the route-posture epic (#24). Default
  on, with the opt-out, is recommended; see Open questions.

**Size.** M.

**Dependencies.** Should land before #27 (route posture for WebSockets and SSE), which needs to know which guards
protect a gateway handler; #25 (route posture design) should reference this decision. #17 fixes the order of
`Provider::app_filter` for HTTP; gateway filters must reuse the corrected order. E2 (#35, IoAdapter for gateways) will
reuse the same gateway pipeline.

**Risks.** Behavior change for apps with global guards and gateways: `examples/cats-app`'s global `AuthGuard` reads a
bearer token from the handshake request and would reject every `/chat` message (browsers cannot set `Authorization`
on a WebSocket handshake). The change must update that example (mark the gateway public or make the guard skip
`ContextType::Ws`) and the migration notes. A global `ThrottlerGuard` would start counting WebSocket messages per
handshake IP; see feature 6. Global pipes and interceptors stay HTTP-only, which keeps an asymmetry; see Open
questions.

### Feature 3 — TLS

**Current state.**
- The server accepts plain TCP only: `serve` takes a `tokio::net::TcpListener`, wraps each stream in `TimedIo` and
  hands it to hyper-util's `auto::Builder` (`crates/corvis/src/server.rs:580-647`). `TimedIo<S>` is generic over the
  stream (`server.rs:435-445`).
- `listen`, `listen_in_background` and `bind` (`crates/corvis/src/application.rs:438-491`) only bind TCP;
  `get_url` always returns `http://` (`application.rs:494-499`). `ServerOptions` has no TLS field
  (`server.rs:40-62`); `h2c` controls cleartext HTTP/2.
- `Request` has no `protocol()` or `secure()` method (no match in `crates/corvis/src/http/request.rs`).
- Gateway connections are generic over the stream (`run_connection<S: AsyncRead + AsyncWrite>`,
  `crates/corvis/src/websockets.rs:853-855`), so `wss://` would work once the upgrade comes over TLS.
- `rustls` 0.23 with the `ring` provider is already in the dependency graph through corvis-orm's `tls-rustls` feature
  (`Cargo.lock:1631-1640`); `tokio-rustls` is not.
- Docs: `docs/faq/multiple-servers.md` (status `partial`), `docs/deployment.md:63` and `docs/http/performance.md:19`
  say Corvis does not terminate TLS and recommend a reverse proxy.

**Proposed scope.**
- A `tls` cargo feature (not default) with `tokio-rustls` and the `ring` provider.
- `TlsOptions` built from PEM files, PEM bytes, or a ready `rustls::ServerConfig`; ALPN `h2` and `http/1.1`.
- `app.listen_tls(addr, tls)` and `app.listen_tls_in_background(addr, tls)`, sharing `serve` with the plain listeners;
  the TLS handshake runs inside the per-connection task with its own timeout, never in the accept loop;
  `max_connections`, header and keep-alive timeouts apply as for plain TCP.
- `get_url` returns `https://` for a TLS listener.

**Alternatives considered.**
- An `https_options` field on `CorvisApplicationOptions` that turns `listen` into HTTPS, mirroring the original's
  `httpsOptions`. Easier to map during a migration, but one setting for every listener makes "HTTPS on 443 plus an
  internal HTTP port" (`docs/faq/multiple-servers.md`) impossible. Could be added later as sugar over `listen_tls`.
- `native-tls`/OpenSSL. Rejected: needs OpenSSL on Linux images (`docs/deployment.md:166` already promises no OpenSSL
  for the ORM's TLS) and behaves differently per platform.
- The `aws-lc-rs` provider (rustls' default). Rejected for now: it needs CMake and NASM to build on Windows, and `ring`
  is already compiled for corvis-orm.

**Size.** M.

**Dependencies.** None blocking. #106 (smaller and faster dev builds): the feature must stay off by default. #6 (CI):
add a job with `--features tls`. `req.protocol()` / `req.secure()` are a separate gap (see proposal.md, Out of scope).

**Risks.** A TLS handshake done in the accept loop would let one slow client stall every new connection: covered by
doing it in the spawned task, with a timeout, and a regression test. Certificate rotation needs a restart (no hot
reload in scope). Supply-chain: new crates must pass `cargo deny` (`deny.toml` sets `unmaintained = "all"`), so PEM
parsing should use `rustls-pki-types`' PEM support rather than the separate `rustls-pemfile` crate if the latter is
flagged as unmaintained.

### Feature 4 — Compression

**Current state.**
- No response compression anywhere in `crates/corvis/src` (no gzip, deflate or brotli encoder).
- Compressed request bodies are rejected: the body parser answers `415 unsupported content encoding` for any
  `Content-Encoding` other than `identity` (`crates/corvis/src/http/body_parser.rs:108-117`).
- `finalize` turns JSON/text bodies into bytes and sets `Content-Length` and `ETag`
  (`crates/corvis/src/http/express.rs:235-298`); static files are `Body::Stream`
  (`crates/corvis/src/http/serve_static.rs:662-665`), as are SSE responses (`Body::Stream`,
  `crates/corvis/src/http/response.rs:45-52`).
- Docs: `docs/http/compression.md` (status `unavailable`) recommends compressing in the reverse proxy;
  `docs/faq/raw-body.md:53-56` describes the 415 for compressed bodies.

**Proposed scope (response compression only).**
- `app.enable_compression(CompressionOptions)`, an application setting applied after `finalize` in `server::handle`
  and in the test client, so it sees the final bytes, `Content-Type`, `ETag` and status.
- Behavior follows the `compression` middleware for Express, verified with a differential test against it: encoding
  negotiation from `Accept-Encoding` (q-values), a size threshold (1 KiB default), only compressible content types, no
  compression for `HEAD`, for responses that already have `Content-Encoding` or `Cache-Control: no-transform`, and for
  204/304; `Vary: Accept-Encoding` added; `Content-Length` removed when the body is compressed.
- gzip and deflate; brotli if the version of `compression` used as reference supports it (see Open questions).
- Streamed bodies (static files) are compressed chunk by chunk; `text/event-stream` is never compressed, so SSE events
  are not held in an encoder buffer.
- Off by default, behind a `compression` cargo feature.

**Alternatives considered.**
- A `CorvisMiddleware` like the original `app.use(compression())`. Rejected: middleware runs before `finalize`, so it
  would see `Body::Json` without `Content-Length` or `ETag` and would have to duplicate the serialization step.
- Decompressing request bodies in the same issue. Kept out: it is a body-parser change with its own risk
  (decompression bombs) and its own reference library (`body-parser`'s inflate); listed in Out of scope.

**Size.** M.

**Dependencies.** #106 (keep the feature out of default builds). #8 (compatible vs strict mode) does not block it.
Interacts with the ETag logic in `finalize`: the differential test must cover what the reference does to `ETag`.

**Risks.** BREACH-style attacks when a compressed response mixes secrets with reflected input: document it and keep
compression opt-in. CPU cost on large bodies: bounded compression levels (brotli quality well below 11) and the size
threshold. Proxies that also compress: the `Content-Encoding` check avoids double compression.

### Feature 5 — Async variants for Config, Cache and Throttler

**Current state.**
- `ConfigModule::for_root(options)` builds `ConfigService` inside a `Provider::use_factory` that is already async
  (`crates/corvis/src/config.rs:275-284`), but options are a value and `load` factories are synchronous
  (`ConfigFactory.factory: Arc<dyn Fn() -> Value>`, `config.rs:23-27`; applied in `merge_factory`, `config.rs:187`).
- `CacheModule::register(options)` builds the `CacheManager` eagerly and registers it under two tokens,
  `Token::of::<CacheManager>()` and `CACHE_MANAGER` (`crates/corvis/src/modules/cache.rs:154-163`).
- `ThrottlerModule::for_root(throttlers)` and `for_root_storage(storage)` take values; the module is always global
  (`crates/corvis/src/modules/throttler.rs:143-154`).
- Existing async variants disagree on their signature: `JwtModule::register_async(global, factory)`
  (`crates/corvis/src/jwt.rs:342`), `MulterModule::register_async(factory)` (`crates/corvis/src/multer/mod.rs:268`),
  `ScheduleModule::for_root_async(imports, factory)` (`crates/corvis/src/modules/schedule/mod.rs:573`),
  `OrmModule::for_root_async(is_global, factory)` (`crates/corvis-orm/src/module.rs:223`). All take
  `F: Fn(ResolveCtx) -> Fut`, `Fut: Future<Output = corvis::Result<Options>>`. `DynamicModule::import` is public
  (`crates/corvis/src/di/module.rs:131`), so callers can add imports to any returned module.
- Docs: `docs/fundamentals/dynamic-modules.md:107-156` explains the pattern and lists the built-in modules; Config,
  Cache and Throttler have no async variant there.

**Proposed scope.**
- `ConfigModule::for_root_async(is_global, factory)` where the factory returns `ConfigModuleOptions`.
- `CacheModule::register_async(is_global, factory)` where the factory returns `CacheOptions`; both tokens resolve to
  the same `CacheManager` (one factory provider plus a `Provider::use_existing` alias).
- `ThrottlerModule::for_root_async(factory)` where the factory returns anything `Into<ThrottlerStorage>` (with
  `From<Vec<ThrottlerOptions>>`), so both `for_root` and `for_root_storage` shapes are covered; still global.
- Naming follows each module's synchronous method (`for_root` → `for_root_async`, `register` → `register_async`), and
  the signature convention is: `(is_global, factory)` when the module has an `is_global` option, `(factory)` when the
  module is always global; extra imports through `.import(...)`.

**Alternatives considered.**
- Read `is_global` from the factory's result. Impossible: the module graph (and so globality) is fixed before any
  factory runs.
- Async `load` factories for `ConfigModule` (the original accepts factories that return a promise). Useful to read
  configuration from a secrets service, but not what the issue asks; listed under Open questions as an optional
  extension of the same issue.

**Size.** S.

**Dependencies.** None. #18 (consistent units) touches `CacheOptions.ttl`; whichever lands second rebases.

**Risks.** Low. A factory error must surface as the existing factory error message (`docs/faq/common-errors.md:71`).
Making `Into<ThrottlerStorage>` generic must not make type inference fail for closures returning `Vec<_>` (test both
shapes).

### Feature 6 — `Retry-After` and `X-RateLimit-*` headers in the throttler

**Current state.**
- `ThrottlerGuard::can_activate` increments a counter per throttler and returns
  `HttpException::new("ThrottlerException: Too Many Requests", 429)` when `hits > limit`
  (`crates/corvis/src/modules/throttler.rs:166-181`). `ThrottlerStorage::increment` already returns
  `(hits, time_to_expire)` (`throttler.rs:102`), but the guard discards the time (`let (hits, _)`, `throttler.rs:176`).
- No headers are sent; `docs/security/rate-limiting.md:122` lists that as a limitation.
- A guard cannot set response headers on success or on error (see Context).

**Proposed scope.**
- A small, general mechanism for enhancers to add response headers: a header set on the request's execution context
  (for example `ctx.switch_to_http().set_response_header(name, value)`) is merged into the final response in
  `execute_route` on success **and** on error (also when a filter builds the response, without overriding headers the
  filter set), and copied into the `Res` when the route uses `#[res]` (created in `run_pipeline`). This mirrors
  Express, where headers set on `res` by a guard survive the exception response.
- `ThrottlerGuard` sets, per throttler, the headers of `@nestjs/throttler`: `X-RateLimit-Limit`,
  `X-RateLimit-Remaining`, `X-RateLimit-Reset` on every counted request, and `Retry-After` on a 429, with a `-<name>`
  suffix for throttlers not named `default`; values and units (seconds) confirmed by a differential test against
  `@nestjs/throttler`.
- An option to turn the headers off.
- In a WebSocket context (once feature 2 lands) the guard sets no headers.

**Alternatives considered.**
- `HttpException::with_header` for the 429 only. Simpler, but does not cover the `X-RateLimit-*` headers on
  successful responses, which is most of the feature. It could still be added later for user exceptions.
- Making `get_response()` always return a `Res`. Changes the meaning of `#[res]` passthrough for every route; too wide
  for this issue.

**Size.** M (the header mechanism plus the throttler and its differential test).

**Dependencies.** #17 (enhancers and routing edge cases) touches the same dispatch path (`execute_route`, filter order):
land after it or rebase. Feature 2 decides whether the throttler runs on gateways.

**Risks.** Header merge order mistakes could override a header the handler or a filter set on purpose: the merge must
never replace an existing header, with a test for each path (`Ok`, `Err` without filter, `Err` with filter, `#[res]`
passthrough, `#[res]` handled). `examples/cats-app` uses `ThrottlerGuard` globally, so its responses gain headers;
check its tests.

## Risks / Trade-offs

- [The spike's recommendations may be wrong in details that only show up when implementing] → each follow-up issue
  still goes through `/opsx:propose` and review; this design is the starting point, not the spec.
- [Six new issues dilute Phase 0] → the suggested order in `new-issues.md` keeps only feature 2 before route posture
  (Phase 0.5) and moves the rest after it.
- [Changing defaults for gateways and the throttler surprises existing users] → both are called out in the issues,
  require migration notes in the docs, and the ports are unaffected.

## Open questions

1. **Archiving without a spec delta.** This change has no deltas, so `.openspec.yaml` sets `skip_specs: true`
   (`openspec validate` accepts it); archive it with `openspec archive e13-missing-core-features-users-expect
   --skip-specs` (or the `/opsx:archive` equivalent). Confirm that is the convention for spikes.
2. **Request scope shape.** Explicit request scope with a startup error (recommended, M) or full bubbling like the
   original (L)? Does any port or planned migration (E12) need constructor injection of request-scoped services?
3. **Gateway defaults.** Should global guards and filters reach gateways by default (recommended, with an app-level
   opt-out), or only when enabled? Should global pipes and interceptors follow in the same issue for symmetry, or stay
   HTTP-only?
4. **Gateway filter contract.** Is "the returned `Response`'s JSON body becomes the `exception` frame data; an empty
   body sends nothing" acceptable, or should gateways get their own filter trait?
5. **TLS entry point.** `listen_tls(addr, tls)` (recommended) only, or also an `https_options` factory option for a
   one-to-one migration mapping?
6. **Compression reference version.** Which version of the `compression` package is the reference for the
   differential test (it decides whether brotli is in scope and how `ETag` is handled)? Default features: should
   `compression` and `tls` stay out of the default feature set?
7. **Config async load factories.** Include async `load` factories (`register_as_async`) in the async-variants issue,
   or file them separately?
8. **Throttler headers default.** On by default, as in the original (recommended), or opt-in to avoid changing
   responses of existing apps?
9. **Phase placement.** The suggested placement in `new-issues.md` (gateway enhancers in Phase 0.4, before route
   posture; the rest in a new phase after Phase 0.6) needs the maintainer's confirmation in #111.

## Review notes (2026-11-08)

- REQUEST scope: the explicit version (M) with a startup error when a request-scoped provider is injected into a
  singleton; full implicit propagation is not planned.
- Gateways: follow the original model as the spike describes it (in a non-hybrid app, global guards and filters apply
  to gateway handlers), with the application switch to keep them HTTP-only; the change verifies it with a differential
  test. Global pipes and interceptors on gateways go in the same issue only if the original applies them the same way.
- Gateway exception filters reuse `ExceptionFilter` and translate its response into the `exception` frame.
- TLS: `listen_tls` / `listen_tls_in_background` only. `tls` and `compression` stay out of the default features.
- Compression: the latest 1.x of the `compression` middleware is the reference.
- No async `load` factories in issue 5.
- Throttler headers on by default, as in the original.
- Placement in #111: issue 2 in phase 0.4 (before route posture for gateways, #27); issues 5, 6, 1, 3, 4 in a new
  phase 0.7 after phase 0.6.
- Spikes are archived with `--skip-specs`.
