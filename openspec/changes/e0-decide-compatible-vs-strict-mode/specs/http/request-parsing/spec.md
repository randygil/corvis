# Spec Delta

## ADDED Requirements

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
