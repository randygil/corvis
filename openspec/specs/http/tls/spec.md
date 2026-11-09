# http/tls Specification

## Purpose
How the built-in server terminates TLS itself (the opt-in `tls` feature, rustls): certificate loading, ALPN between
HTTP/1.1 and HTTP/2, the handshake timeout, and how the protections of the plain server (framing checks, HTTP/2
limits, send timeout, response memory cap, WebSocket read pause) keep working over TLS.

## Requirements

### Requirement: TLS is an opt-in feature built on rustls
TLS termination SHALL be available only with the `tls` cargo feature of `corvis`, which MUST NOT be a default feature.
Without it, the public API and the normal dependency tree of `corvis` MUST be the same as before this change. With it,
TLS MUST be implemented with rustls (no OpenSSL), using the process-wide rustls `CryptoProvider` when one is installed
and the `ring` provider otherwise, and MUST NOT panic when more than one provider is compiled in.

#### Scenario: Default build
- **WHEN** `cargo tree -p corvis -e normal` runs with default features
- **THEN** neither `tokio-rustls` nor `rustls` appears under `corvis`

#### Scenario: Process-wide provider installed
- **WHEN** an application installs, as the process-wide default, a `ring`-based provider restricted to one TLS 1.3
  cipher suite, and then builds `TlsOptions::from_pem(..)`
- **THEN** every TLS 1.3 handshake with that server negotiates that cipher suite

#### Scenario: No provider installed
- **WHEN** no process-wide provider is installed and `TlsOptions::from_pem(..)` is called
- **THEN** it returns `Ok` without panicking and handshakes succeed with the `ring` provider

### Requirement: TLS options are built from PEM or from a rustls configuration
`TlsOptions::from_pem_files(cert_chain, key)` and `TlsOptions::from_pem(cert_chain, key)` SHALL load every certificate
of the PEM chain in order and exactly one private key in PKCS#8, PKCS#1 (RSA) or SEC1 (EC) form.
`TlsOptions::from_server_config(config)` SHALL accept a ready `rustls::ServerConfig`.

#### Scenario: Valid PEM files
- **WHEN** `TlsOptions::from_pem_files("cert.pem", "key.pem")` is called with a leaf and intermediate certificate and
  its PKCS#8 key
- **THEN** it returns `Ok`, and a TLS client that trusts the root receives both certificates in the handshake

#### Scenario: Ready configuration
- **WHEN** `TlsOptions::from_server_config(config)` receives a configuration built with rustls' SNI certificate
  resolver holding certificates for `a.test` and `b.test`
- **THEN** a client that sends the server name `b.test` receives the `b.test` certificate

### Requirement: Loading errors are early and never contain key material
The `TlsOptions` constructors MUST fail with a `TlsError`, before the application initializes, when a file cannot be
read (naming the path), when there is no certificate, no private key or more than one private key, when the key is
encrypted (saying it must be decrypted first), or when rustls rejects the key or it does not match the certificate. No
error message MAY contain key material.

#### Scenario: Missing key file
- **WHEN** the key path does not exist
- **THEN** it returns `Err(TlsError::Io { .. })` whose message contains the path

#### Scenario: Encrypted key
- **WHEN** the key file contains an `ENCRYPTED PRIVATE KEY` block
- **THEN** it returns an error that says encrypted keys must be decrypted first

#### Scenario: Key of another certificate
- **WHEN** the certificate and the key do not belong together
- **THEN** it returns an error, and the error text contains no line of the key file

### Requirement: TLS defaults are secure without configuration
Options built from PEM SHALL accept TLS 1.2 and TLS 1.3 only, use rustls' default cipher suites, request no client
certificate, and serve the same certificate whatever server name the client sends (or none). Every `TlsOptions`,
including one built with `from_server_config`, SHALL refuse 0-RTT early data.

#### Scenario: Old protocol version
- **WHEN** a client offers only TLS 1.1
- **THEN** the handshake fails and no HTTP response is sent

#### Scenario: Early data in a user configuration
- **WHEN** `from_server_config` receives a configuration with `max_early_data_size` set to 16384
- **THEN** the server accepts no early data on any connection

#### Scenario: No SNI
- **WHEN** a client connects without sending a server name
- **THEN** the handshake succeeds with the configured certificate

### Requirement: ALPN advertises HTTP/2 and HTTP/1.1
ALPN SHALL advertise `h2` and `http/1.1` by default and only `http/1.1` after `.http2(false)`. A user configuration
that already lists ALPN protocols SHALL be kept, minus `h2` when `.http2(false)` is set.

#### Scenario: Default ALPN
- **WHEN** a client offers `h2` and `http/1.1` to a server built with `TlsOptions::from_pem(..)`
- **THEN** the negotiated protocol is `h2`

#### Scenario: HTTP/2 turned off
- **WHEN** a server uses `TlsOptions::from_pem(..)?.http2(false)` and a client offers `h2` and `http/1.1`
- **THEN** the negotiated protocol is `http/1.1`

### Requirement: HTTPS listeners serve the same application as plain ones
`app.listen_tls(addr, tls)` and `app.listen_tls_in_background(addr, tls)` SHALL initialize the application (running the
route posture check) before binding, and SHALL serve the same routes, middleware and gateways as the plain listeners.
Plain and TLS listeners MAY run at the same time on different addresses.

#### Scenario: HTTPS request
- **WHEN** an application listens with `listen_tls_in_background("127.0.0.1:0", tls)` and a client sends
  `GET /cats` over TLS
- **THEN** the response is the same status, headers and body as over a plain listener

#### Scenario: Plain and TLS together
- **WHEN** an application calls `listen_in_background("127.0.0.1:0")` and `listen_tls_in_background("127.0.0.1:0",
  tls)`
- **THEN** both addresses answer, one over plain HTTP and one over TLS

#### Scenario: Posture check before the port opens
- **WHEN** route posture is enforced, a route has no access guard, and the application calls `listen_tls(..)`
- **THEN** `listen_tls` returns the posture error and nothing listens on the address

### Requirement: Server options apply to TLS connections
`ServerOptions` (`max_connections`, `headers_timeout`, `request_timeout`, `keep_alive_timeout`, `max_header_size`, the
HTTP/1 framing check and `insecure_http_parser`) MUST apply to TLS connections as they apply to plain ones, with the
headers timeout counted from the end of the handshake.

#### Scenario: Framing check over TLS
- **WHEN** a client sends, over a TLS connection that negotiated `http/1.1`, a request with both `Content-Length` and
  `Transfer-Encoding: chunked`
- **THEN** it receives the same `400` and connection close as over a plain connection

#### Scenario: Headers timeout over TLS
- **WHEN** `headers_timeout` is 1 s and a client completes the handshake and then sends nothing
- **THEN** the server closes the connection about 1 s after the handshake

### Requirement: The TLS handshake cannot block the server
The TLS handshake SHALL run in the connection's own task, never in the accept loop, and SHALL be bounded by the
handshake timeout (10 s by default, set with `TlsOptions::handshake_timeout`). A connection MUST hold its
`max_connections` permit while it handshakes. A handshake that fails or times out MUST close the socket without sending
HTTP data and without the closing drain, and MUST release the permit.

#### Scenario: Stalled handshake
- **WHEN** one client opens a TCP connection to a TLS listener and sends nothing (or half a `ClientHello`), and a second
  client then sends an HTTPS request
- **THEN** the second client gets its response without waiting for the first, and the server closes the first
  connection at the handshake timeout

#### Scenario: Handshakes count against max_connections
- **WHEN** `max_connections` is 1, the handshake timeout is 1 s, and one client opens a connection and stalls its
  handshake
- **THEN** a second client's connection is not served until the first is closed at the handshake timeout, and is then
  answered

#### Scenario: Plain HTTP to the TLS port
- **WHEN** a client sends `GET / HTTP/1.1` in cleartext to a TLS listener
- **THEN** the server closes the connection and sends no HTTP response

### Requirement: The application protocol of a TLS connection comes from ALPN
A TLS connection that negotiated `h2` SHALL be served as HTTP/2 only. A TLS connection that negotiated `http/1.1`, or no
ALPN protocol, SHALL be served as HTTP/1.x only and MUST refuse the HTTP/2 connection preface, whatever the `h2c`
option says. A connection that negotiated any other protocol MUST be closed.

#### Scenario: HTTP/2 over TLS
- **WHEN** a client offers ALPN `h2` and sends `GET /cats`
- **THEN** the response has version HTTP/2 and status `200`

#### Scenario: HTTP/2 preface without ALPN
- **WHEN** a client completes a handshake without ALPN and sends the HTTP/2 connection preface, on a server with
  `h2c(true)`
- **THEN** the server closes the connection without a response

### Requirement: URLs and WebSockets over TLS
After `listen_tls` or `listen_tls_in_background`, `app.get_url()` SHALL return an `https://` URL with the bound port,
as long as the TLS listener is the last one started. WebSocket gateways SHALL accept `wss://` connections on a TLS
listener, with the same handshake checks and message handling as `ws://`.

#### Scenario: get_url
- **WHEN** an application starts only `listen_tls_in_background("127.0.0.1:0", tls)`
- **THEN** `app.get_url()` returns `https://127.0.0.1:<port>`

#### Scenario: Gateway over wss
- **WHEN** a client connects to a gateway with `wss://` through a TLS listener and sends a message the gateway answers
- **THEN** it receives the same reply as over `ws://`
