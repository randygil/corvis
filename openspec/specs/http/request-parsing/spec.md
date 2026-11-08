# http/request-parsing Specification

## Purpose
Guarantees that the request parsers (JSON bodies, urlencoded bodies, query strings and multipart forms) handle any
input a hostile client can send without crashing the server and without using resources out of proportion to it.

## Requirements

### Requirement: Body parsing is total
For any request body, content type, charset and content encoding, the body parser SHALL produce either a parsed value
or an HTTP error response. It MUST NOT panic or abort the process.

#### Scenario: Arbitrary bytes as a JSON body
- **WHEN** a request declares `Content-Type: application/json` and carries arbitrary bytes, with or without a
  `Content-Encoding` such as `gzip`, `br` or an unknown value
- **THEN** the server answers with a parsed body or a 4xx error, and keeps serving other requests

#### Scenario: Unusual charset
- **WHEN** a JSON or urlencoded body declares a charset such as `utf-16`, `latin1` or `koi8-r`
- **THEN** the body parser produces a value or an HTTP error, and never fails internally

### Requirement: JSON parsing matches the reference grammar
JSON bodies SHALL be accepted when the reference JSON grammar accepts them, and values SHALL round-trip: integers,
strings, booleans, null, arrays and objects with non-numeric keys parse to the same value they were serialized from.

#### Scenario: Valid JSON is accepted
- **WHEN** a body is a document that a standard JSON parser accepts
- **THEN** the body parser accepts it too

#### Scenario: Round trip
- **WHEN** a JSON value without numeric object keys is serialized and sent as a body
- **THEN** the parsed body equals the original value

### Requirement: JSON nesting is bounded
JSON parsing SHALL reject documents nested deeper than the configured maximum depth with an error, and MUST NOT
exhaust the stack for any nesting depth.

#### Scenario: Deeply nested arrays
- **WHEN** a body opens and closes more nested arrays than the configured maximum depth (up to tens of thousands)
- **THEN** parsing fails with an error and the process keeps running

### Requirement: Query string parsing is total and proportional
Query strings and urlencoded bodies parsed with the extended (qs) parser SHALL never fail unless strict depth is
enabled, and the parsed value MUST stay proportional to the input size, whatever array indexes, brackets or
percent-encodings it contains.

#### Scenario: Far array index
- **WHEN** a query string contains a parameter such as `a[999999999]=x`
- **THEN** the parsed value has a size proportional to the query string, not to the index

#### Scenario: Strict depth
- **WHEN** strict depth is enabled and a parameter nests deeper than the configured depth
- **THEN** parsing fails with a depth error; without strict depth the same input parses successfully

### Requirement: Multipart parsing is total and bounded
Multipart parsing SHALL accept any body and boundary without crashing. Every reported field or file MUST correspond to
a boundary delimiter present in the body, and the stored field values and file contents MUST NOT exceed twice the body
size. Without `preservePath`, reported file names MUST NOT contain path separators.

#### Scenario: Malformed multipart body
- **WHEN** a multipart body mixes valid part headers, broken delimiters and random bytes
- **THEN** the parser reports only parts backed by delimiters and stores no more than twice the body size

#### Scenario: Empty boundary
- **WHEN** the content type is `multipart/form-data; boundary=""`
- **THEN** the boundary is accepted, as the original multipart parser does, and parsing stays total and bounded

#### Scenario: Path in a file name
- **WHEN** a part declares a file name such as `../../etc/passwd` or `C:\x\a.txt` and `preservePath` is off
- **THEN** the reported file name contains no `/` or `\`

### Requirement: Multipart field nesting is bounded
Nested multipart field names (for example `a[b][0]`) SHALL nest at most the configured depth, and arrays built from
field names MUST NOT grow beyond a fixed budget of holes, so the stored object stays proportional to the fields
received.

#### Scenario: Sparse index in a field name
- **WHEN** a form has a field named `x[4294967294]`
- **THEN** the stored object stays proportional to the input instead of allocating a huge array

### Requirement: Route paths are total
Joining controller and method paths and translating them to the router's syntax SHALL complete for any path strings
without crashing.

#### Scenario: Unusual route characters
- **WHEN** controller and method paths contain characters such as `:`, `*`, `{`, `}`, `(`, `)`, `?`, `+`, `~` or `\`
- **THEN** joining and translating them completes without a panic

### Requirement: Multipart field nesting limit is a client error
When a file interceptor has `limits.field_nesting_depth` set and a text field name nests deeper than that limit
(counted as the number of `[` in the name, as multer counts it), the request SHALL be answered with
`400 Bad Request` and the body `{ "message": "Field name nesting too deep - <field name>", "error": "Bad Request",
"statusCode": 400 }`, as current `@nestjs/platform-express` does. It MUST NOT be answered with a 5xx status.

#### Scenario: Field name deeper than the configured limit
- **WHEN** a route uses `NoFilesInterceptor` with `Limits::new().field_nesting_depth(2)` and receives a field named
  `a[b][c][d]`
- **THEN** the response is `400` with message `"Field name nesting too deep - a[b][c][d]"`

#### Scenario: Field name within the limit
- **WHEN** the same route receives a field named `a[b][c]`
- **THEN** the request reaches the handler and `req.body` is `{ "a": { "b": { "c": "<value>" } } }`

#### Scenario: No limit configured
- **WHEN** `field_nesting_depth` is not set and a field name nests 5000 levels
- **THEN** the request is not rejected; past the default depth of 32 the name is stored as a literal key

### Requirement: Chunk sizes accept leading zeros and reject overflow
For HTTP/1.1 requests with `Transfer-Encoding: chunked`, a chunk size written with leading zeros SHALL be accepted and
read as its hexadecimal value, as Node accepts it. A chunk size whose value does not fit in 64 bits MUST be rejected:
the request never reaches a handler with a body, and it is answered with `400` or the connection is closed.

#### Scenario: Leading zeros
- **WHEN** a request to a JSON route sends the chunk size `00000000000000000000000002` followed by a two-byte chunk
  and the last chunk
- **THEN** the handler receives the two bytes as the body

#### Scenario: Overflowing chunk size
- **WHEN** a request sends the chunk size `10000000000000002` (17 hexadecimal digits)
- **THEN** the response is `400` or the connection is closed, and no handler runs with a body

### Requirement: Chunked framing requires CRLF
The lines of the chunked framing (chunk size lines and the line ending after each chunk) MUST end with CRLF. A bare LF
in that framing SHALL make the request fail with `400` or a closed connection, never a parsed body.

#### Scenario: Bare LF after a chunk size
- **WHEN** a request sends `2\nhi\n0\n\n` as its chunked body
- **THEN** the response is `400` or the connection is closed, and no handler runs with a body

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
