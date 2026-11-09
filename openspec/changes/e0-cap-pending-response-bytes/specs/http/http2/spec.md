# Spec Delta

## MODIFIED Requirements

### Requirement: Concurrent streams per connection are capped
`ServerOptions::http2_max_concurrent_streams(n)` SHALL set the number of streams a client may have open at once on
one HTTP/2 connection (`SETTINGS_MAX_CONCURRENT_STREAMS`), with a default of 100 and a minimum of 1. The server MUST
never run more handlers at once for one connection than this number, even when the client resets streams and opens
new ones faster than their handlers stop (rapid reset). It MUST close a connection whose cancelled requests exceed
500 and exceed a quarter of the requests it completed, over the last 10 seconds (current window plus the previous one
weighted), or exceed `ServerOptions::http2_max_cancelled_streams` (default 2000, minimum 1) whatever it completed,
after a graceful GOAWAY and a grace period of 1 second. A request SHALL count as cancelled when its stream is reset by
the client before its response body ended (before the handler returned, while the request body was being read, or
after the handler returned while the response body was still being sent) or when the server drops its response for
exceeding `max_pending_response_bytes`; it SHALL count as completed only when its response body ended. From the
verdict on, new streams MUST be reset without reaching a handler. A connection that cancels at most a quarter as many
requests as it completes, below the ceiling, MUST NOT be closed for it. A request aborted by the client SHALL NOT be
logged.

#### Scenario: Cap of two
- **WHEN** a server has `ServerOptions::new().h2c(true).http2_max_concurrent_streams(2)` and an HTTP/2 client starts
  three requests at once to a handler that waits until released
- **THEN** at most two handlers run at the same time, and all three requests are answered once the handlers are
  released

#### Scenario: Default advertised
- **WHEN** an HTTP/2 client connects to a server with `ServerOptions::new().h2c(true)`
- **THEN** the server's SETTINGS frame advertises `SETTINGS_MAX_CONCURRENT_STREAMS` = 100

#### Scenario: Streams reset after their handler finished
- **WHEN** an HTTP/2 client with window 0 opens streams to a small route, waits for each response's HEADERS frame
  and resets the stream before any DATA, 600 times within 10 seconds
- **THEN** the server sends a GOAWAY and closes the connection, because those requests count as cancelled and none
  completed

#### Scenario: Downloads that finish still earn credit
- **WHEN** an HTTP/2 client completes 3000 small requests and resets 600 streams after their HEADERS within 10 seconds
- **THEN** the connection is not closed
