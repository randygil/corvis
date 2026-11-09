# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state verified at `510576f` (paths under
`crates/corvis/` unless stated) and in the reference packages: `@nestjs/platform-ws` 11.2.9,
`@nestjs/platform-socket.io` 11.2.9, `@nestjs/websockets` 11.2.9, `@nestjs/throttler` 6.7.1, `socket.io` 4.8.4 and
`engine.io` 6.6.11 (`npm pack` into the scratchpad; the port apps under `corvis-tests/nest` have no WebSocket or
throttler packages), and `@nestjs/core` 11.2.1 from `corvis-tests/nest/facten-api/node_modules`.

**Corvis**

- `Request` keeps the trust setting in `trust_proxy` (`src/http/request.rs:29-30`) and uses it in `ip()`, `ips()`
  (`:148-168`, through `proxy_addr::all_addrs`) and `hostname()` (`:172-176`). `Request::with_parser` copies it from
  the `BodyParserOptions` it receives (`:59-86`, `:82`); `Request::new` calls it with
  `BodyParserOptions::default()` (`:49-56`), whose `trust_proxy` is `TrustProxy::default()` (`src/http/body_parser.rs:72`),
  that is `None`.
- `set_trust_proxy` stores the setting in `config.http_settings.body_parser.trust_proxy`
  (`src/application.rs:407-408`); `init` copies `http_settings` into `AppShared` (`:578`, field
  `src/router/mod.rs:574`).
- HTTP requests: `from_hyper` builds them with `Request::with_parser(.., parser)` (`src/server.rs:276`, `:306`), with
  `parser = &shared.http_settings.body_parser` (`:367`), so they honor the setting. The test client does the same
  (`src/testing.rs:383`).
- Gateways: `handle` routes upgrade requests to `websockets::upgrade(gw, req, remote)` (`src/server.rs:348-350`),
  which builds the handshake with `Request::new` and sets `version` and `remote_addr` (`src/websockets.rs:725-729`).
  That request goes to `verify_client` (`:730-735`), into the connection (`run_connection`, `:744`, `:963-976`), to
  `client.handshake()` (`:295-297`) and to every message's `ExecutionContext::for_ws` (`:891-898`;
  `src/common/context.rs:84-102`), where `ctx.request()` returns it.
- `ThrottlerGuard::can_activate` keys by `req.ip()` of `ctx.request()` (`src/modules/throttler.rs:190-199`), so on a
  gateway it keys by the handshake IP.
- Upgrade listeners: `handle` calls `http_adapter::dispatch_upgrade(&shared.http_server, req, remote)`
  (`src/server.rs:353-356`), which also uses `Request::new` (`src/http_adapter.rs:272-276`); listeners read it with
  `UpgradeRequest::request()` (`:171`), and `corvis::ws` keeps it as `upgrade_request()`.
- Gateway handler parameters: only `#[connected_socket]` and `#[message_body]`/`#[payload]` are recognized
  (`crates/corvis-macros/src/handlers.rs:165-177`); `#[ip]` exists for controllers only
  (`crates/corvis-macros/src/controller.rs:195-207`). The issue's "`#[ip]`" therefore means the handshake IP read
  through `client.handshake().ip()` or `ctx.request().ip()`.
- Tests: `tests/trust_proxy.rs` covers HTTP (`ip`, `ips`, `hostname`, throttler) with `set_trust_proxy`;
  `tests/websockets.rs:275-278` only checks that the handshake's `remote_addr` is loopback; no test combines gateways
  and `trust proxy`.
- Docs: `docs/websockets/gateways.md:193` ("`client.handshake()` | The upgrade's HTTP `Request`: headers, query string,
  IP"), `:300` (throttler counts per handshake IP); `docs/security/rate-limiting.md:121-131` (`set_trust_proxy` fixes
  the shared counter behind a proxy), `:135-136`; `docs/websockets/adapter.md:94-95`, `:124`.

**The original (Node)**

- Express's `trust proxy` lives in `req.ip` / `req.ips` / `req.hostname`, getters that Express adds to the requests
  it handles as a `request` listener. A WebSocket upgrade is emitted as the HTTP server's `upgrade` event and never
  reaches Express:
  - `@nestjs/platform-ws`: `httpServer.on('upgrade', (request, socket, head) => wsServer.handleUpgrade(request, ...,
    (ws) => wsServer.emit('connection', ws, request)))` (`adapters/ws-adapter.js:237-246`). `request` is Node's raw
    `IncomingMessage`; it has no `ip`.
  - `socket.io` / `engine.io`: the client address is `req.connection.remoteAddress` (or the WebSocket's
    `_socket.remoteAddress`) cached when the engine.io socket is created (`engine.io/build/socket.js:52-59`) and
    exposed as `client.handshake.address` (`socket.io/dist/socket.js:135`).
- In a WebSocket `ExecutionContext` the arguments are `[client, data]`; `switchToHttp().getRequest()` returns the
  first one, the client socket (`@nestjs/core/helpers/execution-context-host.js:37`). So a gateway guard has no Express
  request at all. The unmodified `ThrottlerGuard` keys by `req.ip` of that object (`throttler.guard.js:114`, `:123`,
  `:150-151`), which is `undefined`, and `normalizeIp` returns it unchanged (`ip.js:76-79`): every WebSocket client
  shares one counter even without a proxy.
- `@nestjs/throttler`'s README ("Websockets") tells users to subclass the guard and key by
  `client._socket.remoteAddress` (platform-ws) or `client.conn.remoteAddress` (socket.io): the socket address, that
  is the proxy's address behind a proxy. `trust proxy` is never applied.
- **Conclusion:** with Express `trust proxy`, a gateway guard behind a proxy sees the proxy's address (or nothing, with
  the stock throttler). Matching that would keep the vulnerability the issue describes, and contradict Corvis's own
  docs; the project rule "security over parity" applies. Corvis already gives gateways a full `Request` for the
  handshake, so honoring the setting the application configured is the consistent choice.

## Goals / Non-Goals

**Goals:**
- Every `Request` Corvis builds for an incoming connection (HTTP request, gateway handshake, upgrade listener) resolves
  the client address with the same `trust proxy` setting.
- Behind a trusted proxy, the throttler on gateways counts per client.

**Non-Goals:**
- The handshake's query parser, `#[ip]` for gateway handlers, IPv6 prefix grouping (#154), runtime changes of the
  setting (proposal, Out of scope).

## Decisions

1. **One constructor for upgrade requests.** `Request::for_upgrade(parts: http::request::Parts, remote: SocketAddr,
   trust: &TrustProxy) -> Result<Request, HttpException>` (crate-private, in `src/http/request.rs`): `Request::new`
   with an empty body, as today, then `version`, `remote_addr` and `trust_proxy = trust.clone()`. Both
   `websockets::upgrade` and `http_adapter::dispatch_upgrade` use it and keep their error branches unchanged (`400 Bad
   Request` and a destroyed socket): `Request::new` can still fail on an empty body, because the body parser rejects a
   declared JSON or urlencoded body with an unsupported charset or `Content-Encoding` before looking at its bytes
   (`src/http/body_parser.rs:84-107`, `has_body` `:143-153` is true for `Content-Length: 0`). That rejection is not
   this change's concern.
   Alternatives considered: build the handshake with `Request::with_parser(.., &shared.http_settings.body_parser)`.
   Rejected for now: it would also switch the handshake's query string to the application's query parser, a separate
   behavior change (Out of scope); the constructor can take the whole `HttpSettings` later if that is wanted. Setting
   `handshake.trust_proxy` after `Request::new` at each call site. Rejected: two call sites to keep in sync, which is
   how the second one was missed.
2. **Pass the setting from `handle`.** `websockets::upgrade(gw, req, remote, &shared.http_settings.body_parser.
   trust_proxy)` and `dispatch_upgrade(&shared.http_server, req, remote, trust)`; `handle` already has `shared`
   (`src/server.rs:340-356`). Alternative: store the setting in each `GatewayRuntime`. Rejected: a second copy of an
   application setting that `AppShared` already holds.
3. **Diverge from the original, and say so.** The spec states the Corvis behavior; the docs page on gateways gets a
   short note that, unlike a Node gateway behind Express (where the upgrade bypasses `trust proxy` and guards see the
   socket address), Corvis applies `set_trust_proxy` to handshakes. No differential test: the original has no
   equivalent value to compare (its stock throttler key is `undefined`), which the note also states.
4. **Tests through real sockets.** In `tests/trust_proxy.rs`, next to the HTTP cases: a gateway whose `ip` message
   returns `client.handshake().ip()`, `ips()` and `hostname()`, reached with `tokio_tungstenite::connect_async` and a
   request built with `IntoClientRequest` plus `X-Forwarded-For` / `X-Forwarded-Host`; a gateway with a global
   `ThrottlerGuard` (limit 2) and two clients with different forwarded addresses, with and without
   `set_trust_proxy(1)`; a gateway with `verify_client` checking the IP. In `tests/http_adapter.rs`: an `upgrade`
   listener that answers with `req.request().ip()` in a header of its `Respond` decision.

## Risks / Trade-offs

- [Applications that set `trust proxy` too broadly] → a client could choose its gateway IP with `X-Forwarded-For`, as
  it already can for HTTP. The rate-limiting page's warning about `true` now covers gateways too; the docs say so.
- [Behavior change for gateways behind a proxy] → only with a trust setting, where the old value (the proxy's address)
  was wrong by the docs' own description. Called out in the change notes.
- [Applications that relied on the proxy address in `handshake().ip()`] → they can read
  `client.handshake().remote_addr()`, which keeps the socket address; the docs mention it.

## Open questions

1. **Query parser.** Should the handshake also follow `set_query_parser` (Decision 1, alternatives), in this change or
   in a follow-up issue? Proposed: follow-up issue.
2. **`#[ip]` in gateway handlers.** Worth a follow-up issue so gateway handlers can take `#[ip] ip: String` like
   controllers? Proposed: yes, low priority.

## Review notes (2026-11-09)

- Q1: the handshake following `set_query_parser` is filed as a separate issue (low priority), out of scope here.
- Q2: `#[ip]` in gateway handlers is filed as a separate issue (low priority), out of scope here.

## Implementation notes

- `Request::for_upgrade` is as designed (`src/http/request.rs`), with a unit test (`upgrade_tests`). It is not feature-gated: `http_adapter` uses it with and without `websockets`.
- Tests: the gateway, throttler and `verify_client` scenarios are in `tests/trust_proxy.rs`, plus an untrusted peer forging `X-Forwarded-For` / `X-Forwarded-Host` (ignored) and a one-hop chain where the client-supplied left-most entry is ignored. The upgrade listener tests are in `tests/http_adapter.rs`, on their own small module: the bridge module's listener destroys unknown paths before a second listener could answer.
- The throttler test checks the `exception` event of the third message; without trust both clients share the `127.0.0.1` counter.
- Docs: the line numbers of the rate-limiting page had moved since the design; edited its "Behind a proxy" section.
- Task 4.5 (`corvis-parity`) is left to the maintainer's session, as instructed.
