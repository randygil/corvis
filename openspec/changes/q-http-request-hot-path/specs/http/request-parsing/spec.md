# Spec Delta

## ADDED Requirements

### Requirement: The JSON result does not depend on the parsing path
A JSON body SHALL produce the same parsed value or the same error, with the same message and the same UTF-16 position,
whether it is parsed over its UTF-8 bytes or over its UTF-16 decoding. Any error MUST be reported as the UTF-16 parse
reports it, and every case of the JSON fixture recorded from V8 MUST still match.

#### Scenario: Valid body with non-ASCII strings
- **WHEN** a UTF-8 JSON body with accented, CJK and emoji strings is parsed
- **THEN** the parsed value equals the one the UTF-16 parse produces

#### Scenario: Error after a non-ASCII character
- **WHEN** a UTF-8 JSON body has a syntax error after an emoji
- **THEN** the error message and its position are the ones V8 reports, counted in UTF-16 code units

#### Scenario: Invalid UTF-8
- **WHEN** a JSON body declared as UTF-8 contains bytes that are not valid UTF-8
- **THEN** the result is the one today's lossy UTF-16 decoding produces

### Requirement: A request shares its parsed parts with its clones
A clone of a `Request` SHALL share its parsed body, parsed query and headers with the original until one of them changes
them through `set_body` or `headers_mut`; a change made on one MUST NOT be visible on the other. Dispatching a request
with CORS and middleware MUST NOT copy its parsed body.

#### Scenario: Clone of a large request
- **WHEN** a request with a 16 KiB JSON body is cloned
- **THEN** `body()`, `query()` and `headers()` of the clone point to the same data as the original's

#### Scenario: Changing a clone
- **WHEN** a middleware calls `headers_mut()` on a clone and inserts a header
- **THEN** the original request does not have that header

#### Scenario: CORS and middleware
- **WHEN** a request with a 16 KiB JSON body is dispatched with CORS enabled and one global middleware
- **THEN** no copy of the parsed body is allocated during the dispatch
