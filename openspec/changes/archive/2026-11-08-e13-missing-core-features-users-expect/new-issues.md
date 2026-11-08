# New issues from the #23 spike

Six issues, one per feature named in #23. Each is a sub-issue of epic #13, milestone "Phase 0 — Foundations", project
"Corvis Roadmap" (Todo). Bodies follow the style of the existing E13 issues; the full analysis is in `design.md` of
this change, by feature number.

Suggested order in #111:

- **Phase 0.4** (with #8, #23, #9): issue 2 (gateway enhancers), because #25/#27 (route posture) need to know which
  guards protect a gateway handler.
- **New phase after Phase 0.6** ("Core features users expect"): issues 5, 6, 1, 3, 4, in that order (smallest and
  most used first; TLS and compression have a reverse-proxy workaround).

| # | Title | Size | Priority | Labels | OpenSpec change |
|---|---|---|---|---|---|
| 1 | REQUEST-scoped providers | M | medium | `area:core`, `type:task`, `size:M`, `priority:medium` | `e13-request-scoped-providers` |
| 2 | Exception filters and global guards on gateways | M | high | `area:ws`, `area:security`, `type:task`, `size:M`, `priority:high` | `e13-exception-filters-and-global-guards-on-gateways` |
| 3 | TLS termination in the built-in server | M | medium | `area:http`, `area:security`, `type:task`, `size:M`, `priority:medium` | `e13-tls-termination-in-the-built-in-server` |
| 4 | Response compression | M | low | `area:http`, `type:task`, `size:M`, `priority:low` | `e13-response-compression` |
| 5 | Async registration for Config, Cache and Throttler | S | medium | `area:core`, `type:task`, `size:S`, `priority:medium` | `e13-async-registration-for-config-cache-and-throttler` |
| 6 | Rate limit headers in the throttler | M | medium | `area:http`, `area:security`, `type:task`, `size:M`, `priority:medium` | `e13-rate-limit-headers-in-the-throttler` |

---

## 1. REQUEST-scoped providers

**Labels:** `area:core`, `type:task`, `size:M`, `priority:medium`

**Body:**

```markdown
`#[injectable(scope = "request")]` is a compile error (`corvis-macros/src/injectable.rs`) and `Scope` only has `Default` and `Transient` (`di/provider.rs`). Code that keeps per-request state in a provider (current user, tenant, request id) has to be rewritten with request extensions and `FromContext` (docs: `fundamentals/injection-scopes`, `recipes/async-local-storage`).

- Add `Scope::Request`: one instance per HTTP request, built on first use and cached in the request, never in the container.
- The current `Request` is injectable into a request-scoped provider (the `REQUEST` token of the original).
- Explicit consumption: a `FromContext` parameter (for example `Scoped<T>`) in handlers, guards, interceptors and pipes, and `ModuleRef::resolve_for(&request)`.
- Injecting a request-scoped provider into a singleton or a controller is a startup error naming both, instead of the original's silent scope bubbling (full bubbling, size L, is the alternative; see the #23 spike).
- Resolving outside a request (gateways, standalone context, hooks, scheduled jobs) is an error.
- Regression test: concurrent requests get distinct instances; one request reuses its instance.
- Docs: injection scopes, async local storage recipe, migration guide, common errors (EN + ES).

Depends on #14 and #15. Design notes: #23 spike, feature 1.

**Epic:** #13
**OpenSpec change:** `e13-request-scoped-providers` (create it with `/opsx:propose` when starting; it links back to this issue)
**Definition of done:** see [ROADMAP.md](../blob/main/ROADMAP.md#definition-of-done).
```

---

## 2. Exception filters and global guards on gateways

**Labels:** `area:ws`, `area:security`, `type:task`, `size:M`, `priority:high`

**Body:**

```markdown
Gateways ignore global enhancers: `build_gateways` receives the router config with the global guards and filters and does not use it (`websockets.rs`). `#[use_filters]` on a gateway is a compile error (`corvis-macros/src/gateway.rs`). A global `AuthGuard` therefore does not protect WebSocket messages, and error frames can only be shaped with `WsException::object` or an interceptor (docs: `websockets/exception-filters`, `websockets/gateways#global-enhancers`).

- `#[use_filters]` below `#[websocket_gateway]` and on `#[subscribe_message]` methods, with the same `ExceptionFilter` trait as HTTP. Order: handler, gateway, global, then the built-in mapping. The JSON body of the `Response` a filter returns becomes the `data` of the `exception` frame (the frame `id` is still echoed); an empty body means the filter already answered.
- Global guards and global filters (`use_global_guards`, `use_global_filters`, `Provider::app_guard`, `Provider::app_filter`) apply to gateway handlers, before the gateway's own guards, like the original in a non-hybrid app.
- An application switch to keep global enhancers HTTP-only, for gradual migration.
- Update `examples/cats-app`: its global `AuthGuard` would reject every `/chat` message (mark the gateway public or skip `ContextType::Ws`).
- Differential test against the original framework's gateway behavior for filter order and frames.
- Docs: websockets exception filters, gateways, guards; migration guide; deployment checklist; rate limiting limitations (EN + ES).

Do before #27 (route posture for WebSockets and SSE); reuse the filter order fixed in #17. Design notes: #23 spike, feature 2.

**Epic:** #13
**OpenSpec change:** `e13-exception-filters-and-global-guards-on-gateways` (create it with `/opsx:propose` when starting; it links back to this issue)
**Definition of done:** see [ROADMAP.md](../blob/main/ROADMAP.md#definition-of-done).
```

---

## 3. TLS termination in the built-in server

**Labels:** `area:http`, `area:security`, `type:task`, `size:M`, `priority:medium`

**Body:**

```markdown
The server only accepts plain HTTP/1.1 and h2c (`server.rs`, `application.rs`); HTTPS needs a reverse proxy (docs: `faq/multiple-servers`, `deployment`). Small deployments and internal services want to serve HTTPS directly.

- A `tls` cargo feature (off by default) with `tokio-rustls` and the `ring` provider (already in the graph through corvis-orm).
- `TlsOptions` from PEM files, PEM bytes or a `rustls::ServerConfig`; ALPN `h2` and `http/1.1`.
- `app.listen_tls(addr, tls)` and `app.listen_tls_in_background(addr, tls)`, sharing the accept loop with the plain listeners; `max_connections` and the timeouts apply.
- The TLS handshake runs in the per-connection task with its own timeout, never in the accept loop (regression test: a client that never finishes the handshake does not block other connections).
- `get_url()` returns `https://` for a TLS listener; gateways work over `wss://`.
- `cargo deny` passes with the new crates.
- Docs: HTTPS and multiple servers, deployment, HTTP adapters (EN + ES).

Out of scope: certificate hot reload, mTLS, ACME. Design notes: #23 spike, feature 3.

**Epic:** #13
**OpenSpec change:** `e13-tls-termination-in-the-built-in-server` (create it with `/opsx:propose` when starting; it links back to this issue)
**Definition of done:** see [ROADMAP.md](../blob/main/ROADMAP.md#definition-of-done).
```

---

## 4. Response compression

**Labels:** `area:http`, `type:task`, `size:M`, `priority:low`

**Body:**

```markdown
Corvis does not compress responses (docs: `http/compression`, which recommends the reverse proxy). Apps that used the `compression` middleware lose it when they migrate.

- `app.enable_compression(CompressionOptions)`, applied after the Express response defaults (`express::finalize`) in the server and in the test client, so it sees the final bytes, `Content-Type`, `ETag` and status.
- Same behavior as the `compression` middleware on Express, checked with a differential test against it: `Accept-Encoding` negotiation with q-values, 1 KiB threshold, compressible types only, no compression for `HEAD`, 204/304, existing `Content-Encoding` or `Cache-Control: no-transform`; `Vary: Accept-Encoding`; no `Content-Length` on compressed bodies.
- gzip and deflate (brotli if the reference version supports it).
- Streamed bodies (static files) compressed chunk by chunk; `text/event-stream` never compressed.
- A `compression` cargo feature, off by default; compression itself off unless enabled.
- Docs: compression page, SSE note (EN + ES), including the BREACH caveat.

Out of scope: decompressing request bodies (today a 415). Design notes: #23 spike, feature 4.

**Epic:** #13
**OpenSpec change:** `e13-response-compression` (create it with `/opsx:propose` when starting; it links back to this issue)
**Definition of done:** see [ROADMAP.md](../blob/main/ROADMAP.md#definition-of-done).
```

---

## 5. Async registration for Config, Cache and Throttler

**Labels:** `area:core`, `type:task`, `size:S`, `priority:medium`

**Body:**

```markdown
`ConfigModule`, `CacheModule` and `ThrottlerModule` only take options as values (`config.rs`, `modules/cache.rs`, `modules/throttler.rs`), so their options cannot come from `ConfigService` or another provider. JWT, Multer, Schedule and the ORM already have async variants, with different signatures.

- `ConfigModule::for_root_async(is_global, factory)` → `ConfigModuleOptions`.
- `CacheModule::register_async(is_global, factory)` → `CacheOptions`; `Arc<CacheManager>` and `CACHE_MANAGER` resolve to the same instance.
- `ThrottlerModule::for_root_async(factory)` → anything `Into<ThrottlerStorage>` (a `Vec<ThrottlerOptions>` or a configured storage); still global.
- Convention, documented: `(is_global, factory)` when the module has an `is_global` option, `(factory)` when it is always global; extra imports with `.import(...)`.
- Tests: each variant reads a value from `ConfigService`; a factory error aborts bootstrap with the usual factory error.
- Docs: configuration, caching, rate limiting, dynamic modules table (EN + ES).

Design notes: #23 spike, feature 5.

**Epic:** #13
**OpenSpec change:** `e13-async-registration-for-config-cache-and-throttler` (create it with `/opsx:propose` when starting; it links back to this issue)
**Definition of done:** see [ROADMAP.md](../blob/main/ROADMAP.md#definition-of-done).
```

---

## 6. Rate limit headers in the throttler

**Labels:** `area:http`, `area:security`, `type:task`, `size:M`, `priority:medium`

**Body:**

```markdown
`ThrottlerGuard` answers 429 without `Retry-After` and never sends `X-RateLimit-*` (`modules/throttler.rs`; docs: `security/rate-limiting#limitations`). It cannot today: a guard has no way to add response headers unless the handler declares `#[res]`, and `HttpException` carries no headers.

- A response-header mechanism for enhancers: headers set through the execution context are merged into the final response on success and on error (also when a filter builds the response), never overriding a header already set, and copied into `Res` for `#[res]` routes.
- `ThrottlerGuard` sends `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset` and, on 429, `Retry-After`, with a `-<name>` suffix for throttlers not named `default`, as `@nestjs/throttler` does; values checked with a differential test against it.
- An option to turn the headers off.
- No headers in a WebSocket context.
- Tests for each merge path: `Ok`, `Err` without filter, `Err` with filter, `#[res]` passthrough, `#[res]` handled.
- Docs: rate limiting (EN + ES), and the new header API where guards and interceptors are documented.

Land after #17 (same dispatch path). Design notes: #23 spike, feature 6.

**Epic:** #13
**OpenSpec change:** `e13-rate-limit-headers-in-the-throttler` (create it with `/opsx:propose` when starting; it links back to this issue)
**Definition of done:** see [ROADMAP.md](../blob/main/ROADMAP.md#definition-of-done).
```
