# Spec Delta

## ADDED Requirements

### Requirement: The test client reads a request like the server
A request sent through `TestClient` SHALL go through the same request path as a request received by a listening
server: the body limits of the body parser and of the transport, the refusal of a declared length over the limit, the
pending multipart body read by file interceptors, and the response finishing. Only the transport (sockets, TLS,
upgrades, Engine.IO) is left out.

#### Scenario: Body over the parser limit
- **WHEN** a test sends a JSON body larger than the JSON parser's limit through `TestClient`
- **THEN** the response is `413`, as a listening server answers it

#### Scenario: Declared length over the limit
- **WHEN** a test sends a request whose `Content-Length` exceeds the transport limit
- **THEN** the response is `413` and the body is not parsed

#### Scenario: Multipart upload
- **WHEN** a test uploads a file through `TestClient` to a route with a file interceptor
- **THEN** the interceptor reads the body through the same streaming path as over a socket

#### Scenario: Same answer as a listening server
- **WHEN** the same request is sent through `TestClient` and to a listening server of the same application
- **THEN** both answers have the same status, the same headers apart from `Date`, and the same body
