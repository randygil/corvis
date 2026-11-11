# Authentication benchmark

`bench/auth/run.sh` measures what one request costs on the authentication hot paths of Corvis: the JWT strategy,
`JwtService` and the session middleware. It is a Corvis-only benchmark; the cross-framework comparison is
`bench/run.sh` and whole applications are measured by `bench/port-vs-original/`.

- `bench/auth/` is a workspace member, `corvis-auth-bench` (`publish = false`, `corvis` with `jwt` and
  `auth-session`). Its binary serves one route per scenario on `127.0.0.1`, counts every allocation of the process with
  a global allocator, counts requests, and answers `GET /__bench/stats` with `{"requests", "allocations", "bytes"}`.
- `bench/auth/run.sh` builds it, starts it, fetches the tokens and seeds the session through setup routes, checks the
  status and body of every scenario, measures, and writes the results.

The numbers inform; nothing asserts them. The deterministic counters of the unit tests and the allocation bounds of
`crates/corvis/tests/auth_hot_path_heap.rs` are what a change is accepted on.

## Scenarios

| Scenario | Route | What it exercises |
| --- | --- | --- |
| `jwt-hs256` | `GET /jwt/hs256`, bearer HS256 | the JWT strategy with a static HMAC key |
| `jwt-provider-rs256` | `GET /jwt/provider`, bearer RS256 | the JWT strategy with a key provider that returns clones of one cached `JwtKey` |
| `service-verify-rs256` | `GET /service/verify`, bearer RS256 | a guard calling `JwtService::verify_or_unauthorized` with RS256 module keys |
| `service-sign-hs256` | `POST /service/sign` | `JwtService::sign` with an HMAC secret |
| `service-sign-rs256` | `POST /service/sign-rs256` | `JwtService::sign` with an RSA private key |
| `session-none` | `GET /session/none`, no cookie | the session middleware for a request that saves nothing |
| `session-read` | `GET /session/read`, cookie of a 16 KiB session | load, change detection and the commit of an unchanged session |
| `session-write` | `POST /session/write`, same cookie | one key changed: the update path |

The session cookie path is `/session`, so the JWT and service routes do not run the session middleware. The session
uses the memory store with `max_session_bytes` of 64 KiB.

For every scenario the script runs a warmup, reads the stats, runs `oha` closed loop with `CONNS` connections for
`DURATION` seconds, reads the stats again, then runs `oha` at a fixed `RATE` for the latency percentiles. Allocations and
bytes per request are the change of the server's counters divided by the change of its request counter, over the closed
loop run (the stats requests themselves are not counted as requests).

## Prerequisites

- Git Bash on Windows (the script stops the server with `taskkill`), or a POSIX shell elsewhere.
- Rust, Node.js and `curl`.
- The `oha` load generator: `cargo install oha --locked`.

## Run it

```bash
bash bench/auth/run.sh --label before
# ... change the code ...
bash bench/auth/run.sh --label after --compare before
```

Run it from any directory, with no other heavy process on the machine, and with the same parameters for the two runs
you compare. `--compare <label>` prints a before and after table against `results/<label>.json`.

Each run writes `bench/auth/results/<label>.json` (ignored by git) with the date, operating system, CPU, the `rustc` and
`oha` versions, the commit (with `+changes` when `crates/` or `bench/auth/` have uncommitted changes), the parameters and
one row per scenario, and prints a Markdown table to copy into a change's design. A scenario whose check fails stops the
run with its name, and no results file is written.

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `CONNS` | `50` | Concurrent connections. |
| `DURATION` | `10` | Seconds of each measured run (closed loop and fixed rate). |
| `WARMUP` | `3` | Seconds of warmup per scenario (not measured). |
| `RATE` | `1000` | Requests per second of the latency run. |
| `SCENARIOS` | all eight | Scenarios to run, separated by spaces. |
| `OHA_THREADS` | `4` | Load generator threads; empty means the `oha` default. |
| `SKIP_BUILD` | unset | `1` skips the build and uses the binary that already exists. |
| `PORT` | `3290` | Port of the server, on `127.0.0.1`. |

## Where the binary is built

`run.sh` builds with `cargo build --release -p corvis-auth-bench` from the repository root, without overriding the
target directory, so the binary is `<target>/release/corvis-auth-bench(.exe)`, where `<target>` is what Cargo reports
for the workspace (`CARGO_TARGET_DIR` when it is set, otherwise `<repo>/target`). A run never creates a target directory
of its own. With `SKIP_BUILD=1` and no binary, the script stops before starting the server and prints the path and the
build command.
