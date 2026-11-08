# Spec Delta

## ADDED Requirements

### Requirement: HTTP ping check makes an HTTP request
`HttpHealthIndicator::ping_check(key, url)` SHALL send an HTTP `GET` request to `url` and report the indicator `up`
only when the final response has a 2xx status. A non-2xx response, a connection error or an invalid URL MUST report
the indicator `down` with a `message`; when a response was received, the details MUST also contain `statusCode` and
`statusText`. The `down` details SHALL have the same keys as `@nestjs/terminus` `HttpHealthIndicator.pingCheck` for the
same situation.

#### Scenario: Server answers 200
- **WHEN** `ping_check("api", "http://127.0.0.1:<port>/")` runs against a server that answers `200`
- **THEN** the indicator is reported as `{ "status": "up" }`

#### Scenario: Server accepts the connection but answers 500
- **WHEN** the target server accepts the TCP connection and answers `500 Internal Server Error`
- **THEN** the indicator is reported `down` with `statusCode` `500`, `statusText` `"Internal Server Error"` and a
  `message`, and `HealthCheckService::check` returns a `503`

#### Scenario: Nothing listens on the port
- **WHEN** no process listens on the URL's port
- **THEN** the indicator is reported `down` with a `message` and without `statusCode`

#### Scenario: Address without a scheme
- **WHEN** `ping_check` receives `"127.0.0.1:6379"` (no `http://` or `https://`)
- **THEN** the indicator is reported `down` with a `message` that names `tcp_check` as the indicator for plain TCP
  addresses

#### Scenario: HTTPS without TLS support compiled in
- **WHEN** `ping_check` receives an `https://` URL and the crate was built without its TLS feature
- **THEN** the indicator is reported `down` with a `message` naming the feature to enable, and no plain-text request
  is sent

### Requirement: TCP check is explicit
`HttpHealthIndicator::tcp_check(key, addr)` SHALL report the indicator `up` when a TCP connection to `addr`
(`host:port`) can be opened, and `down` with a `message` otherwise. It MUST NOT be described as an HTTP check.

#### Scenario: Port open
- **WHEN** `tcp_check("redis", "127.0.0.1:<port>")` runs and a listener accepts on that port
- **THEN** the indicator is reported `up`

#### Scenario: Port closed
- **WHEN** nothing listens on the port
- **THEN** the indicator is reported `down` with a `message`

### Requirement: RSS check measures on every supported platform
`MemoryHealthIndicator::check_rss(key, threshold_bytes)` SHALL measure the resident set size of the current process on
Linux, Windows and macOS, and report `down` with `"message": "Used rss exceeded the set threshold"` when it exceeds
`threshold_bytes`. On Linux the measurement MUST use the system page size, not a fixed 4096. When the platform does not
allow the measurement, the indicator MUST be reported `down` with a message saying that RSS could not be measured; it
MUST NOT be reported `up` without a measurement.

#### Scenario: Under the threshold
- **WHEN** `check_rss("memory_rss", u64::MAX)` runs on Linux, Windows or macOS
- **THEN** the indicator is reported `up`

#### Scenario: Over the threshold
- **WHEN** `check_rss("memory_rss", 1)` runs on Linux, Windows or macOS
- **THEN** the indicator is reported `down` with `"message": "Used rss exceeded the set threshold"`

#### Scenario: Measurement not available
- **WHEN** the RSS of the process cannot be read on the current platform
- **THEN** the indicator is reported `down` with a message that RSS could not be measured

### Requirement: Indicators have a time limit
`HealthCheckService::check` SHALL run every indicator with a time limit: 5 seconds unless the indicator was built with
`HealthIndicator::timeout(duration)`. An indicator that does not finish within its limit MUST be reported `down` with
`"message": "timeout of <ms>ms exceeded"`, and the other indicators MUST still be reported with their own results.
Indicators SHALL keep running concurrently, so the response time is bounded by the largest limit, not their sum.

#### Scenario: One indicator hangs
- **WHEN** `check` runs an indicator that never completes, with `.timeout(Duration::from_millis(100))`, together with an
  indicator that returns `Ok`
- **THEN** within about 100 ms `check` returns a `503` whose `error` contains the hung indicator with
  `"message": "timeout of 100ms exceeded"` and whose `info` contains the other indicator as `up`

#### Scenario: Default limit
- **WHEN** an indicator built without `.timeout(...)` never completes
- **THEN** it is reported `down` with `"message": "timeout of 5000ms exceeded"` after about 5 seconds

### Requirement: Response format
The body returned by `HealthCheckService::check` SHALL be `{ "status", "info", "error", "details" }`, with `status`
`"ok"` and HTTP `200` when every indicator is up, and `status` `"error"` with HTTP `503 Service Unavailable` when any
indicator is down; each indicator entry MUST contain `"status": "up"` or `"status": "down"` merged with the details
the indicator returned.

#### Scenario: Mixed results
- **WHEN** one indicator returns `Ok({})` and another returns `Err({ "message": "down" })`
- **THEN** the response is `503`, `info` holds the first as `up`, `error` holds the second as `down` with its message,
  and `details` holds both
