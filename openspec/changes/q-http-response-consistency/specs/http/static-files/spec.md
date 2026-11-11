# Spec Delta

## ADDED Requirements

### Requirement: Conditional requests follow the original freshness rule
Static files and responses sent with `res.send()` / `res.json()` SHALL decide between `200` and `304` with one freshness
rule, the one of the original `fresh` package, replayed from a recorded fixture: `Cache-Control` is matched as a
`no-cache` token, and `If-None-Match`, `If-Modified-Since` and their combination are evaluated as the fixture records.

#### Scenario: A token that only contains no-cache
- **WHEN** a client requests a static file with a matching `If-None-Match` and `Cache-Control: no-cache-x`
- **THEN** the answer is the one recorded from the original static middleware for the same headers

#### Scenario: The no-cache token
- **WHEN** a client requests a static file with a matching `If-None-Match` and `Cache-Control: max-age=0, no-cache`
- **THEN** the answer is `200` with the file

#### Scenario: Same rule for both paths
- **WHEN** the same conditional headers are sent to a static file and to a handler using `res.send()` whose response has
  the same `ETag` and `Last-Modified`
- **THEN** both are answered `304`, or both `200`
