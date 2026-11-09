# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state verified at `eb8bed1` (`main`, which
holds #158); paths under `crates/corvis/` unless stated. Reference packages, `npm pack` into the scratchpad:
`engine.io` 6.6.11, `socket.io` 4.8.4, `@nestjs/platform-ws` 11.2.9, `@nestjs/websockets` 11.2.9.

**Corvis**

- `set_query_parser` stores the setting in `config.http_settings.body_parser.query_parser`
  (`src/application.rs:401-404`); `init` copies `http_settings` into `AppShared` (`:588`, field
  `src/router/mod.rs:576`). `QueryParser` is `Simple` (default, `node:querystring`), `Extended` (`qs` with Express'
  options: `arrayLimit` 20, `depth` 5, `parameterLimit` 1000, `src/http/body_parser.rs:299-301`) or `Disabled`
  (`body_parser.rs:52-62`).
- `Request::with_parser` parses the query with `parse_query_with(uri.query(), parser.query_parser)`
  (`src/http/request.rs:76-104`, `:87`; `parse_query_with` `:310-350`) and copies `parser.trust_proxy` (`:100`). It
  keeps a body-parser error in `body_error` instead of failing.
- `Request::new` (`request.rs:51-57`) calls `with_parser` with `BodyParserOptions::default()` and turns `body_error`
  into `Err`.
- `Request::for_upgrade(parts, remote, trust)` (`request.rs:62-73`, from #158) calls `Request::new` with an empty
  body, then sets `version`, `remote_addr` and `trust_proxy = trust.clone()`. Its query is therefore always parsed
  with `QueryParser::Simple`. Unit test: `upgrade_tests::for_upgrade_resolves_the_address_with_the_given_setting`
  (`request.rs:352-375`), which also checks `query_param("a")`.
- Callers: `websockets::upgrade(gateway, req, remote, trust)` (`src/websockets.rs:716-731`; a failed `for_upgrade`
  answers `400 Bad Request`) and `http_adapter::dispatch_upgrade(server, req, remote, trust)`
  (`src/http_adapter.rs:265-276`; a failure destroys the socket). Both are called from `handle` with
  `&shared.http_settings.body_parser.trust_proxy` (`src/server.rs:477`, `:483`).
- HTTP requests: `from_hyper(.., &shared.http_settings.body_parser, ..)` (`src/server.rs:502`, `:388-397`) builds them
  with `Request::with_parser`, so they follow `set_query_parser`. The test client does the same
  (`src/testing.rs:385`).
- The handshake request reaches `verify_client` (`websockets.rs:732-737`), `client.handshake()` (`:294-297`) and
  `ctx.request()` of every message's `ExecutionContext`.
- `UpgradeRequest::query_param` (`src/http_adapter.rs:190-193`) is documented as "`url.searchParams.get(name)`" but
  returns `self.inner.request.query_param(name)`, a string value of the parsed query (`request.rs:159-161`), so a
  repeated key (an array with the `simple` parser) gives `None`. `query_string()` (`:186-189`) returns the raw string.
- Tests: `tests/express_parity.rs:495-512` checks the `simple` and `extended` parsers on HTTP routes;
  `tests/websockets.rs` reads `?user=` in a gateway guard (`:153-170`) and connects with a query
  (`secure_client`, `:288-295`); `tests/http_adapter.rs:182` reads `query_param("token")` from a `corvis::ws`
  connection. No test sets a query parser for a gateway or an upgrade listener.
- Docs: `docs/websockets/gateways.md:193` ("`client.handshake()` | The upgrade's HTTP `Request`: headers, query string,
  IP ..."), `:213-224` (Tertulia reads `?token=` with `query_param`); `docs/websockets/adapter.md:60`, `:93`;
  `docs/http/performance.md:141` (the `req.query` parser row).

**The original (Node)**

- Express' `query parser` setting is applied by Express' `query` middleware to the requests Express handles as the
  HTTP server's `request` listener. An upgrade is emitted as the server's `upgrade` event and never reaches Express
  (see #158's design, Context).
- `@nestjs/platform-ws`: `httpServer.on('upgrade', (request, socket, head) => ... wsServer.handleUpgrade(request,
  socket, head, (ws) => wsServer.emit('connection', ws, request)))` (`adapters/ws-adapter.js:237-246`). The gateway
  gets Node's raw `IncomingMessage`, which has `url` and no parsed query; applications parse it themselves (usually
  `new URL(request.url, base).searchParams`).
- `socket.io` / `engine.io`: `prepare(req)` sets `req._query = Object.fromEntries(new URL(req.url,
  "https://socket.io").searchParams.entries())` unless something already set `req._query` (`engine.io
  build/server.js:520-526`, `build/utils/objectFromEntries.js`), and the handshake exposes it as
  `socket.handshake.query` (`socket.io dist/socket.js:142`). That object is flat and keeps the last value of a
  repeated key; it is never the Express query parser's output.
- **Conclusion:** no Node setup applies `query parser` to a WebSocket handshake. Corvis gateways already expose the
  handshake as a full `Request` with a parsed query (the `ws` adapter's shape plus a convenience), so the question is
  only which parser fills it. Using the one the application configured is the consistent choice.

## Goals / Non-Goals

**Goals:**
- Every `Request` Corvis builds for an incoming connection (HTTP request, gateway handshake, upgrade listener) parses
  its query string with the same application setting, as it already resolves the client address with the same trust
  setting (#158).

**Non-Goals:**
- `#[ip]` in gateway handlers (#160), `searchParams.get` semantics for repeated keys, a socket.io handshake query,
  runtime changes of the setting (proposal, Out of scope).

## Decisions

1. **`for_upgrade` takes the whole `BodyParserOptions`.** New signature: `Request::for_upgrade(parts:
   http::request::Parts, remote: SocketAddr, parser: &BodyParserOptions) -> Result<Request, HttpException>`
   (crate-private). It calls `Request::with_parser(parts.method, parts.uri, parts.headers, Bytes::new(), parser)`,
   returns `Err` when `body_error` is set (as `Request::new` does), and sets `version` and `remote_addr`. The trust
   setting now comes from `parser.trust_proxy` through `with_parser`, so the separate `trust` argument goes away.
   *Alternatives:* add a `query_parser: QueryParser` argument next to `trust`. Rejected: two arguments taken from one
   struct, and each future request setting would add another; #158's Decision 1 already anticipated passing the
   whole settings. Pass `&HttpSettings`. Rejected: the request only reads the body-parser part, and HTTP requests are
   built from that same part (`from_hyper`).
   *Side effect, accepted:* the body checks that can reject an empty body (a declared JSON or urlencoded
   `Content-Type` with an unsupported charset or a `Content-Encoding`, `src/http/body_parser.rs:84-107`) now follow
   the application's options too: with `disable_body_parser()` such a handshake is accepted instead of answered with
   `400`, as an HTTP request with those headers is. The upgrade's body is never read, so nothing is parsed either
   way; the check is not a security control for upgrades.
2. **Pass the options from `handle`.** `websockets::upgrade(gw, req, remote, &shared.http_settings.body_parser)` and
   `dispatch_upgrade(&shared.http_server, req, remote, &shared.http_settings.body_parser)`. One copy of the setting,
   in `AppShared`, as #158's Decision 2.
3. **Gateways and upgrade listeners both follow the setting.** They share the constructor, and an `upgrade` listener's
   request is documented as "the handshake request (method, URL, headers, query, remote address...)"
   (`http_adapter.rs:169-173`). Splitting them would bring back the two-call-site drift #158 removed.
4. **Diverge from the original, and say so.** The spec states the Corvis behavior. `docs/websockets/gateways.md` gets
   a note: unlike a Node gateway, where the upgrade bypasses Express and its `query parser`, Corvis parses the
   handshake query with `set_query_parser`; `uri().query()` (and `UpgradeRequest::query_string()`) give the raw string
   for code that parsed `request.url` itself. No differential test: no original exposes a value to compare (platform-ws
   has no parsed query; socket.io's is a different, fixed parser).
   Security note: one parser per application means a guard shared by HTTP routes and gateways reads the same
   structure on both. With `Extended`, `?token[]=a` is an array and `query_param("token")` is `None` on both
   transports, so the guard fails closed in both instead of only on one. The `qs` limits (depth 5, 20 array items,
   1000 parameters, total and proportional per the existing "Query string parsing is total and proportional"
   requirement) already bound handshake parsing, since handshakes are bounded by the same header limits as HTTP heads.
5. **Correct `UpgradeRequest::query_param`'s rustdoc.** It becomes "A string parameter of the parsed query
   ([`Request::query`], with the application's query parser); `None` when absent, repeated (with the `simple` parser)
   or nested. [`query_string`](Self::query_string) has the raw string." Its behavior for repeated keys is unchanged
   (proposal, Out of scope).
6. **Tests through real sockets.** In `tests/websockets.rs`: a gateway with a `query` message that replies
   `client.handshake().query()` and `uri().query()`, connected with `tokio_tungstenite::connect_async` to apps with
   each parser (default, `Extended`, `Disabled`), plus `verify_client` reading `query().get("auth")` and the
   query-plus-trust scenario. In `tests/http_adapter.rs`: an `upgrade` listener on its own small module (as #158's
   tests, since the bridge module destroys unknown paths) that answers with the parsed query and `query_string()` in
   headers of its `Respond` decision. The unit test in `request.rs` passes `BodyParserOptions` (default, `Extended`,
   with a trust setting) and checks both the query and `ip()`.

## Risks / Trade-offs

- [Apps that disabled the query parser and read `?token=` from a gateway handshake] → `query_param` now returns
  `None`, so their connection check rejects every client (fails closed, visible on the first connection). The docs
  and the change notes show `client.handshake().uri().query()` with `form_urlencoded`, or re-enabling the parser.
- [Apps with `Extended` that read `handshake().query_param("x")` for a key sent as `x[]=..`] → `None`, as their HTTP
  routes already get. Same note in the docs.
- [Handshakes accepted with `disable_body_parser()` and a bogus `Content-Type` charset] → see Decision 1; harmless
  because the body is never read.

## Open questions

None. The technical choices are recorded above and in the review notes.

## Review notes (2026-11-09)

- Drafted without a maintainer round. Decided here: the whole `BodyParserOptions` is passed to `for_upgrade`
  (Decision 1, including the accepted side effect on body checks); both gateways and upgrade listeners follow the
  setting (Decision 3); the divergence from Node is documented without a differential test (Decision 4); the
  `query_param` rustdoc is corrected but its repeated-key behavior is left for a possible new issue.

## Review notes, maintainer (2026-11-09)

- Approved. The handshake follows the application's query parser like HTTP routes; with `Disabled`,
  `query_param` returns `None` and a token check fails closed. Corvis is not published yet, so the behavior change
  needs only the docs note (read `uri().query()` for the raw string).

## Implementation notes

- Code and tests were written together and then run; the tests-first "verify they fail" step of tasks 1.x was not run
  separately.
- The unit test of `for_upgrade` gained a second test (`for_upgrade_parses_the_query_with_the_given_parser`) covering
  the three parsers.
- The docs note on the difference with a Node gateway is a second `warning` container after the trust-proxy one in
  `docs/websockets/gateways.md`, and also covers `Extended` with `token[]=a`.
- Task 4.5 (corvis-parity) is left for the maintainer's session, as the implementation brief states.

## Review notes, security round (2026-11-09)

- Finding (MEDIUM): with `QueryParser::Extended`, `dispatch` cloned the handshake `Request` for every message, so a
  client that sent a large nested query once paid a deep copy of the parsed query on each tiny frame (2.36 ms per
  30-byte frame with a ~29 KB query, against 21.7 us without it).
- Fix: the handshake is held as `Arc<Request>` in the client and `ExecutionContext::for_ws` takes
  `impl Into<Arc<Request>>` (the HTTP constructor wraps its request once). A message costs a reference count whatever
  the size of the handshake. `WsClient::handshake()` still returns `&Request`. The "Cheap to clone" comment on
  `Request` was false and is corrected.
- Test: `messages_share_the_handshake_and_its_resolved_address` checks that the `Request` seen by guards in every
  message is the one `client.handshake()` returns (same address).
- Measured with the reviewer's PoC (release, 29 KB query, 1434 params, Extended): 21.2 us/message after the fix,
  against 2360 us before; the baseline is 20.3 us.

