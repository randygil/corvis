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
