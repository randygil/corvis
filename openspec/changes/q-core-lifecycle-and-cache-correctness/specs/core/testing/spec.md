# Spec Delta

## ADDED Requirements

### Requirement: The test client refuses requests it cannot build
A `TestRequest` given an invalid header name or value, a path that is not a valid URI, or a `json()` value that fails to
serialize SHALL NOT send a different request. `try_send()` MUST return a `TestRequestError` describing the first invalid
input without dispatching anything, and `send()` MUST panic with that description.

#### Scenario: Invalid header value
- **WHEN** a test calls `request(&app).get("/").header("x-trace", "a\nb").try_send()`
- **THEN** it returns a `TestRequestError` that names the header and no request reaches the application

#### Scenario: Invalid path with send
- **WHEN** a test calls `request(&app).get("/cats/ id").send()`
- **THEN** it panics with a message that names the path

#### Scenario: A body that cannot be serialized
- **WHEN** a test calls `json()` with a map whose keys are tuples (which JSON cannot represent) and then `try_send()`
- **THEN** it returns a `TestRequestError` and no request with an empty body is sent
