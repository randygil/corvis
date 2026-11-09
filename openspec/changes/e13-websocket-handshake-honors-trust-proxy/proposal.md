# Proposal

Implements [randygil/corvis#158](https://github.com/randygil/corvis/issues/158) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Found while proposing #154
(`e13-throttler-groups-ipv6-clients-by-prefix`, which lists it as a possible new issue).

## Why

The WebSocket handshake request is built with `Request::new` (`crates/corvis/src/websockets.rs:725-729`), which uses
the default body-parser options and therefore `TrustProxy::None` (`crates/corvis/src/http/request.rs:49-56`, `:82`),
whatever the application set with `set_trust_proxy`. HTTP requests get the application's setting
(`Request::with_parser` with `shared.http_settings.body_parser`, `crates/corvis/src/server.rs:276`, `:306`).

Behind a reverse proxy, every WebSocket client then has the proxy's address in `client.handshake().ip()`, `ips()` and
`hostname()`, and in `ctx.request()` of gateway guards and interceptors. A global `ThrottlerGuard`, which counts
gateway messages by `req.ip()` (`crates/corvis/src/modules/throttler.rs:199`), puts all clients in one counter, so one
client can exhaust the limit for everyone. The docs promise the opposite: `set_trust_proxy` is the documented fix for
the throttler behind a proxy (`docs/security/rate-limiting.md:121-131`), and `client.handshake()` is documented as
giving the client's IP (`docs/websockets/gateways.md:193`).

The `upgrade` listeners of `corvis::ws` (`HttpServer::on_upgrade`) build their request the same way
(`crates/corvis/src/http_adapter.rs:272-276`) and have the same problem.

## What Changes

- The handshake `Request` of gateways and of `upgrade` listeners carries the application's `trust proxy` setting, so
  `ip()`, `ips()` and `hostname()` resolve `X-Forwarded-For` / `X-Forwarded-Host` exactly as for HTTP requests.
  With the default setting (`TrustProxy::None`) nothing changes.
- One crate-private constructor builds the upgrade request from the hyper parts, the socket address and the
  application's HTTP settings; `websockets::upgrade` and `http_adapter::dispatch_upgrade` both use it.
- **Divergence from the original, documented:** with Express `trust proxy`, a gateway behind a proxy still sees the
  proxy's address, because WebSocket upgrades never pass through Express (design.md, Context). Corvis applies the
  setting the application configured (security over parity).
- Tests with and without `trust proxy` for a gateway's handshake IP, for `ThrottlerGuard` on a gateway, and for an
  `upgrade` listener. Docs (EN + ES) on gateways, the `corvis::ws` adapter and rate limiting.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `http/trust-proxy`: adds that WebSocket handshakes and upgrade requests resolve the client address with the
  application's trust setting.

## Impact

- `crates/corvis/src/websockets.rs`: `upgrade` takes the application's trust setting (or `&AppShared`); the handshake
  request is built with the new constructor.
- `crates/corvis/src/http_adapter.rs`: `dispatch_upgrade` likewise.
- `crates/corvis/src/server.rs`: `handle` passes the setting to both (`:348-356`).
- `crates/corvis/src/http/request.rs`: the constructor (crate-private).
- Tests: `crates/corvis/tests/trust_proxy.rs` (gateway and throttler), `crates/corvis/tests/http_adapter.rs`
  (`upgrade` listener).
- Docs: `docs/websockets/gateways.md`, `docs/websockets/adapter.md`, `docs/security/rate-limiting.md` and their copies
  under `docs/es/`.
- **Behavior change** only for applications that set `trust proxy` and have gateways or upgrade listeners: the
  handshake IP becomes the forwarded client address, as the docs already say. The port apps do not use gateways, so
  apidiff should not move; the parity check confirms it.

## Out of scope

- The handshake's query string keeps the default query parser instead of the application's `set_query_parser`
  setting (same `Request::new` call): possible new issue.
- `#[ip]` as a gateway handler parameter: gateway handlers only accept `#[connected_socket]` and
  `#[message_body]`/`#[payload]` (`crates/corvis-macros/src/handlers.rs:165-177`); the IP is read with
  `client.handshake().ip()`. Possible new issue.
- Grouping IPv6 clients by prefix in the throttler (#154, its own change); it applies to gateway keys once both land.
- `trust proxy` changes after `init()`: HTTP requests do not see them either (the setting is copied into
  `AppShared` at init, `crates/corvis/src/application.rs:578`).
