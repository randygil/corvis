# Spec Delta

## ADDED Requirements

### Requirement: WebSocket handshakes use the application's query parser
The handshake request of a WebSocket gateway connection SHALL parse its query string with the application's query
parser setting (`set_query_parser`), exactly as an HTTP request to the same URL does. This request is the one returned
by `client.handshake()`, passed to `GatewayOptions::verify_client` and returned by `ctx.request()` in gateway guards,
interceptors and pipes. The raw query string MUST stay available through `uri().query()` whatever the setting.

#### Scenario: Extended parser
- **WHEN** an application with `set_query_parser(QueryParser::Extended)` accepts a WebSocket connection to
  `/chat?room[id]=7&tags[]=a&tags[]=b`
- **THEN** `client.handshake().query()` is `{"room": {"id": "7"}, "tags": ["a", "b"]}`, the same object an HTTP route
  of that application gets from `#[query]` for that query string

#### Scenario: Default parser
- **WHEN** an application without a query parser setting accepts a WebSocket connection to `/chat?room[id]=7&a=1&a=2`
- **THEN** `client.handshake().query()` is `{"room[id]": "7", "a": ["1", "2"]}`

#### Scenario: Disabled parser
- **WHEN** an application with `set_query_parser(QueryParser::Disabled)` accepts a WebSocket connection to
  `/chat?token=abc`
- **THEN** `client.handshake().query()` is empty, `client.handshake().query_param("token")` is `None`, and
  `client.handshake().uri().query()` is `Some("token=abc")`

#### Scenario: verify_client sees the parsed query
- **WHEN** an application with `set_query_parser(QueryParser::Extended)` has a gateway whose `verify_client` accepts
  only handshakes whose query has an object `auth` with a string `token`, and a client connects with
  `?auth[token]=abc`
- **THEN** the connection is accepted, and a connection with `?auth=abc` is rejected with `401`

#### Scenario: Trust setting is kept
- **WHEN** an application with `set_query_parser(QueryParser::Extended)` and `set_trust_proxy(1)` accepts a WebSocket
  connection from `127.0.0.1` whose handshake carries `X-Forwarded-For: 203.0.113.7`
- **THEN** `client.handshake().ip()` is `203.0.113.7`

### Requirement: Upgrade listeners use the application's query parser
The request given to `upgrade` listeners (`HttpServer::on_upgrade`, `UpgradeRequest::request()`) and kept by
`corvis::ws` connections (`upgrade_request()`) SHALL parse its query string with the application's query parser
setting. `UpgradeRequest::query_param(name)` SHALL return the string value of `name` in that parsed query, and
`UpgradeRequest::query_string()` SHALL return the raw query string unchanged.

#### Scenario: Upgrade listener with the extended parser
- **WHEN** an application with `set_query_parser(QueryParser::Extended)` has an `upgrade` listener and receives an
  upgrade request for `/raw?user[name]=ana`
- **THEN** `req.request().query()` in the listener is `{"user": {"name": "ana"}}` and `req.query_string()` is
  `Some("user[name]=ana")`

#### Scenario: Upgrade listener with the parser disabled
- **WHEN** an application with `set_query_parser(QueryParser::Disabled)` has an `upgrade` listener and receives an
  upgrade request for `/raw?token=abc`
- **THEN** `req.query_param("token")` is `None` and `req.query_string()` is `Some("token=abc")`
