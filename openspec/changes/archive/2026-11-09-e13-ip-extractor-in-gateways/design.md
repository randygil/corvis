# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state verified at `eb8bed1` (`main`, which
holds #158). Reference packages, `npm pack` into the scratchpad: `@nestjs/websockets` 11.2.9 and `@nestjs/common`
11.1.9.

**Corvis**

- Controllers: `PARAM_ATTRS` maps `ip` to `ParamKind::Ip` (`crates/corvis-macros/src/controller.rs:195-207`); the
  first string literal argument is `data` and the rest are pipes (`:353-364`). The router extracts it as
  `Value::String(req.ip())` (`crates/corvis/src/router/mod.rs:828`), runs the route's and the parameter's pipes on it
  (`:835-865`) and converts it with `extract_arg`, which uses `lenient::from_value` for every kind other than body
  and files and maps errors to `BadRequestException` (`crates/corvis/src/__private.rs:369-383`).
  `tests/trust_proxy.rs:171-176` checks that it keeps the full IPv6 address.
- Gateway handlers: `#[handlers]` (`crates/corvis-macros/src/handlers.rs`) collects typed parameters in one loop for
  every handler kind (`:155-183`). `connected_socket` sets role `socket`; `message_body` / `payload` set role `body`
  with an optional property name and pipes; any other attribute is kept on the parameter (`:178-180`), and the
  default role is `body`. Today `#[ip]` therefore stays on the parameter and rustc rejects it ("cannot find attribute
  `ip` in this scope"): `corvis` exports no `ip` attribute (`crates/corvis/src/lib.rs:258`).
- `#[subscribe_message]` expansion (`handlers.rs:260-338`): a `socket` parameter becomes
  `let __aN: T = ::corvis::websockets::client_from_any(&__client)?;`; a `body` parameter registers a `ParamDef` of
  `ParamKind::Body` with its pipes in `WsEnhancers` and becomes `extract_arg::<T>(&mut __args, body_index, ..)?`,
  incrementing `body_index` (`:268-306`). The call closure receives the client as `Arc<dyn Any + Send + Sync>` and the
  piped body values (`:319-336`).
- Runtime: `invoke_handler` (`crates/corvis/src/websockets.rs:822-837`) runs the global, gateway and handler pipes plus
  each body parameter's own pipes on the `ParamDef`s only; the socket parameter never goes through pipes.
  `client_from_any` (`websockets.rs:346-348`) downcasts the client; `WsClient::handshake()` (`:294-297`) returns the
  handshake `Request`, whose `ip()` follows `set_trust_proxy` since #158 (`Request::for_upgrade`,
  `crates/corvis/src/http/request.rs:62-73`).
- Other handler kinds in the same loop: `#[on_event]` binds the first parameter to the payload (`handlers.rs:189-219`);
  `#[cron]`, `#[interval]`, `#[timeout]` take no parameters (`:220-258`). Enhancers on a method without a handler
  attribute already produce named compile errors (`:131-150`, requirement "Enhancers on a method without a handler
  attribute in `#[handlers]` are a compile error" of `core/enhancers`), tested with `compile_fail` doctests
  (`crates/corvis/src/doctests.rs`).
- `OnGatewayConnection::handle_connection(&self, client)` and `handle_disconnect` are trait methods
  (`websockets.rs:76`, `:82`), not expanded by `#[handlers]`.
- Docs: `docs/websockets/gateways.md:108-118` (parameters table: `#[message_body]`, `#[payload]`,
  `#[connected_socket]`), `:193` (`client.handshake()` gives the IP); `docs/websockets/pipes.md:7-19` (pipes run on
  every `#[message_body]`); `docs/controllers.md:106` (`#[ip] ip: String`, honors `set_trust_proxy`).

**The original (Node)**

- `@Ip()` is `createRouteParamDecorator(RouteParamtypes.IP)` (`@nestjs/common decorators/http/route-params.decorator.js:84`),
  with `RouteParamtypes.IP = 11` (`enums/route-paramtypes.enum.js`). It writes into `ROUTE_ARGS_METADATA`, which is also
  the WebSocket `PARAM_ARGS_METADATA` (`@nestjs/websockets constants.js:12`), so it is accepted on a gateway handler
  without error.
- The WebSocket context reads every parameter through `WsParamsFactory.exchangeKeyForValue(type, data, args)`
  (`context/ws-context-creator.js:90-104`), which knows `SOCKET` (0), `PAYLOAD` (3) and `ACK` (13)
  (`enums/ws-paramtype.enum.js`) and returns `null` for any other type (`factories/ws-params-factory.js`). `@Ip()` in a
  gateway therefore receives `null` (after the pipes, which run on every WebSocket parameter:
  `ws-context-creator.js:106-116`).
- **Conclusion:** there is no original value to match; matching `null` would keep a silent trap for migrated code.
  Corvis gives the handshake address, consistent with its own controllers and with `client.handshake().ip()`.

## Goals / Non-Goals

**Goals:**
- `#[ip]` means "the client's address" in controllers and gateway message handlers alike, with the same trust rules.
- Misplaced `#[ip]` inside `#[handlers]` fails to compile with a message that says where it can go.

**Non-Goals:**
- `#[ip]` in lifecycle hooks, other controller parameter attributes on gateways, pipes on non-body gateway parameters
  (proposal, Out of scope).

## Decisions

1. **Semantics: the handshake address, per message.** `#[ip]` reads `client.handshake().ip()` when each message is
   handled. The handshake and the trust setting do not change during a connection, so the value is the same for every
   message; computing it per message keeps no extra state on the client and costs one `proxy_addr` walk over the
   handshake headers, the same as a controller's `#[ip]`. Not normalized by IPv6 prefix (that is the throttler's
   tracker, #154), like the controller parameter.
   *Alternatives:* the socket address (`remote_addr()`). Rejected: behind a proxy it is the proxy for every client,
   which is the problem #158 fixed for the handshake; `client.handshake().remote_addr()` stays available.
   Matching the original's `null`. Rejected (Context, Conclusion).
2. **Same conversion as controllers, through a private helper.** The macro emits
   `let __aN: T = ::corvis::__private::ws_ip_arg::<T>(&__client)?;`. The helper (`#[cfg(feature = "websockets")]`,
   `#[doc(hidden)]` like the rest of `__private`) downcasts with `client_from_any`, builds
   `Value::String(client.handshake().ip())` and converts with `lenient::from_value`, mapping an error to
   `BadRequestException::new("ip: <error>")` as `extract_arg` does for a named parameter. The handler call returns that
   error, so the client gets an `exception` frame and the handler does not run.
   *Alternative:* a `ParamDef` of `ParamKind::Ip` that `invoke_handler` resolves from the handshake. Rejected: it would
   change the runtime's argument vector (today body values only) and run pipes on a value the server computed (see
   Decision 3).
3. **No arguments, no pipes.** In a gateway `#[ip]` takes no arguments: `#[ip(..)]` is a compile error ("`<method>`:
   `#[ip]` takes no arguments in gateway handlers; gateway pipes only run on #[message_body]"). Global, gateway and
   handler pipes do not run on it, as they do not run on `#[connected_socket]` (`websockets.rs:822-837`). The value
   comes from the server, not from the frame, so a validation pipe has nothing to check; a pipe that wants it can read
   `ctx.request().ip()`. This differs from controllers, where `#[ip(Pipe)]` is accepted and route pipes run on it; the
   gateway docs say so.
   *Alternative:* accept and run pipes like controllers. Rejected: it needs Decision 2's rejected runtime change, and
   global validation pipes (`ZodValidationPipe::auto()`) would start running on a new kind of gateway argument.
4. **Compile errors in `#[handlers]`.** The parameter loop (`handlers.rs:155-183`) records `#[ip]` parameters with
   role `ip`. After the loop, for a method whose kind is not `subscribe_message`, an `#[ip]` parameter is a
   `syn::Error` spanned on the attribute: "`<method>` has an #[ip] parameter, but #[ip] is only available on
   #[subscribe_message] handlers (and on routes in #[routes]); <kind> handlers have no client". Errors are combined
   with the existing `discarded` errors so all of a block's errors are reported together (requirement "All
   discarded-attribute errors of a block are reported together" of `core/enhancers`). Each error gets a
   `compile_fail` doctest with a passing twin in `crates/corvis/src/doctests.rs`, as the enhancer errors have.
5. **Divergence documented, no differential test.** The gateways page states that `#[ip]` gives the handshake address
   where the original's `@Ip()` gives `null`. There is no original value to compare.
6. **Tests.** In `tests/trust_proxy.rs`, next to `WhoGateway` (`:186-198`): a handler with `#[ip] ip: String`, reached
   with the existing `ws_connect` helper, with `set_trust_proxy(1)`, without a setting, and with an IPv6
   `X-Forwarded-For`. In `tests/websockets.rs`: the mixed-parameter handler, a gateway with a counting pipe and an
   `#[ip]`-only handler, and `#[ip] ip: u32` (exception frame).

## Risks / Trade-offs

- [Apps that set `trust proxy` too broadly] → a client can choose its `#[ip]` value with `X-Forwarded-For`, exactly as
  for controllers and `client.handshake().ip()`; the gateways page links the trust warning of the rate-limiting page.
- [Code migrated from the original that relied on `@Ip()` being `null` in gateways] → unlikely to be intended; the
  divergence note covers it.
- [Different pipe rules for `#[ip]` in controllers and gateways] → documented in both pipe tables; a compile error
  catches `#[ip(Pipe)]` in gateways instead of silently ignoring the pipe.

## Open questions

None. The technical choices are recorded above and in the review notes.

## Review notes (2026-11-09)

- Drafted without a maintainer round. Decided here: the handshake address resolved with the trust setting (Decision
  1), the same conversion as controllers (Decision 2), no arguments and no pipes in gateways (Decision 3), compile
  errors only for `#[ip]` (other controller parameter attributes keep rustc's error, proposal Out of scope), and the
  divergence from the original's `null` documented without a differential test.

## Review notes, maintainer (2026-11-09)

- Approved as drafted: the trust-proxy-resolved handshake address, no arguments or pipes, and compile errors on
  methods that have no client.

## Implementation notes

- Task 2.1 asked for a unit test of `ws_ip_arg` (String and `u32` conversions). A `WsClient` cannot be built outside a
  live connection, so no unit test was written; the integration tests cover both conversions through real sockets
  (`ip_parameter_mixes_with_body_and_socket`, `ip_conversion_error_is_an_exception_frame`).
- Code and tests were written together; the "fails to compile" check of tasks 1.x was not run on its own.
- `#[ip(...)]` with arguments returns its error immediately (like other malformed attributes in the loop) instead of
  being combined with the block's discarded-attribute errors; `#[ip]` on non-message handlers is combined, as
  Decision 4 says.
- Both `compile_fail` doctests were also compiled as plain code to confirm they fail with the intended messages
  ("`who`: #[ip] takes no arguments in gateway handlers..." and "`on_order` has an #[ip] parameter, but ...").
- Task 5.5 (corvis-parity) is left for the maintainer's session.

## Review notes, security round (2026-11-09)

- Finding (MEDIUM): `ws_ip_arg` called `client.handshake().ip()` for every message, and `Request::ip()` built the whole
  `X-Forwarded-For` chain (latin1 conversion and one `String` per entry): 1.47 ms per message with `trust proxy` set
  and a 29 KB header (68 times the baseline).
- Fix (a): the client address is resolved once per connection, when the client is created (`ClientInner::ip`), and
  `ws_ip_arg` reads `WsClient::ip()`. `WsClient::ip()` is public.
- Fix (b): `Request::ip()` no longer builds the chain. `proxy_addr::client_addr` walks the entries from the right on
  raw bytes, converts only the entries it examines, and stops at the first untrusted address; with `All` it takes the
  left-most non-empty entry. `all_addrs` stays for `ips()`. This also makes HTTP `req.ip` cheaper.
- Tests: a differential test compares `client_addr` with the last element of `all_addrs` for every setting (None, All,
  Hops 0/1/2/50, subnets), six socket addresses and sixteen header shapes (empty and space-only entries, repeated
  headers, IPv6, bracketed with port, invalid entries, tabs); a test of `Hops(n)` beyond the header length; a test
  with 4000-entry headers; and `messages_share_the_handshake_and_its_resolved_address` checks that `client.ip()` is
  the same string (same pointer) for every message of a connection.
- Measured with the reviewer's PoC (release, `#[ip]`, 29 KB `X-Forwarded-For`, trust proxy `true`): 20.5 us/message
  after the fix, against about 1470 us before; the baseline is 20.3 us.

## Review notes, security round 2 (2026-11-09)

- Finding (LOW, pre-existing): `ThrottlerGuard` called `ctx.request().ip()` for every WebSocket message, which walks the
  handshake's `X-Forwarded-For` again when the trust setting trusts the client's chain (about 230 us per message).
- Fix: in a WebSocket context the guard's built-in tracker uses `WsClient::ip()` (resolved once per connection) from the
  context's client; HTTP requests and custom trackers are unchanged. The `websockets` feature gates the lookup.
- Test: `the_throttler_tracks_gateway_messages_with_the_connection_address` checks, through a key generator, that the
  tracker of every message equals `client.ip()`.
- The rustdoc of `WsClient::handshake()` and `WsClient::ip()` had been swapped; each has its own sentence again.


## Verification (2026-11-09)

Verified together in batch-q (#162, #159, #160, #117): `cargo fmt --check`, `cargo clippy --workspace --all-targets
-- -D warnings` and clippy of corvis without default features and with each feature alone (jwt, websockets,
health-tls, compression, tls) are clean; `cargo test --workspace --no-fail-fast` with `CORVIS_ORM_PG_REQUIRED=1` passed
1344 tests, `cargo test -p corvis --features compression` passed 1004 and `--features tls,websockets` passed 1010, 0
failures; naming, crate files, the examples (32/32 smoke checks) and `npm run docs:check` pass. CI run 42701896050 is
green on Linux and Windows. corvis-parity: consi 832/832, facten 727/727, houseplan 636/636 equal, databases
equivalent. Security review: two rounds, closed. Fixed: `#[ip]` resolved trust proxy over the whole X-Forwarded-For on
every message (1.47 ms per frame with a 29 KB header); the address is resolved once per connection, the throttler uses
it for WebSocket messages, and `Request::ip()` walks the header lazily (`proxy_addr::client_addr`, checked against
the previous implementation with a differential fuzzer: 21 million comparisons, no difference).
