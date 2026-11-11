# Socket.IO benchmark

`bench/socket-io/run.sh` measures a Corvis Socket.IO server under load: how many messages per second it delivers, with
what latency, and how many allocations and bytes the server allocates per message. It exists so that a change with a
performance claim can show the same figures before and after; it does not compare Corvis with the Node server.

- `corvis-sio-bench server --port <p>` starts an `IoServer` with default options on `127.0.0.1:<p>`, with a counting
  global allocator and the route `GET /stats` (`GET /stats/reset` resets the counters and the peak resident memory).
- `corvis-sio-bench load ...` opens raw Engine.IO / Socket.IO clients over WebSocket or long-polling, runs one scenario
  and prints one JSON line with the figures of the measured window. Each client sends a 4 KiB `cookie` header and binds
  one of 200 loopback addresses (`127.0.1.1` to `127.0.1.200`), so the per-address caps of the default options are not
  what is measured.
- `corvis-sio-bench report <dir>` turns the JSON lines into the table of `results.md`.

Scenarios:

| Scenario | What it measures |
| --- | --- |
| `emit` | Each client sends its messages one at a time with an acknowledgement; the handler answers. Latency: send to answer. |
| `server-emit` | The server emits the messages to each socket from a task. Latency: the server's time stamp to delivery. |
| `broadcast` | The server emits the messages to the whole namespace. Latency: the server's time stamp to delivery. |

## Prerequisites

- Rust and `curl`; on Windows, Git Bash (the script stops the server with `taskkill`).
- No other heavy process on the machine while it runs.

## Run it

```bash
bash bench/socket-io/run.sh
LARGE_SIZE=16384 bash bench/socket-io/run.sh   # also server-emit and broadcast with a 16 KiB payload
```

`bench/socket-io/results.md` is overwritten by every run (`git checkout bench/socket-io/results.md` restores the
published one). The raw JSON lines and the server logs go to `bench/socket-io/raw/`, which git ignores. The benchmark is
not a test: it never runs in CI or in `cargo test`.

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `WS_CONNECTIONS` | `1000` | Connections of the WebSocket runs. |
| `POLLING_CONNECTIONS` | `200` | Connections of the polling runs. |
| `MESSAGES` | `100` | Messages per connection. |
| `SIZE` | `64` | Payload bytes of every message. |
| `LARGE_SIZE` | unset | When set, `server-emit` and `broadcast` run again with this payload size. |
| `RUNS` | `3` | Runs per measurement; the table shows the median with the minimum and maximum. |
| `SCENARIOS` | `emit server-emit broadcast` | Scenarios to run. |
| `TRANSPORTS` | `websocket polling` | Transports to run. |
| `PORT` | `3300` | Port of the server, on `127.0.0.1`. |
| `SKIP_BUILD` | unset | `1` skips the build and uses the binary that exists. |

## Where the binary is built

`run.sh` builds with `cargo build --release -p corvis-sio-bench` from the repository root, without overriding the
target directory, so the binary is in the workspace's target directory: `<target>/release/corvis-sio-bench` (`.exe` on
Windows), where `<target>` is what `cargo metadata` reports (`CARGO_TARGET_DIR` when it is set, otherwise
`<repo>/target`). With `SKIP_BUILD=1` and no binary there, the script stops before starting a server and prints the path
it looked for and the build command.
