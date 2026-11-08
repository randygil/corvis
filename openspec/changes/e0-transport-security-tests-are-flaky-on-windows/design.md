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
non-generic and owns `Option<TcpStream>`; `Drop` takes the socket and, inside a Tokio runtime, spawns `linger_close`:
shutdown of the write half (FIN, a no-op if hyper already did it), then reads and discards until EOF, a read error,
1 MiB (`LINGER_MAX_BYTES`) or 2 s (`LINGER_MAX_TIME`) in total, whichever comes first. Doing it in `Drop` covers every
path (hyper error responses, handler early responses, normal close) without touching hyper. Alternative: wrap
`serve_connection` and drain after it returns; rejected because hyper consumes the IO and returns it only on upgrade.

### 2. Bounded drain, and no drain when the server is punishing the peer

The drain is the only new resource use. It is capped at 1 MiB and 2 s per connection, so an endless sender gets a reset
after at most 2 s, and a silent peer holds the task for at most 2 s. The permit of `max_connections` moves into the
closing task, so lingering sockets still count against the limit. Connections the server closes because of a timeout
(`headersTimeout`, `keepAliveTimeout`) or a refused HTTP/2 preface set `linger = false` and close at once: waiting for
a peer that is already misbehaving only prolongs the slowloris it just stopped. Upgraded connections (WebSocket) also
skip the drain.

### 3. Constants, not options

1 MiB covers the head limit (32 KiB default) plus pipelined data, and 2 s is far below the 5 s keep-alive default.
Options would be API surface without a demonstrated need.

## Risks / Trade-offs

- A malicious client can keep a closing socket for up to 2 s and make the server read up to 1 MiB of garbage per
  connection. This is bounded, counted by `max_connections`, and comparable to what Node does (`socket.end` plus
  waiting for the peer).
- If the Tokio runtime is shutting down, `Handle::try_current` can fail and the socket is dropped at once (a reset is
  then acceptable).

## Migration Plan

None: behavior only changes for connections closed while input was unread.

## Open questions

- Whether the drain limits should become `ServerOptions` fields if a user needs them.
