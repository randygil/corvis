# Spec Delta

## ADDED Requirements

### Requirement: A panicking indicator is reported down
`HealthCheckService::check` SHALL isolate every indicator from panics: a panic raised while the indicator's check is
started or awaited MUST be caught for that indicator only and reported as `down` with
`"message": "health indicator panicked"`, in the same `{ "status", "info", "error", "details" }` body and with the
same HTTP `503 Service Unavailable` as any other `down` indicator. The other indicators MUST still be run and reported
with their own results.

#### Scenario: One indicator panics, the other is up
- **WHEN** `check` runs `HealthIndicator::new("db", || async { Ok(json!({})) })` and
  `HealthIndicator::new("cache", || async { panic!("connection pool poisoned") })`
- **THEN** `check` returns a `503` whose body has `"status": "error"`, `info.db.status` `"up"`,
  `error.cache` `{ "status": "down", "message": "health indicator panicked" }`, and both keys in `details`

#### Scenario: The panic happens when the check is started
- **WHEN** an indicator's closure panics before returning its future (for example
  `HealthIndicator::new("cfg", || -> futures::future::Ready<Result<Value, Value>> { panic!("bad config") })`)
- **THEN** `cfg` is reported `down` with `"message": "health indicator panicked"` and the other indicators are
  reported normally

#### Scenario: Panic details stay out of the response
- **WHEN** an indicator panics with the message `"postgres://user:secret@db/app"`
- **THEN** the response body does not contain `"secret"`, and an error log line contains the indicator key and the
  panic message

#### Scenario: Through an HTTP route
- **WHEN** a controller route returns `self.health.check(..)` with one panicking indicator and the route is requested
- **THEN** the response status is `503`, not `500`, and the body is the health body

### Requirement: The panic payload stays out of the health response
The payload of a caught indicator panic MUST NOT appear in the health response body. It SHALL be logged at error level
together with the indicator key.

#### Scenario: A panic message with a secret
- **WHEN** an indicator panics with "connect failed: postgres://user:secret@db"
- **THEN** the response body contains "health indicator panicked" and not "secret", and the error log contains the
  indicator key and the panic message
