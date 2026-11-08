# Spec Delta

## ADDED Requirements

### Requirement: Request head lines end with CRLF
By default, every line of an HTTP/1 request head (the request line, each header line and the empty line that ends
the head) MUST end with CRLF. A bare LF in any of these positions SHALL make the server answer `400 Bad Request` and
close the connection, as Node's `http` module does, and no handler runs for that request. Empty lines before the
request line (CRLF or bare LF) are still ignored, as Node ignores them.

#### Scenario: Bare LF in the request line and headers
- **WHEN** a client sends `GET / HTTP/1.1\nHost: x\n\n`
- **THEN** the response is `400` and the server closes the connection

#### Scenario: Head ended by CRLF LF
- **WHEN** a client sends `GET / HTTP/1.1\r\nHost: x\r\n\n`
- **THEN** the response is `400` and the server closes the connection

#### Scenario: Last header line ended by a bare LF
- **WHEN** a client sends `GET / HTTP/1.1\r\nHost: x\n\r\n`
- **THEN** the response is `400` and the server closes the connection

#### Scenario: Empty line before the request line
- **WHEN** a client sends `\r\nGET / HTTP/1.1\r\nHost: x\r\n\r\n`, or the same request preceded by a bare `\n`
- **THEN** the request is answered normally, as if the empty line were absent

### Requirement: Chunk size line has no whitespace
By default, in a request with `Transfer-Encoding: chunked`, the chunk size MUST be followed directly by CRLF or by
`;` (a chunk extension). Whitespace (space or tab) between the chunk size and CRLF or `;` SHALL make the request fail
with `400` or a closed connection; no handler receives the body.

#### Scenario: Space after the chunk size
- **WHEN** a request to a JSON route sends the chunked body `2 \r\nhi\r\n0\r\n\r\n`
- **THEN** the response is `400` or the connection is closed, and no handler runs with a body

#### Scenario: Tab after the chunk size
- **WHEN** a request sends the chunked body `2\t\r\nhi\r\n0\r\n\r\n`
- **THEN** the response is `400` or the connection is closed, and no handler runs with a body

#### Scenario: Space before a chunk extension
- **WHEN** a request sends the chunked body `2 ;a=b\r\nhi\r\n0\r\n\r\n`
- **THEN** the response is `400` or the connection is closed, and no handler runs with a body

#### Scenario: Chunk extension without whitespace
- **WHEN** a request to a JSON route sends the chunked body `2;a=b\r\nhi\r\n0\r\n\r\n` with a valid JSON chunk
  instead of `hi`
- **THEN** the handler receives the chunk data as the body

### Requirement: Content-Length with Transfer-Encoding is rejected
By default, a request that carries both a `Content-Length` header and a `Transfer-Encoding` header, in any order,
SHALL be answered with `400 Bad Request` and the connection closed, as Node does. No handler runs for it.

#### Scenario: Content-Length before Transfer-Encoding
- **WHEN** a client sends `Content-Length: 4` and then `Transfer-Encoding: chunked` with the chunked body
  `2\r\nhi\r\n0\r\n\r\n`
- **THEN** the response is `400`, the server closes the connection and no handler runs

#### Scenario: Transfer-Encoding before Content-Length
- **WHEN** a client sends `Transfer-Encoding: chunked` and then `Content-Length: 4` with the same body
- **THEN** the response is `400`, the server closes the connection and no handler runs

### Requirement: Trailer lines end with CRLF
By default, every line of the trailer section of a chunked request, and the empty line that ends it, MUST end with
CRLF. A bare LF anywhere in the trailer section SHALL make the request fail with `400` or a closed connection; the
handler never receives that body or its trailers.

#### Scenario: Bare LF between two trailer fields
- **WHEN** a request sends the chunked body `2\r\nhi\r\n0\r\nX-A: b\nX-C: d\r\n\r\n`
- **THEN** the response is `400` or the connection is closed, and no handler runs with a body

#### Scenario: Bare LF LF inside the trailer section
- **WHEN** a request sends the chunked body `2\r\nhi\r\n0\r\nX-A: b\n\nGARBAGE\r\n\r\n` followed by a pipelined
  request
- **THEN** the response is `400` or the connection is closed, and the pipelined request is not processed

#### Scenario: Valid trailer field
- **WHEN** a request to a JSON route sends a valid JSON chunk, the last chunk and the trailer line `X-A: b\r\n`
  followed by the final CRLF
- **THEN** the handler receives the chunk data as the body

### Requirement: A framing rejection ends the connection
When a request is rejected for one of the framing errors of this capability (the requirements above, and the
chunked framing errors that were already rejected), the server MUST NOT read or process any later request sent on
the same connection, pipelined or not. Responses to requests that came before the rejected one are still sent.

#### Scenario: Rejected request followed by a valid one
- **WHEN** a client sends, in one write, `GET / HTTP/1.1\nHost: x\n\n` followed by a valid `GET` to a route that
  counts its calls
- **THEN** the client receives at most one response, a `400`, the connection is closed, and the route was not called

#### Scenario: Valid request followed by a rejected one
- **WHEN** a client sends, in one write, a valid `GET` followed by `GET / HTTP/1.1\nHost: x\n\n`
- **THEN** the client receives the `200` of the first request, then a `400` or the end of the connection, and no
  handler runs for the second request

#### Scenario: Rejected chunked body followed by a valid request
- **WHEN** a client sends a chunked request with `2 \r\n` as its first chunk size, followed in the same write by a
  valid `GET` to a route that counts its calls
- **THEN** the route is not called and the connection is closed

### Requirement: The insecure_http_parser option restores lenient framing
`ServerOptions::insecure_http_parser(true)` SHALL turn off the checks of the requirements "Request head lines end
with CRLF", "Chunk size line has no whitespace", "Content-Length with Transfer-Encoding is rejected" and "Trailer
lines end with CRLF", and only those. The default is `false`. With it on, Corvis logs a warning at startup.

#### Scenario: Bare LF accepted with the option
- **WHEN** the server runs with `insecure_http_parser(true)` and a client sends `GET / HTTP/1.1\nHost: x\n\n`
  followed by a pipelined valid `GET`
- **THEN** both requests are answered normally

#### Scenario: Both length headers with the option
- **WHEN** the server runs with `insecure_http_parser(true)` and a client sends `Content-Length: 4` and
  `Transfer-Encoding: chunked` with the chunked body `2\r\nhi\r\n0\r\n\r\n` to a JSON route that echoes its body
- **THEN** the body is decoded as chunked, the request is answered, and the server closes the connection afterwards

#### Scenario: Errors that stay rejected with the option
- **WHEN** the server runs with `insecure_http_parser(true)` and a client sends a bare CR inside a header line, a
  chunk size over 64 bits, a bare LF in the chunked framing or an LF inside a chunk extension
- **THEN** each request is still answered with `400` or a closed connection

#### Scenario: Startup warning
- **WHEN** an application starts listening with `insecure_http_parser(true)`
- **THEN** Corvis logs one warning that names the option and says that HTTP/1 framing checks are off
