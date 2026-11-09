# Proposal

Implements [randygil/corvis#159](https://github.com/randygil/corvis/issues/159) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Found while proposing #158
(`e13-websocket-handshake-honors-trust-proxy`, archived on `main`), whose proposal lists it as out of scope. The issue
body suggests the slug `e13-websocket-handshake-uses-the-query-parser`; this change uses
`e13-websocket-handshake-honors-query-parser`, the name it was assigned when drafted, to match #158's.

## Why

`Request::for_upgrade` (`crates/corvis/src/http/request.rs:62-73`, added by #158) builds the handshake request of
gateways and of `upgrade` listeners with `Request::new`, which uses `BodyParserOptions::default()`
(`request.rs:51-57`), so the query string is always parsed with `QueryParser::Simple` (`body_parser.rs:70`). The
application's `set_query_parser` setting (`crates/corvis/src/application.rs:401-404`) only reaches HTTP requests, which
are built with `Request::with_parser(.., &shared.http_settings.body_parser)` (`crates/corvis/src/server.rs:502`, `:392`).

So for the same URL a gateway and an HTTP route can see different query objects. With
`set_query_parser(QueryParser::Extended)`, `/chat?room[id]=7` gives `{"room": {"id": "7"}}` to a route and
`{"room[id]": "7"}` to `client.handshake().query()`; with `QueryParser::Disabled`, the route sees `{}` and the gateway
still sees the parsed query. Code shared between routes and gateways (a guard that reads `ctx.request().query()`, a
`query_as::<Dto>()` call) then behaves differently depending on the transport. The docs describe the handshake as "the
upgrade's HTTP `Request`" (`docs/websockets/gateways.md:193`), with nothing saying that one application setting is
skipped.

## What Changes

- `Request::for_upgrade` takes the application's `BodyParserOptions` instead of only its trust setting, and builds the
  request with `Request::with_parser` and an empty body. The handshake of gateways and of `upgrade` listeners then
  parses its query string with `set_query_parser` (and keeps resolving the client address with `set_trust_proxy`, as
  #158 made it).
- `handle` (`crates/corvis/src/server.rs:471-489`) passes `&shared.http_settings.body_parser` to
  `websockets::upgrade` and `http_adapter::dispatch_upgrade`.
- With `QueryParser::Disabled` the handshake's `query()` is empty and `query_param()` is `None`, as for HTTP requests;
  the raw query string stays available through `uri().query()` and `UpgradeRequest::query_string()`.
- **Divergence from the original, documented:** in Node, upgrades never pass through Express, so its `query parser`
  setting never applies to WebSocket handshakes (design.md, Context). Corvis applies the setting the application
  configured, so routes and gateways agree.
- The rustdoc of `UpgradeRequest::query_param` (`crates/corvis/src/http_adapter.rs:190-193`), which claims
  `url.searchParams.get(name)` semantics, is corrected to describe the parsed query.
- Tests for gateways and `upgrade` listeners with each parser; docs (EN + ES) on gateways, the `corvis::ws` adapter
  and the query parser setting.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `http/request-parsing`: adds that WebSocket handshakes and upgrade requests parse their query string with the
  application's query parser.

## Impact

- `crates/corvis/src/http/request.rs`: `for_upgrade` signature and body; its unit test (`upgrade_tests`).
- `crates/corvis/src/websockets.rs`: `upgrade` takes `&BodyParserOptions` (`:716-731`).
- `crates/corvis/src/http_adapter.rs`: `dispatch_upgrade` takes `&BodyParserOptions` (`:265-276`); rustdoc of
  `UpgradeRequest::query_param`.
- `crates/corvis/src/server.rs`: `handle` passes the options (`:477`, `:483`).
- Tests: `crates/corvis/tests/websockets.rs` (gateway), `crates/corvis/tests/http_adapter.rs` (`upgrade` listener).
- Docs: `docs/websockets/gateways.md`, `docs/websockets/adapter.md`, `docs/http/performance.md` (the `req.query`
  parser row) and their copies under `docs/es/`.
- **Behavior change** only for applications that call `set_query_parser` and have gateways or upgrade listeners. An
  application that disabled the query parser and reads `?token=` from `handshake().query_param()` now gets `None`
  and rejects the connection (fails closed); the docs show `uri().query()` for that case. No example and no port app
  calls `set_query_parser` (only `crates/corvis/tests/express_parity.rs:503` does), so apidiff should not move.

## Out of scope

- `#[ip]` in gateway handlers (#160, its own change).
- `UpgradeRequest::query_param` returning the first value of a repeated key, as `url.searchParams.get` does: with the
  default `simple` parser a repeated key is an array and `query_param` returns `None`, before and after this change.
  Possible new issue if a migrated `ws` app depends on it.
- A socket.io-style handshake query (`Object.fromEntries(searchParams)`, last value wins): Corvis has no socket.io
  adapter yet (E2).
- Changes to the query parser after `init()`: HTTP requests do not see them either (settings are copied into
  `AppShared` at init).
