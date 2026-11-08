# Node probe

`probe.mjs` sends raw bytes over a fresh TCP connection per case and prints the status of every response received
on it. It extends the probe of `e0-decide-compatible-vs-strict-mode` (archived under
`openspec/changes/archive/2026-11-08-e0-decide-compatible-vs-strict-mode/node-probe/`) with trailer, pipelining and
length-header cases. Everything binds and connects to `127.0.0.1`.

    node probe.mjs                          # Node's http module (an echo server started by the script)
    node probe.mjs --target 3000 --path /x  # a server already listening on 127.0.0.1:3000
    node probe.mjs --json                   # the same, as JSON (source of the Rust test fixture)

Most cases are followed by a pipelined `GET` with `Connection: close`. Two statuses mean the server went on reading
the connection after the first request; one status means it stopped there.

- `output-node-24.13.0.txt`: Node v24.13.0 (llhttp 9.3.0), the oracle.
- `output-corvis-hyper-1.12.0.txt`: Corvis before this change, measured on the `cats-app` example (see the header of
  the file for what the statuses mean there).

Known difference that is not a parsing difference: for `pipelined-valid-then-bare-lf` Node answers only `400`. Node
parses the whole read before the first response is written and destroys the socket on the error, so the valid first
request gets no response. hyper answers requests one at a time, so after this change Corvis answers `200`, then
`400` (or closes). The Rust test checks that the second request is not processed, not the exact sequence.
