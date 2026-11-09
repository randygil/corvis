# Spec Delta

## ADDED Requirements

### Requirement: WebSocket handshakes use the application's trust setting
The handshake request of a WebSocket gateway connection SHALL resolve `ip()`, `ips()` and `hostname()` with the
`trust proxy` setting of the application (`set_trust_proxy`), exactly as an HTTP request to the same server with the
same socket address and headers does. This request is the one returned by `client.handshake()`, passed to
`GatewayOptions::verify_client` and returned by `ctx.request()` in gateway guards, interceptors and pipes.

#### Scenario: Gateway behind one trusted proxy
- **WHEN** an application with `set_trust_proxy(1)` accepts a WebSocket connection from `127.0.0.1` whose handshake
  carries `X-Forwarded-For: 203.0.113.7`
- **THEN** `client.handshake().ip()` is `203.0.113.7` and `client.handshake().ips()` is `["203.0.113.7"]`

#### Scenario: Default setting ignores forwarded headers
- **WHEN** an application without a trust setting accepts a WebSocket connection from `127.0.0.1` whose handshake
  carries `X-Forwarded-For: 203.0.113.7`
- **THEN** `client.handshake().ip()` is `127.0.0.1`

#### Scenario: Forwarded host
- **WHEN** an application with `set_trust_proxy("loopback")` accepts a WebSocket connection from `127.0.0.1` whose
  handshake carries `X-Forwarded-Host: chat.example.com`
- **THEN** `client.handshake().hostname()` is `chat.example.com`

#### Scenario: verify_client sees the forwarded address
- **WHEN** a gateway with `set_trust_proxy(1)` has a `verify_client` that accepts only `203.0.113.7`, and a handshake
  from `127.0.0.1` carries `X-Forwarded-For: 203.0.113.7`
- **THEN** the connection is accepted, and the same handshake without the header is rejected with `401`

### Requirement: The throttler keys gateway messages by the forwarded client
A `ThrottlerGuard` that applies to a gateway SHALL key messages by the handshake address resolved with the trust
setting, so that clients behind the same trusted proxy have separate counters.

#### Scenario: Two clients behind one proxy
- **WHEN** an application with `set_trust_proxy(1)` and a global `ThrottlerGuard` limited to 2 messages per minute
  receives three messages from a client with `X-Forwarded-For: 203.0.113.7` and one from a client with
  `X-Forwarded-For: 203.0.113.8`, both through `127.0.0.1`
- **THEN** the first client's third message gets the throttler's `exception` reply and the second client's message is
  handled

#### Scenario: No trust setting
- **WHEN** the same application without a trust setting receives those messages
- **THEN** both clients share the counter of `127.0.0.1`, and the second client's message is throttled

### Requirement: Upgrade listeners use the application's trust setting
The request given to `upgrade` listeners (`HttpServer::on_upgrade`, `UpgradeRequest::request()`) and kept by
`corvis::ws` connections (`upgrade_request()`) SHALL resolve `ip()`, `ips()` and `hostname()` with the application's
trust setting.

#### Scenario: Upgrade listener behind a proxy
- **WHEN** an application with `set_trust_proxy(1)` has an `upgrade` listener and receives an upgrade request from
  `127.0.0.1` with `X-Forwarded-For: 203.0.113.7`
- **THEN** `req.request().ip()` in the listener is `203.0.113.7`
