# Spec Delta

## ADDED Requirements

### Requirement: Closing the application closes gateway connections
`close()` SHALL close every connection of the application's WebSocket gateways (the `ws` protocol) with a close frame of
code 1001 and reason `Server shutting down`, sent after the frames already queued for that client. A peer that does not
answer within the gateway's `close_timeout` MUST have its TCP connection destroyed.

#### Scenario: Connected client at close
- **WHEN** a client is connected to a gateway of an application served with `listen_in_background` and `close()` is called
- **THEN** the client receives a close frame with code 1001 and the gateway server's `count()` is 0

#### Scenario: A peer that never answers
- **WHEN** the client does not answer the close frame
- **THEN** its TCP connection is destroyed once `close_timeout` has elapsed

### Requirement: WsServer::for_path does not return servers of closed applications
When an application is closed or dropped, `WsServer::for_path` SHALL stop returning that application's gateway servers.
A path that another live application registered SHALL keep returning that application's server; a path no live
application registered SHALL return a new, empty server.

#### Scenario: Two applications on one path
- **WHEN** two applications mount a gateway at `/chat` and the second one, registered last, is closed
- **THEN** `WsServer::for_path("/chat")` returns the first application's server

#### Scenario: All applications closed
- **WHEN** every application that mounted `/chat` is closed or dropped
- **THEN** `WsServer::for_path("/chat")` returns a server with no clients
