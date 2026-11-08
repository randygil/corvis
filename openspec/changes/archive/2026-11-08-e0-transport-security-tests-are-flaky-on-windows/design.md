# Design

## Context

Hyper answers an oversized head (`431`), a malformed chunked body (`400`) or a handler that rejects a request without
reading its body, writes the response, shuts down the write half and drops the connection. In `serve`
(`crates/corvis/src/server.rs`) the socket is wrapped in `TimedIo`, and dropping it closes the `TcpStream`. If the
kernel receive queue still holds request bytes at that moment (or more arrive afterwards), Windows closes with a reset
(RST) instead of a FIN. A peer that receives RST discards any response data it has not read yet.

### Evidence

Running the compiled test binary of `transport_security` 40 times on Windows with the client logging what it reads:
4 failures, all `oversized_head_is_rejected`, each with
`read error kind=ConnectionAborted os=Some(10053) after 0 bytes`. Nothing of the response reached the client, which
confirms the reset hypothesis (the server had written and flushed it). A new test that keeps sending after the head
fails on the unfixed code on every run (empty response), so the race is reproduced deterministically.

## Goals / Non-Goals

**Goals:** the response written before an early close always reaches a peer that reads it; the extra work is bounded in
bytes and time; no new options.

**Non-Goals:** changing which requests are rejected; HTTP/2 (streams are reset by the protocol, not by the socket).

## Decisions

### 1. Close gracefully in `TimedIo::drop`, not in the tests

The response can be lost on the server side, so the fix belongs there (issue task 2, first branch). `TimedIo` becomes
non-generic and owns `Option<TcpStream>`; `Drop` hands the socket to the connection task (decision 2b), which runs
`linger_close`: shutdown of the write half (FIN, a no-op if hyper already did it), then reads and discards until EOF, a read error,
1 MiB (`LINGER_MAX_BYTES`) or 2 s (`LINGER_MAX_TIME`) in total, whichever comes first. Doing it when the connection task ends covers every
path (hyper error responses, handler early responses, normal close) without touching hyper. Hyper consumes the IO and returns it only on upgrade, so `TimedIo` keeps the socket for the task instead.

### 2. Bounded drain, and no drain when there is nothing to protect

The drain is the only new resource use. It is capped at 1 MiB and 2 s per connection (read buffer of 4 KiB), so an
endless sender gets a reset after at most 2 s, and a silent peer holds the task for at most 2 s. The permit of
`max_connections` stays with the socket until it is closed, so draining sockets still count against the limit.

The drain is skipped when the server closes the connection to punish the peer or when there is no response to protect:
the headers timeout and keep-alive timeout (the `TimedIo` one and hyper's own `header_read_timeout`), a refused
HTTP/2 preface, an upgraded connection, and a socket destroyed by an upgrade listener (`SocketDestroyed`). Waiting for a
peer that is already misbehaving only prolongs the slowloris the server just stopped. The `408` of `requestTimeout` is
a real response and does get the drain.

### 2b. Where the decision is taken (option (a), not deadline alignment)

hyper's `header_read_timeout` counts from the end of the previous response on a keep-alive connection, while the
`TimedIo` deadline counts from the first byte of the next request, so hyper's fires first and `TimedIo` never sees it.
Two ways to cover it were considered:

- (b) align the deadlines so `TimedIo` always fires first. Rejected: the two clocks start at different events by
  design, aligning them means changing when timeouts fire (a behavior change of the timeouts) and still leaves the
  decision spread over two components.
- (a) chosen: `TimedIo::drop` only leaves the socket and the permit in a slot of `ConnState`; the connection task
  (`finish_connection`) takes them after `conn.await` and drains unless `ConnState.no_linger` (set by `TimedIo`'s
  timeouts and refused h2c), `destroyed` (set when the service returns `SocketDestroyed`) or `upgraded` is set, or the
  connection error is a hyper timeout (`hyper::Error::is_timeout`, looked up through the error source chain). One
  place decides, with every signal available, and no timeout changes. If the task is cancelled the slot is dropped with
  the state and the socket closes without a drain.

### 3. Constants, not options

1 MiB covers the head limit (32 KiB default) plus pipelined data, and 2 s is far below the 5 s keep-alive default.
Options would be API surface without a demonstrated need.

## Risks / Trade-offs

- A malicious client can keep a closing socket for up to 2 s and make the server read up to 1 MiB of garbage per
  connection. This is bounded, counted by `max_connections`, and comparable to what Node does (`socket.end` plus
  waiting for the peer).
- If the Tokio runtime is shutting down, `Handle::try_current` can fail and the socket is dropped at once (a reset is
  then acceptable).

- A client that sends `Connection: close` and then does not close its side after reading the response now keeps the
  permit for up to 2 s (the drain), where before the socket was released at once. Clients that close, which is the
  normal case, end the drain immediately.

## Migration Plan

None: behavior only changes for connections closed while input was unread.

## Open questions

- Whether the drain limits should become `ServerOptions` fields if a user needs them.
