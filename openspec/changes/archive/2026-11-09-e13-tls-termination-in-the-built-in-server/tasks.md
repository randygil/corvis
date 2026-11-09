# Tasks

## 1. Feature and options

- [x] 1.1 Add `tls = ["dep:tokio-rustls"]` to `crates/corvis/Cargo.toml:50-54` (not in `default`); confirm with
  `cargo metadata --no-deps` and `cargo tree -p corvis -e normal` that the default tree has no `rustls`, and that
  `rustls::pki_types::pem::PemObject` is available with the features `tokio-rustls` enables (otherwise add
  `rustls-pki-types` with its `std` feature, already in `Cargo.lock` at 1.15.1); verify
  `cargo clippy -p corvis --no-default-features --features tls -- -D warnings`
- [x] 1.2 Create `crates/corvis/src/server/tls.rs` with `TlsOptions`, `TlsError` and the PEM constructors (design
  decision 3; provider choice of decision 2, reusing the logic of `crates/corvis/src/modules/health.rs:376`), and the
  hardening of decision 5 applied in one private function used by all constructors; expose it as `corvis::tls` with a
  `rustls` re-export in `crates/corvis/src/lib.rs` under `#[cfg(feature = "tls")]`; rustdoc with an example; verify unit
  tests in `tls.rs` for each `TlsError` variant (PEM built in the test from `rcgen` output), the key-mismatch case (if
  `with_single_cert` in rustls 0.23.46 does not report it, check it with `CertifiedKey::keys_match`), no key text in
  any error message, `max_early_data_size == 0` after `from_server_config`, and the ALPN lists of decision 5
- [x] 1.3 Add `rcgen` as a dev-dependency of `corvis` (ring backend only; `cargo tree -p corvis -e dev` shows no
  `aws-lc-rs`), unless Open question 2 chose fixtures; verify `cargo deny check` passes

## 2. Server

- [x] 2.1 Introduce `ServerIo` (decision 4) in `crates/corvis/src/server.rs`: `TimedIo.io` (`server.rs:478-498`),
  `ConnState::closing` (`server.rs:410`), `TimedIo::sock` and the `AsyncRead`/`AsyncWrite` impls
  (`server.rs:594-596`, `751-808`), and `linger_close` (`server.rs:510-523`) work on it; plain connections only so far;
  verify the existing tests unchanged: `cargo test -p corvis --test transport_security --test http1_framing
  --test http_adapter --test websockets`
- [x] 2.2 Add `Transport { Plain, Tls(..) }` to `serve` (`server.rs:811-885`), and for TLS move the per-connection work
  into the spawned task: handshake under the timeout, permit held from the start, no linger on failure, graceful
  watcher taken in the loop (decision 6); verify with tasks 4.2 and 4.3
- [x] 2.3 Choose the protocol per TLS connection from ALPN (decision 8): an HTTP/2-only builder with `TimedIo`'s preface
  and framing checks off for `h2`, an HTTP/1-only builder with both checks on for `http/1.1` or none, close otherwise;
  plain listeners keep the current `auto::Builder`; verify with task 4.4

## 3. Application

- [x] 3.1 Add `listen_tls` and `listen_tls_in_background` to `crates/corvis/src/application.rs` sharing `bind` and the
  bodies of `listen` / `listen_in_background` (`application.rs:597-656`), with an HTTPS start log line (decision 9);
  record the scheme with `local_addr` and make `get_url` return `https://` for TLS (`application.rs:659-664`,
  decision 10); verify with task 4.5

## 4. Regression tests

All in `crates/corvis/tests/tls.rs` (`#![cfg(feature = "tls")]`), against `listen_tls_in_background("127.0.0.1:0",
..)`, with a client that trusts only the test CA. One test per scenario of `specs/http/tls/spec.md`; run with
`cargo test -p corvis --features tls --test tls`.

- [x] 4.1 Loading and defaults: valid chain (client sees leaf and intermediate), ready configuration with rustls' SNI
  resolver (client gets the certificate of the name it sends), missing file, encrypted key, key mismatch without key
  text, process-wide provider honored, no provider installed, TLS 1.1 refused (a static
  TLS 1.1 `ClientHello` written to the socket gets an alert or a close, never HTTP), early data refused with a user
  config, default ALPN negotiates `h2`, `http2(false)` negotiates `http/1.1`, no SNI accepted; verify the command above
- [x] 4.2 Stalled handshake does not block a second client; the stalled socket is closed at the handshake timeout
  (timeout 500 ms in the test, assertion window several seconds); verify the command above
- [x] 4.3 `max_connections(1)` with a stalled handshake: the second connection is answered only after the timeout;
  plain HTTP sent to the TLS port is closed without an HTTP response; verify the command above
- [x] 4.4 HTTP/2 over ALPN `h2` answers with version HTTP/2; HTTP/2 preface without ALPN is closed even with
  `h2c(true)`; framing check (`Content-Length` with `Transfer-Encoding`) over TLS gives the same `400` as plain;
  headers timeout counted after the handshake; verify the command above
- [x] 4.5 Same response over plain and TLS listeners of one application; posture enforced with an unguarded route makes
  `listen_tls` fail and nothing listens; `get_url` returns `https://127.0.0.1:<port>`; a gateway answers over `wss://`
  (`tokio-tungstenite` `client_async` over the TLS stream); verify the command above
- [x] 4.6 Run the TLS tests on Windows as well (locally or in CI) and repeat 4.2-4.4 ten times in a loop to check for
  flakiness; verify no failure

## 5. CI

- [x] 5.1 `.github/workflows/ci.yml`: add `tls` to the per-feature clippy loop (line 67) and a step
  `cargo test -p corvis --features tls --test tls` after `cargo test --workspace` in the Linux (line 107) and Windows
  (line 128) jobs; verify the workflow on the pull request

## 6. Example

- [x] 6.1 `examples/cats-app`: an optional `tls = ["corvis/tls"]` feature; when it is on and `CATS_TLS_CERT` /
  `CATS_TLS_KEY` are set, `main.rs` also starts an HTTPS listener (port from `CATS_TLS_PORT`, default 3443) next to the
  plain one; document the variables in `examples/cats-app/.env.example` without any key material; verify
  `cargo build -p cats-app --features tls` and `cargo test -p cats-app` (without the feature)

## 7. Documentation

- [x] 7.1 `docs/faq/multiple-servers.md`: rewrite the HTTPS section (built-in HTTPS with the `tls` feature, `listen_tls`,
  `TlsOptions` constructors, defaults and why, handshake timeout as a divergence from Node's 120 s, HTTP/2 via ALPN
  and `http2(false)`, SNI and custom resolvers through `from_server_config`, certificate rotation needs a restart,
  `get_url`, `wss://`, when a reverse proxy is still the better choice), and an "HTTPS on 443 plus internal HTTP"
  example; update the front matter `status`; same in `docs/es/faq/multiple-servers.md`; verify `npm run docs:check`
  in `docs/`
- [x] 7.2 `docs/deployment.md` (line 63 and the checklist at line 177) and `docs/http/performance.md` (line 27): replace
  "does not terminate TLS" with the two options (proxy or `tls` feature); `docs/security/http-parsing.md` (line 99):
  HTTP/2 over TLS is negotiated with ALPN and the framing check applies to HTTP/1.1 over TLS; same in `docs/es/`;
  verify `npm run docs:check`

## 8. Definition of done

- [x] 8.1 `cargo build --workspace --all-targets` with no warnings, and `cargo build -p corvis --features tls
  --all-targets` with no warnings
- [x] 8.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature checks including
  `cargo clippy -p corvis --no-default-features --features tls -- -D warnings`
- [x] 8.3 `cargo test --workspace` passes, and `cargo test -p corvis --features tls --test tls` passes on Linux and
  Windows
- [x] 8.4 Regression tests: every scenario of `specs/http/tls/spec.md` maps to a test in `crates/corvis/tests/tls.rs`
  (list the mapping in the PR description); `cargo deny check` passes
- [x] 8.5 Docs updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes
- [x] 8.6 Framework runtime code changed (the shared accept loop and `TimedIo`): run the `corvis-parity` check and
  verify the three port apps (consi, facten, houseplan) keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #117, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

## 9. Review notes, security round (2026-11-09)

- [x] 9.1 `Proto::H2Only` rejects anything but the HTTP/2 preface in `TimedIo`, and is served through the HTTP/2-only
  `serve_connection` path (no HTTP/1 fallback); verify `cargo test -p corvis --features tls --test tls`
- [x] 9.2 Regression tests: HTTP/1 request and upgrade over ALPN `h2` get no HTTP bytes; the preface over ALPN
  `http/1.1` still closes; reviewer PoC `smuggle` re-run
- [x] 9.3 Docs EN + ES: what is enforced for `h2` connections, `headers_timeout` after the handshake, handshake cost
  and `max_connections`, partial ClientHello cost; `npm run docs:check`
