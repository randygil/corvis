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
