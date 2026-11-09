# Proposal

Implements [randygil/corvis#117](https://github.com/randygil/corvis/issues/117) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Design notes: the #23 spike (archived
`2026-11-08-e13-missing-core-features-users-expect/design.md`, feature 3, and its review notes).

## Why

The built-in server accepts only plain HTTP/1.1, plus h2c when enabled (`crates/corvis/src/server.rs:811-885`,
`crates/corvis/src/application.rs:597-656`). HTTPS needs a reverse proxy (`docs/faq/multiple-servers.md`,
`docs/deployment.md:63`). Small deployments and internal services (service-to-service HTTPS, a single VM, a
development setup that must match production URLs) want to serve HTTPS directly. The original framework offers
`httpsOptions`; migrated apps that use it have no equivalent today.

## What Changes

- A `tls` cargo feature of `corvis`, off by default, that enables `tokio-rustls` with the `ring` provider (both already
  in the dependency graph through the `health-tls` feature and corvis-orm). No OpenSSL.
- `corvis::tls::TlsOptions`, built from PEM files (`from_pem_files`), PEM bytes (`from_pem`) or a ready
  `rustls::ServerConfig` (`from_server_config`); `corvis::tls::rustls` re-exports the rustls version Corvis uses.
  Options: `handshake_timeout(Duration)` (default 10 s) and `http2(bool)` (default `true`).
- Secure defaults: TLS 1.2 and 1.3 only, rustls' default cipher suites (AEAD with forward secrecy), 0-RTT early data
  always off, no client certificates, one certificate for every server name, ALPN `h2` and `http/1.1`.
- `app.listen_tls(addr, tls)` and `app.listen_tls_in_background(addr, tls)`, sharing the accept loop with `listen`
  and `listen_in_background`. `ServerOptions` (`max_connections`, headers, request and keep-alive timeouts,
  `max_header_size`, the HTTP/1 framing check, `insecure_http_parser`) applies to TLS listeners as to plain ones.
- The TLS handshake runs in the per-connection task, with its own timeout, never in the accept loop, and the
  connection holds its `max_connections` permit while it handshakes.
- The protocol of each TLS connection comes from ALPN: `h2` serves HTTP/2 only; `http/1.1` or no ALPN serves HTTP/1.x
  only, with the HTTP/1 framing check, and refuses the HTTP/2 preface (h2 over TLS requires ALPN; `h2c` stays a
  cleartext-only option).
- `get_url()` returns `https://` after a TLS listener starts. WebSocket gateways work over `wss://`.
- Errors loading certificates or keys are returned by the `TlsOptions` constructors, before the application
  initializes, and never contain key material.
- CI: clippy with `--features tls` alone, and the TLS tests run on Linux and Windows. `cargo deny` keeps passing.
- Docs (EN + ES): HTTPS and multiple servers, deployment, HTTP adapters, HTTP parsing. Example: an optional `tls`
  feature in cats-app.

## Capabilities

### New Capabilities

- `http/tls`: TLS termination in the built-in server: options, defaults, handshake limits, protocol selection, URLs,
  WebSockets over TLS.

### Modified Capabilities

- `infra/ci`: the `tls` feature is checked on its own and its tests run.

## Impact

- Code: `crates/corvis/Cargo.toml` (feature), a new `crates/corvis/src/server/tls.rs` (`TlsOptions`, `TlsError`, PEM
  loading, config hardening), `crates/corvis/src/server.rs` (transport enum under `TimedIo`, handshake in the
  connection task, per-ALPN protocol, `linger_close` over TLS), `crates/corvis/src/application.rs` (`listen_tls`,
  `listen_tls_in_background`, scheme for `get_url`), `crates/corvis/src/lib.rs` (`pub mod tls` under the feature),
  `.github/workflows/ci.yml`, `examples/cats-app`.
- Public API: new items only, all behind the feature. Without it, the public API and the dependency tree of `corvis`
  are unchanged.
- Plain listeners: the accept loop and `TimedIo` are refactored to carry a transport; plain connections must behave
  exactly as before (existing transport and framing tests).
- Dependencies: no new crate in the normal graph (`tokio-rustls`, `rustls`, `rustls-pki-types` and `ring` are already
  in `Cargo.lock` at `tokio-rustls` 0.26.6, `rustls` 0.23.46, `rustls-pki-types` 1.15.1, `ring` 0.17.14). One test-only
  dev-dependency to generate certificates (`rcgen`, ring backend), unless the maintainer prefers checked-in fixtures
  (design, Open questions).
- Ports: the three port apps and their originals use no `httpsOptions` and are tested over plain HTTP; apidiff must stay
  at 0 differences (the refactor touches the shared accept loop).
- Docs (EN + ES): `faq/multiple-servers.md`, `deployment.md`, `http/performance.md`, `security/http-parsing.md`.

## Out of scope

- Certificate hot reload without restart. The design keeps it possible (a custom `ResolvesServerCert` through
  `from_server_config` already allows it today); a dedicated API is a later issue.
- Client certificates (mTLS) as a first-class option, and exposing the peer certificate to handlers. Possible today
  through `from_server_config` with a client verifier, without access from the `Request`; later issue.
- ACME / automatic certificates.
- Multiple certificates selected by SNI as a first-class option (possible through `from_server_config` with rustls'
  SNI resolver; documented), and exposing the SNI name to handlers.
- `req.protocol()` / `req.secure()` and `X-Forwarded-Proto` handling: a separate gap found by the #23 spike.
- An `https_options` application option mirroring the original's `httpsOptions`: rejected by the #23 review (one
  setting for every listener prevents "HTTPS plus an internal HTTP port").
- Encrypted private keys (the original's `passphrase`), PKCS#12 / PFX bundles, OpenSSL / native-tls, the `aws-lc-rs`
  provider as Corvis' choice (it is used when the application installs it as the process default).
- Certificate expiry warnings at startup (needs an X.509 parser dependency).
- HTTP/3 / QUIC, HSTS headers set automatically (see the security headers page), plain-to-HTTPS redirects.
- WebSockets over HTTP/2 (RFC 8441 extended CONNECT).
