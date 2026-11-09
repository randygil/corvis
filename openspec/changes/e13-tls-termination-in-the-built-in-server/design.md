# Design

## Context

Issue #117. The #23 spike chose `listen_tls` / `listen_tls_in_background` with rustls and the `ring` provider behind a
non-default `tls` feature; its review kept `tls` out of the default features and rejected an `https_options`
application option.

### The original

The original framework takes `httpsOptions` in the application options and passes them to Node's
`https.createServer`: `key`, `cert` (PEM strings or buffers), optional `ca`, `passphrase`, `requestCert`, etc. With the
Express adapter the HTTPS server speaks HTTP/1.1 only (no ALPN `h2`). Node's TLS defaults: minimum TLS 1.2,
`handshakeTimeout` 120 s, no client certificates. `app.getUrl()` returns `https://...` for an HTTPS server. WebSocket
gateways ride on the same server, so they are served as `wss://`.

### Corvis today (verified in the code)

- `ServerOptions` has `headers_timeout` (60 s), `request_timeout` (300 s), `keep_alive_timeout` (5 s),
  `max_connections` (unlimited), `max_header_size` (32 KiB), `h2c` (off) and `insecure_http_parser` (off)
  (`crates/corvis/src/server.rs:44-67`). No TLS field.
- `serve(listener: TcpListener, shared, options, shutdown)` (`server.rs:811-885`) builds one hyper-util `auto::Builder`
  (`server.rs:819-829`), acquires a `max_connections` permit before `accept` (`server.rs:833-839`), and for each
  accepted socket sets `TCP_NODELAY`, wraps it in `TimedIo`, registers the connection with the graceful-shutdown
  tracker and spawns `finish_connection` (`server.rs:840-875`). Shutdown drains for at most 10 s (`server.rs:879-883`).
- `TimedIo` holds the `TcpStream` directly (`io: Option<TcpStream>`, `server.rs:478-498`); the spike's note that it was
  generic is out of date. It implements the headers and keep-alive timeouts (`poll_idle`, `server.rs:617-647`), the h2c
  preface refusal when `h2c` is off (`preface`, `server.rs:575`, `603-613`) and the HTTP/1 framing check from #146
  (`framing`, `server.rs:576`, `661-749`). On drop it hands the socket and the permit to `ConnState::closing`
  (`server.rs:410`, `525-532`), and `finish_connection` closes it with `linger_close(TcpStream)` (FIN, then read and
  discard up to 1 MiB / 2 s) unless the server is punishing the peer (`server.rs:500-562`).
- `handle` answers WebSocket upgrades through `websockets::upgrade` before routing (`server.rs:346-351`); the gateway
  then runs over `hyper::upgrade::Upgraded`, which is type-erased (`crates/corvis/src/websockets.rs:716-743`), and raw
  upgrade listeners get a `TokioIo<Upgraded>` (`crates/corvis/src/http_adapter.rs:51`, `235`). Neither depends on the
  socket type.
- `listen` / `listen_in_background` call `bind` (`crates/corvis/src/application.rs:597-656`), which runs `init()`
  (including the route posture check, `application.rs:491-494`, `560-561`), warns about `insecure_http_parser`, binds
  TCP, records `local_addr` and logs "Corvis application successfully started". `get_url` always formats `http://`
  (`application.rs:659-664`). Several listeners are allowed; `get_url` reports the last one
  (`docs/faq/multiple-servers.md`).
- `crates/corvis/Cargo.toml:47-54`: `tokio-rustls` 0.26 (`default-features = false`, features `ring`, `tls12`) and
  `webpki-roots` are optional, enabled by `health-tls` for the HTTPS health indicator. Its client code picks the
  process-wide rustls `CryptoProvider` when one is installed and `ring` otherwise, to avoid rustls' panic when both
  `ring` and `aws-lc-rs` are compiled in (`crates/corvis/src/modules/health.rs:359-385`, provider choice at line 376).
- `deny.toml` evaluates the graph with all features (`[graph] all-features = true`) and denies unmaintained crates, so
  `tokio-rustls` already passes. `rustls-pemfile` is archived upstream and must not be added; `rustls-pki-types`
  (already a dependency of `rustls`) has its own PEM parser.
- CI checks each `corvis` feature alone with clippy (`.github/workflows/ci.yml:63-69`), but the test jobs run
  `cargo test --workspace` with default features, so feature-gated tests (today the `health-tls` ones,
  `crates/corvis/tests/health.rs:216`, `330`) do not run in CI.
- Route posture (`crates/corvis/src/posture.rs`) is about guards on routes and gateway handlers; it lists unchecked
  surfaces (static files, WebSocket handshakes, upgrade listeners, `posture.rs:425-438`) and knows nothing about the
  transport.

## Goals / Non-Goals

**Goals:**
- HTTPS (HTTP/1.1 and HTTP/2) and `wss://` from the built-in server with rustls only.
- Defaults that are safe without tuning, and a handshake that cannot be used to exhaust the server.
- Plain listeners unchanged, default builds unchanged.

**Non-Goals:**
- Hot reload, mTLS, ACME, SNI multi-certificate API, `req.secure()` (proposal.md, Out of scope).

## Decisions

1. **Feature `tls = ["dep:tokio-rustls"]`, not default.** `webpki-roots` is a client concern and stays with
   `health-tls`. With `tls` off, nothing below compiles, and the `ServerIo` enum (decision 4) has one variant, so the
   plain path compiles to what it is today. Alternative: on by default. Rejected by the #23 review and #106 (dev build
   size).
2. **rustls with the process default provider, else `ring`.** The `TlsOptions` constructors build the `ServerConfig`
   with `ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions()`, where `provider` is
   `CryptoProvider::get_default()` or `ring::default_provider()`, as `health.rs` does. An application that installs
   `aws-lc-rs` (for example for FIPS) gets it; nobody gets a panic. Alternatives: OpenSSL / native-tls (needs OpenSSL in
   images, per-platform behavior; rejected by the spike) and `aws-lc-rs` as Corvis' own choice (CMake and NASM on
   Windows; rejected by the spike).
3. **`TlsOptions` API (in `corvis::tls`).**
   - `from_pem_files(cert_chain: impl AsRef<Path>, key: impl AsRef<Path>) -> Result<TlsOptions, TlsError>` and
     `from_pem(cert_chain: &[u8], key: &[u8]) -> Result<TlsOptions, TlsError>`: all `CERTIFICATE` blocks in file
     order (leaf first) and exactly one private key (`PRIVATE KEY`, `RSA PRIVATE KEY` or `EC PRIVATE KEY`), parsed with
     `rustls::pki_types::pem::PemObject`. Files are read once, at construction.
   - `from_server_config(config: Arc<rustls::ServerConfig>) -> TlsOptions`: for SNI-based certificates, client
     verifiers, custom resolvers (including a reloading one) or TLS 1.3 only. Corvis clones the config and applies
     decision 5 to the clone.
   - `.handshake_timeout(Duration)` (default 10 s, decision 7) and `.http2(bool)` (default `true`; `false` advertises
     only `http/1.1`, the behavior of an Express HTTPS server).
   - `TlsError` (`thiserror`): `Io { path, source }`, `NoCertificate`, `NoPrivateKey`, `MultiplePrivateKeys { count }`,
     `EncryptedPrivateKey` ("decrypt the key first, for example with `openssl pkcs8`"), `InvalidPem`, and
     `Rustls(rustls::Error)` (unsupported key type, key that does not match the certificate). Messages name the file
     path but never echo file contents.
   - `corvis::tls::rustls` re-exports the rustls crate so users build `ServerConfig`s with the same version.
   Alternatives considered: TLS fields on `ServerOptions` (one TLS setting for every listener, the same problem as
   `https_options`); a `TlsOptions` builder that takes paths and loads them at `listen_tls` time (errors would appear
   after `init()` and its hooks have run).
4. **Layering: TCP, then TLS, then `TimedIo`, then hyper.** The framing check and the preface check must see
   plaintext, and the timeouts must cover the decrypted stream. `TimedIo.io` becomes `Option<ServerIo>` with
   `enum ServerIo { Plain(TcpStream), #[cfg(feature = "tls")] Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>) }`
   implementing `AsyncRead`/`AsyncWrite` by delegation (including `is_write_vectored`). `ConnState::closing` stores a
   `ServerIo`, and `linger_close` takes a `ServerIo`: `shutdown()` on a TLS stream sends `close_notify` and then the
   TCP FIN; the bounded drain reads through TLS. Alternative: make `TimedIo` generic over the stream. Rejected: it was
   made non-generic on purpose by `e0-transport-security-tests-are-flaky-on-windows` (decision 1) so that
   `ConnState` can hold the socket for the closing drain; a generic version would make `ConnState` and
   `finish_connection` generic too, for no gain over one branch per I/O call.
5. **Configuration hardening applied to every `TlsOptions`, including `from_server_config`.**
   - `max_early_data_size = 0`: 0-RTT data can be replayed by an attacker, and handlers are not written to be
     idempotent; Corvis never accepts it.
   - ALPN: if the config's `alpn_protocols` is empty, set `["h2", "http/1.1"]` (or `["http/1.1"]` with
     `http2(false)`); if it is not empty, keep it and remove `h2` when `http2(false)`. A connection that negotiates any
     other protocol is closed.
   - Everything else keeps rustls' defaults: TLS 1.2 and 1.3 (rustls implements nothing older), its default cipher
     suites (ECDHE with AEAD only), session resumption with rustls' in-memory session cache, no client
     authentication in the PEM constructors.
6. **The handshake runs in the connection task.** The accept loop keeps acquiring the permit, accepting and setting
   `TCP_NODELAY`; for a TLS listener it spawns a task that runs `TlsAcceptor::accept` under
   `tokio::time::timeout(handshake_timeout, ..)` and only then builds `TimedIo` and serves the connection. The task owns
   the permit from the start, so connections in the handshake count against `max_connections` and a flood of slow
   handshakes is bounded by that cap and by the timeout. A failed or timed-out handshake drops the socket without the
   linger drain (the peer gets nothing to read) and releases the permit; it is not logged (scanners would flood the
   log). The headers timeout starts when `TimedIo` is created, that is after the handshake. To keep graceful shutdown
   working when the connection future only exists after the handshake, the loop takes a
   `GracefulShutdown::watcher()` per connection (hyper-util 0.1.21, the locked version) and the task calls
   `watcher.watch(conn)`; a handshake that ends after shutdown started is served with an immediate graceful close.
   Alternative: handshake in the accept loop. Rejected: one client that never finishes its `ClientHello` would stop
   every new connection (the risk the issue names).
7. **Handshake timeout 10 s by default.** Node uses 120 s. A legitimate client finishes a TLS handshake in a few round
   trips; 120 s lets a slow-loris client hold a socket, a task and a `max_connections` permit for two minutes before it
   sends a single HTTP byte. Security over parity: 10 s, configurable, documented as a divergence. It cannot be
   disabled.
8. **Protocol per connection from ALPN.**
   - `h2`: an HTTP/2-only connection (hyper-util `auto::Builder::http2_only()`, or the http2 builder), with `TimedIo`'s
     preface check and HTTP/1 framing check off, since both parse HTTP/1. HTTP/2 limits are those of h2c today (200
     concurrent streams, 16 KiB header list, hyper's reset-flood protections).
   - `http/1.1` or no ALPN: an HTTP/1-only connection (`http1_only()`), with the framing check (unless
     `insecure_http_parser`) and the preface check on: HTTP/2 over TLS without ALPN negotiation is not allowed by
     RFC 9113 §3.2, and `h2c` is about cleartext, so it does not change this.
   - Plain listeners keep today's `auto::Builder` behavior exactly.
9. **`listen_tls(self, addr, tls)` and `listen_tls_in_background(&mut self, addr, tls)`.** They share `bind` (so
   `init()`, the route posture check and the `insecure_http_parser` warning run first, and a failing posture check
   never opens the port) and the body of `listen` / `listen_in_background` (shutdown hooks, signals). `serve` takes a
   `Transport` (`Plain` or `Tls(Arc<TlsSettings>)`) instead of growing a second loop. The start log line says the
   listener is HTTPS.
10. **`get_url()` scheme.** `bind` records the scheme with the address (`local_addr` becomes the address plus a
    `secure` flag); `get_url` returns `https://` when the last listener is TLS. `HttpServer::address()` is unchanged.
    The `127.0.0.1` substitution for unspecified addresses stays; the docs note that the certificate must cover the
    host clients use.
11. **WebSockets over TLS.** Upgrades go through the same `handle` over an HTTP/1.1 TLS connection, so `wss://` needs
    no gateway change. Browsers offer only `http/1.1` in ALPN for WebSocket connections; over an `h2` connection an
    upgrade request is not possible (hyper's HTTP/2 server does not enable extended CONNECT). A regression test runs a
    gateway over `wss://`.
12. **No interaction with route posture.** TLS secures the transport; posture is about which routes have access
    guards. The report and the enforce mode are unchanged. The ordering guarantee (posture check in `init()` before the
    TLS port opens) is tested.
13. **Tests generate their certificates.** A test-only dev-dependency on `rcgen` (ring backend; check its default
    features do not pull `aws-lc-rs`) builds a CA and a leaf for `localhost` and `127.0.0.1` per test run, so no
    private key is committed and nothing expires in the repository. The client side uses `tokio-rustls` (already
    optional; the test binary requires the `tls` feature) with that CA as the only root, hyper's client for HTTP/1.1
    and HTTP/2, and `tokio-tungstenite::client_async` over the TLS stream for `wss://`. The test file is
    `crates/corvis/tests/tls.rs` with `#![cfg(feature = "tls")]`.
14. **CI runs the TLS tests.** `tls` joins the per-feature clippy loop, and the Linux and Windows test jobs add
    `cargo test -p corvis --features tls --test tls`. Without this the tests would never run in CI, as happens today to
    the `health-tls` tests.

## Risks / Trade-offs

- [The `TimedIo` refactor changes plain connections] → the plain variant keeps the code paths; the existing
  `transport_security`, `http1_framing`, `http_adapter`, `websockets` tests and the corvis-parity check must pass
  unchanged.
- [Slow or abandoned handshakes] → per-connection task, 10 s timeout, permit held during the handshake; regression
  tests for a stalled `ClientHello` with and without `max_connections`.
- [h2 over TLS enlarges the attack surface compared with the original's HTTP/1.1-only HTTPS] → hyper's HTTP/2 limits
  and reset protections; `http2(false)` for parity; see Open questions.
- [Certificate rotation needs a restart] → documented; a custom resolver through `from_server_config` covers it today;
  hot reload is a later issue.
- [A user `ServerConfig` with dangerous settings] → 0-RTT is forced off; other choices in a user config (for example
  `ignore_client_order`, a permissive client verifier) are the user's responsibility and documented as such.
- [Key material in memory] → rustls' and `rustls-pki-types`' key types zeroize on drop (`zeroize` is in their
  dependency list in `Cargo.lock`); Corvis does not keep other copies of the PEM bytes after construction.
- [Tests flaky on Windows because of timing] → short handshake timeouts in the tests but wide assertion windows, and
  responses over TLS keep the graceful close of `e0-transport-security-tests-are-flaky-on-windows` (the TLS stream goes
  through the same `linger_close`, decision 4), so an early error response is not lost to a reset.

## Open questions

1. **ALPN `h2` by default.** The issue asks for `h2` and `http/1.1`; the original's Express HTTPS server is HTTP/1.1
   only. Keep `h2` on by default (recommended: HTTP/2 is what clients expect from HTTPS, and `http2(false)` gives
   parity), or default to HTTP/1.1 like the original?
2. **Test certificates.** `rcgen` as a dev-dependency (recommended) or checked-in PEM fixtures with a very long
   validity (no new crate, but private keys in the repository, which secret scanners flag)?
3. **Handshake timeout default.** 10 s (recommended) or another value?
4. **Logging failed handshakes.** Not logged (recommended, to avoid log floods). Should a debug-level line or a counter
   exist for diagnosing certificate problems?

## Review notes (2026-11-09)

- Q1: ALPN offers `h2` and `http/1.1` by default; `http2(false)` gives HTTP/1.1 only, like the original. HTTP/2 limits
  come from #150, which lands first.
- Q2: `rcgen` as a dev-dependency for test certificates; no private keys checked in.
- Q3: handshake timeout 10 s.
- Q4: failed handshakes are not logged.

## Implementation notes

Written after the change was rebased on `batch/phase07-p` (send timeout, response cap, WebSocket write deadline and read
pause). These differ from the design above.

- **A gate under rustls (decision 4 changed).** The layering is TCP, `Gate`, rustls, `TimedIo`, hyper. `ServerIo::Tls`
  holds `TlsStream<Gate>`, not `TlsStream<TcpStream>`. `Gate` (`crates/corvis/src/server/transport.rs`) forwards the socket and
  shares a `Wire` with `TimedIo`: bytes TCP accepted, whether the last TCP write was pending, and the read credit. Reason:
  the constraints recorded on #117 by #157 and #161 need to measure the wire, not the plaintext rustls buffers.
  - Send timeout: `poll_write_deadline` restarts the deadline when TCP accepted bytes since the last look, so a TLS flush
    that stays pending while the client reads slowly is not cut (plaintext writes are accepted into rustls' 64 KiB buffer
    even when TCP is blocked, and flushes can be pending without any plaintext write).
  - Read credit: earned by the bytes TCP accepts (`Gate`), spent by the ciphertext `Gate` reads, and the gate parks the
    read itself once the credit is spent behind a blocked write on an upgraded connection. So the pause acts below TLS:
    rustls cannot be fed more ciphertext, and so cannot queue more replies (`KeyUpdate` with `update_requested`, alerts),
    than the credit allows.
  - Writability check: `TimedIo::read_paused` lifts the pause only when the socket is writable and no ciphertext is left
    in rustls (`wants_write`); it pushes that ciphertext with a flush first, using the pending writer's waker so it does
    not take the writer's wake-up.
  - Not done: a flush after every read to push TLS replies when nobody writes. rustls bounds `KeyUpdate` requests per
    received application data (`TooManyKeyUpdateRequests`), and the read credit bounds what an upgraded peer can interleave.
- **Graceful shutdown (decision 6 changed).** The loop already tracks connections with its own `watch` channel, so a TLS
  task subscribes in the loop and selects on it during the handshake (a handshake in progress is abandoned at shutdown)
  instead of using hyper-util's `GracefulShutdown::watcher()`.
- **Connection task.** The per-connection body of `serve` moved to `run_connection` (`ConnCtx`); plain listeners spawn it
  directly, TLS listeners spawn a task that handshakes first. `Proto` (`Auto`, `H1Only`, `H2Only`) tells `TimedIo` how to
  start: `H2Only` has the framing check off and the preface allowed (ALPN `h2` clients still send it), `H1Only` refuses
  the preface even with `h2c(true)`. The builder comes from `connection_builder` for every protocol (`http1_only()` or
  `http2_only()` on a clone), so the HTTP/2 limits apply over TLS.
- **`get_url`.** A separate `local_secure` flag next to `local_addr`.
- **Tests.** WebSocket tests live in `tests/tls_ws.rs` (they need the `websockets` feature) and the process-wide provider
  test in `tests/tls_provider.rs` (the provider can be installed once per process); helpers in `tests/tls_common/`.
  `src/server/tls/tests.rs` (loading, errors, hardening) and `src/server/transport_tests.rs` (the gate) are unit tests.
  The CI step runs `--lib --test tls --test tls_ws --test tls_provider`. Test certificates come from `rcgen` 0.14 (ring
  backend, `default-features = false`), a dev-dependency only.
- **Per-feature clippy.** `tls` joins the list: `cargo clippy -p corvis --no-default-features --features tls -- -D warnings`.
