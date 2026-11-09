# Spec Delta

## Purpose

How much response memory the built-in server holds for one connection whose client does not read (or reads very
little, or resets and reopens HTTP/2 streams), so that the memory of slow clients is bounded per connection
regardless of what they read, while clients that read keep working.

## ADDED Requirements

### Requirement: Pending response bytes option
`ServerOptions::max_pending_response_bytes(n)` SHALL set the most pending response bytes one connection may hold.
The default MUST be 8 MiB (8 388 608 bytes), and `None` MUST disable the limit: no request waits for room and no
response is reset for it.

#### Scenario: Default value
- **WHEN** an application builds `ServerOptions::new()` without calling `max_pending_response_bytes`
- **THEN** each connection is limited to 8 MiB of pending response bytes

#### Scenario: Disabled
- **WHEN** a server has `ServerOptions::new().h2c(true).max_pending_response_bytes(None)` and an HTTP/2 client with
  window 0 requests a 256 KiB in-memory route on 20 streams
- **THEN** all 20 handlers run and all 20 streams receive response headers; no stream is reset by the server

### Requirement: What counts as pending response bytes
The pending response bytes of a connection SHALL be the sum, over its response bodies that have neither ended nor
been dropped, of: the whole size of a body held in memory with a known size, counted until the body ends or is
dropped; otherwise, the part of a data frame the body produced that has not been handed to the protocol layer yet.

#### Scenario: In-memory body
- **WHEN** a handler returns a 1 MiB `String` and the client has read half of it
- **THEN** the response still counts 1 MiB until its body ends or is dropped

#### Scenario: Streamed body
- **WHEN** a handler returns a stream that yields 64 KiB chunks over HTTP/2
- **THEN** the response counts only the part of the current chunk not yet handed to the protocol layer

### Requirement: Compressed responses count their uncompressed size
When response compression wraps an in-memory body, the response SHALL count the uncompressed size of that body.

#### Scenario: Compressed text
- **WHEN** compression is enabled and a handler returns a 1 MiB in-memory text body that is compressed
- **THEN** the response counts 1 MiB towards the connection's pending bytes until its body ends or is dropped

### Requirement: Each handler reserves a share of the cap
Before a request's handler runs, the server SHALL reserve for it a share of the cap equal to the cap divided by the
most requests the connection can run at once: `http2_max_concurrent_streams` over HTTP/2 and 1 over HTTP/1. The
reservation SHALL be released when the handler returns.

#### Scenario: Share with the defaults
- **WHEN** an HTTP/2 connection uses the default options
- **THEN** each running handler reserves 83 886 bytes (8 MiB divided by 100)

### Requirement: Handlers wait for room in the connection's budget
A request's handler MUST start only when the connection's pending response bytes, plus the reservations of its
running handlers, plus the request's own share do not exceed the cap, or when the connection has no pending response
bytes and no running handler. Until then the request MUST NOT reach a handler.

#### Scenario: New streams wait while a stalled client holds the budget
- **WHEN** a server has `h2c(true).max_pending_response_bytes(1 MiB)` and an HTTP/2 client with window 0 holds four
  256 KiB responses, then opens another stream to the same route
- **THEN** the handler of the new stream does not run until the client lets pending bytes drop, and it runs once the
  client grants window and reads one response

#### Scenario: Pipelined HTTP/1 requests run one handler at a time
- **WHEN** an HTTP/1 client sends two pipelined requests to a 1 MiB in-memory route and does not read
- **THEN** the second handler does not run while the first response body has not ended

### Requirement: A reset while waiting cancels the request
A request whose stream the client resets while it waits for room SHALL never reach its handler and SHALL count as
cancelled for the rapid-reset rule.

#### Scenario: Waiting request reset by the client
- **WHEN** an HTTP/2 request waits for room and the client resets its stream
- **THEN** its handler never runs and the request counts as cancelled

### Requirement: Responses above the cap are reset
Over HTTP/2, when a handler returns a response whose pending bytes would take a connection that already holds
pending response bytes above the cap, the server MUST drop the response at once, releasing its memory and any
compression slot, and reset its stream with `ENHANCE_YOUR_CALM`. The request SHALL count as cancelled for the
rapid-reset rule.

#### Scenario: Burst of large responses to a client that does not read
- **WHEN** a server has `h2c(true).http2_max_concurrent_streams(20).max_pending_response_bytes(1 MiB)` and an HTTP/2
  client with window 0 opens 20 streams to a 256 KiB in-memory route
- **THEN** at most four streams receive response headers, the others are reset with `ENHANCE_YOUR_CALM`, and the
  connection never holds more than 1 MiB of pending response bytes

### Requirement: Resets for the cap are logged once per connection
The first reset of a connection for exceeding `max_pending_response_bytes` SHALL be logged as a warning naming the
option, and later ones on the same connection MUST NOT be logged.

#### Scenario: Many resets on one connection
- **WHEN** 16 responses of one connection are reset for exceeding the cap
- **THEN** exactly one warning is logged for that connection

### Requirement: Responses that fit are always sent
A response returned while the connection holds no pending bytes MUST be sent whatever its size, and a response whose
pending bytes fit in the share reserved for its handler MUST never be reset for the cap.

#### Scenario: One response larger than the cap
- **WHEN** a connection that holds no pending bytes requests an 8 MiB in-memory route on a server with a 1 MiB cap
- **THEN** the response is sent in full

#### Scenario: Many small responses at once
- **WHEN** an HTTP/2 client that reads normally sends 100 concurrent requests to a 64 KiB route on a server with the
  default options
- **THEN** all 100 are answered `200` with their full body and no stream is reset

### Requirement: Waiting for room is bounded
A request SHALL wait for room in the connection's budget at most `ServerOptions::max_pending_response_wait` (default
5 seconds; `None` waits without limit). When the time is up over HTTP/2 the server MUST reset the stream with
`REFUSED_STREAM` without running the handler, and the request SHALL NOT count as cancelled or completed for the
rapid-reset rule. Over HTTP/1 the server MUST close the connection. The first expiry on a connection SHALL be logged
as a warning naming the option, and later ones on the same connection MUST NOT be logged.

#### Scenario: A slow download does not hold the other streams for ever
- **WHEN** an HTTP/2 client holds an in-memory response of 1 MiB on a server with a 1 MiB cap and
  `max_pending_response_wait` of 800 ms, reads it at a trickle, and opens five other streams
- **THEN** the five streams are reset with `REFUSED_STREAM` and the connection stays open

#### Scenario: Refusals are not cancellations
- **WHEN** six streams that waited too long are refused on a connection with `http2_max_cancelled_streams(3)`
- **THEN** the connection is not closed and none of their handlers ran

#### Scenario: Unlimited wait
- **WHEN** a server has `max_pending_response_wait(None)` and a stalled client's held responses fill the budget
- **THEN** a new stream is neither answered nor reset after 1.5 seconds

### Requirement: HTTP/1 charges end when hyper flushes the socket
Over HTTP/1 the charge of a response body that hyper took into its write buffer SHALL be released when hyper flushes
the connection's socket (which it does only with its write buffer empty), whatever number of bytes it wrote, so that
bytes hyper drops (a body longer than its declared length) never leave the connection waiting for room.

#### Scenario: Truncated response
- **WHEN** a handler returns a stream longer than its declared `Content-Length` and the client sends another request
  on the same connection
- **THEN** the second request is answered

### Requirement: Slow and resetting clients hold bounded memory
With the limit on, the pending bytes of in-memory responses on one connection MUST NOT exceed the larger of the cap
and the largest single response, whatever the client reads; streamed bodies add at most the data frame each holds.

#### Scenario: One progressing stream next to stalled ones
- **WHEN** an HTTP/2 client with window 0 opens 100 streams to a 1 MiB in-memory route and grants 16 KiB of window
  to stream 1 every period, on a server with the default options
- **THEN** at most eight responses are kept, the rest are reset, and the connection is not closed by the cap while
  stream 1 progresses

#### Scenario: Trickled window updates
- **WHEN** an HTTP/2 client with window 0 opens 100 streams to a 1 MiB in-memory route and grants 16 KiB to every
  stream each period
- **THEN** at most eight responses are kept and new streams wait for room

#### Scenario: RST cycle
- **WHEN** an HTTP/2 client with window 0 repeatedly opens 100 streams to a 1 MiB route, resets them and opens 100
  new ones, never reading
- **THEN** each round keeps at most eight responses, and the streams it resets count as cancelled

### Requirement: HTTP/2 send buffers are small
The server SHALL limit what hyper buffers per HTTP/2 stream to 32 KiB, unless the throughput measurement of the
implementation requires 64 KiB.

#### Scenario: Large window, blocked socket
- **WHEN** an HTTP/2 client grants a 16 MiB window on 100 streams of a 1 MiB route and stops reading the socket
- **THEN** each stream has at most the send buffer plus one 16 KiB slice queued in h2

### Requirement: Ended HTTP/2 bodies do not keep their allocation alive
Over HTTP/2 the server SHALL hand the slices of a data frame that may still be buffered after its body ends (the last
send-buffer size plus 32 KiB of each frame it cuts: one slice handed when there was room and one frame held by the
protocol codec) in allocations of their own, so that the memory kept for a
stream after its response body ended is at most that amount, not the whole response.

#### Scenario: Last slices are copies
- **WHEN** `GuardedBody` hands a 1 MiB in-memory HTTP/2 body to hyper in 16 KiB slices with a 32 KiB send buffer
- **THEN** the slices of the last 64 KiB (send buffer, the slice handed when there was room, and the one frame the
  HTTP/2 codec keeps outside its buffer count) do not point into the original allocation, and the earlier slices do

#### Scenario: Window update trickle after the body ended
- **WHEN** an HTTP/2 client downloads all but the last bytes of a 1 MiB response and then grants one byte of window
  per keep-alive period
- **THEN** the original 1 MiB allocation is released once the body ends
