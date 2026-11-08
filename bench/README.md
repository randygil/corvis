# Throughput benchmark

`bench/run.sh` measures the request throughput and the memory use of a minimal Corvis application against an
equivalent Node.js application, on Express and on Fastify. Both applications serve the same endpoints with the
same validation and the same response bodies, and the script checks that with `curl` before it measures anything.

- `bench/corvis-app/` is the Corvis application (a workspace member, `publish = false`).
- `bench/node-app/` is the equivalent Node.js application; `run.sh` installs its dependencies and builds it.
- `bench/report.js` turns the raw load generator output into `bench/results.md`.

## Prerequisites

- Git Bash on Windows (the script uses `taskkill` and `/proc/<pid>/winpid`).
- Rust, Node.js and `curl`.
- The `oha` load generator: `cargo install oha --locked`.

## Run it

```bash
bash bench/run.sh
```

Run it from any directory, with no other heavy process on the machine. Every server listens on `127.0.0.1` only.

`bench/results.md` is overwritten by every run, so do not commit the result of a short trial run
(`git checkout bench/results.md` restores the published one). Raw data and server logs go to `bench/raw/` and
`bench/logs/`, both ignored by git.

## Environment variables

| Variable | Default | Meaning |
| --- | --- | --- |
| `CONNS` | `100` | Concurrent connections. |
| `DURATION` | `15` | Seconds measured per scenario. |
| `WARMUP` | `5` | Seconds of warmup per scenario (not measured). |
| `FRAMEWORKS` | `corvis corvis-1t node-express node-fastify` | Servers to measure. |
| `SCENARIOS` | `hello json param post` | Scenarios to run. |
| `OHA_THREADS` | `8` | Load generator threads; empty means the `oha` default (one per core). With 8 threads `oha` saturates the Windows loopback (about 100k req/s) and leaves cores to the server. |
| `SKIP_BUILD` | unset | `1` skips the builds and uses the binaries that already exist. |
| `PORT_BASE` | `3200` | Base port; each server uses `PORT_BASE` plus its index. |

## Where the Corvis binary is built

`run.sh` builds the Corvis application in release mode with `cargo build --release -p corvis-app` from the repository
root, without overriding the target directory. The binary is therefore in the shared Cargo target directory:

```
<target>/release/corvis-app.exe
```

`<target>` is what Cargo reports for the workspace (`cargo metadata`): `CARGO_TARGET_DIR` when it is set, otherwise
`<repo>/target`. A run never creates `bench/target`; if one is left from an older version of the script, `run.sh`
prints a notice and leaves it in place, and you can delete it with `rm -rf bench/target`.

`SKIP_BUILD=1` runs whatever binary is in `<target>/release/`. If it is missing, the script stops before starting any
server and prints the path and the build command. For published numbers, run without `SKIP_BUILD` so the binary is
built with the application's own feature set (a `cargo build --release --workspace` unifies the features of all
members and overwrites the same file).
