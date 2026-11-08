# Design

## Context

### The existing benchmark (`bench/`)

- `bench/run.sh` (Git Bash, Windows only) builds `bench/corvis-app` with `CARGO_TARGET_DIR=bench/target` and
  `bench/nest-app` with `npm run build`, then for each of four targets (`corvis`, `corvis-1t` with
  `TOKIO_WORKER_THREADS=1`, Node on Express, Node on Fastify) starts the server on `127.0.0.1`, checks every endpoint
  with curl (`verify_endpoints`), runs oha with a warmup and a measured window (`run_oha`: `--no-tui -w -c $CONNS -z`,
  `--worker-threads 8` by default), and samples memory every 2 s with `tasklist` (`rss_kb`, Windows working set).
- Windows specifics: PIDs come from `/proc/<pid>/winpid` (Cygwin/MSYS), processes are stopped with `taskkill`.
- `bench/report.js` turns the oha JSON (`summary.requestsPerSec`, `latencyPercentiles.p50/p99`,
  `statusCodeDistribution`, `errorDistribution`) and `meta.json` into `bench/results.md`, written in Spanish.
- `bench/corvis-app` is a workspace member (`Cargo.toml:12`); `bench/target`, `bench/raw`, `bench/logs` and the Node
  build output are git-ignored (`.gitignore`).
- Published results (2026-11-07, i9-14900KF, Windows 11, Node 24.13.0, rustc 1.99, oha 1.16.0, 100 connections, 15 s):
  quoted in `docs/faq/corvis-vs-fastify-express-hono.md:29-57`; `docs/http/performance.md:88-92` links there.
- Gaps for this issue: synthetic endpoints only, no database, p99 measured at saturation (closed loop, so it mostly
  shows queueing), memory sampled every 2 s (peaks can be missed), one run per scenario, Windows only.

### The port apps (`C:/Users/Usuario/Devel/corvis-tests`)

- **facten** (`corvis/facten-api`, original `nest/facten-api`): ERP API (invoicing, inventory, accounting), 74 routes,
  JWT bearer auth with a global guard and a roles guard, zod DTOs, `Decimal` money, PostgreSQL through corvis-orm /
  the original's ORM 6.19.3 (`nest/facten-api/package.json`). No external services on the measured paths (the fiscal
  printer bridge and the digital printer integration are opt-in through env vars that `scripts/apidiff.sh` unsets).
  Its parity tooling already: resets `facten_nest` / `facten_corvis` from the `facten_tpl` template
  (`scripts/reset-db.sh`, Docker container `corvis-pg` on `127.0.0.1:55432`), starts the original with a preload that
  forces `127.0.0.1` (`scripts/run-nest.sh`, `scripts/loopback.cjs`) and the port from `target/release/facten-api`
  (`scripts/apidiff.sh:49-63`), and waits on `GET /health` (no auth, outside the `/api/v1` prefix,
  `src/health/health.controller.rs`).
  The template has the admin user, the company, a default customer and the tax rates, but no products and no BCV rate
  (`apidiff/00-setup.mjs:1-3`).
- **consi** (payments gateway): webhooks to a sink on port 4191 and gateway calls; deliveries are asynchronous.
- **houseplan**: Telegram bot, Binance and BDV sync, scheduled reminders, LLM calls and local embeddings with
  onnxruntime (`corvis/houseplan-api/README.md`).
- Runtime: the port's `#[corvis::main]` builds `tokio::runtime::Builder::new_multi_thread()` without a worker count
  (`crates/corvis-macros/src/lib.rs:143-147`), so `TOKIO_WORKER_THREADS` sets the workers. The original runs as one
  Node process (`node dist/main.js`; its Dockerfile uses `node:22`).
- Database pools: corvis-orm defaults to 10 Postgres connections (`crates/corvis-orm/src/client.rs:235`) and does not
  parse `connection_limit` from the URL; the original's ORM 6 defaults to `num_physical_cpus * 2 + 1` and reads
  `connection_limit` from the URL.
- apidiff (`corvis-tests/tools/apidiff/apidiff.mjs`) sends the same requests to two servers and compares status,
  relevant headers and JSON bodies, mapping ids, timestamps and JWTs.

## Goals / Non-Goals

**Goals:**
- Numbers for one real port against its original, on the same machine, database, data and requests.
- p50/p99 that mean latency under a sustainable load, not queueing at saturation.
- Memory peaks that cannot be missed by sampling.
- One command to rerun, and docs tables that are regenerated from committed data in both languages.

**Non-Goals:**
- Replacing the synthetic `bench/run.sh` (it stays: it isolates framework overhead).
- CI integration, Linux numbers, consi and houseplan (see proposal, Out of scope).
- Tuning either side for the benchmark: both run as they ship, except the database pool (decision 6).

## Decisions

1. **facten is the port.** It exercises what migrations are about (guards, validation, ORM, decimals, large JSON) with
   no external service on the measured paths, and its parity scripts already reset databases and start both sides on
   loopback. Alternatives: consi (asynchronous webhook deliveries and gateway calls add noise and need the sink
   running), houseplan (background jobs, bot polling and the embeddings model change memory and CPU independently of
   the load).

2. **Scenarios**, all under the `admin` role with a bearer token obtained in setup:

   | Id | Request | What it exercises |
   |---|---|---|
   | `health` | `GET /health` | framework path with global guards/interceptors, no auth, no database |
   | `products-list` | `GET /api/v1/products` (100 seeded products, no pagination in the route) | JWT + roles guard, ORM `findMany`, large JSON serialization |
   | `product-get` | `GET /api/v1/products/:id` | `ParseUUIDPipe`, `findUnique`, small JSON |
   | `invoice-preview` | `POST /api/v1/invoices/preview` with 5 items and one payment | zod body validation, several reads, `Decimal` fiscal computation, nothing persisted |
   | `login` (opt-in) | `POST /api/v1/auth/login` | password hashing cost (bcryptjs in the original); off by default because it measures the hash, not the framework |

   Writes are excluded (proposal, Out of scope). The setup (`scenario.mjs`, an apidiff scenario) logs in, creates a BCV
   rate for today and 100 products, and sends each measured request once, so apidiff checks equality and the ids for
   the oha command lines come from the same run.

3. **Load tool: oha.** Already used by `bench/run.sh`, installed on the machine (1.16.0), written in Rust, produces
   JSON with percentiles, supports headers and body files (`-H`, `-D`), fixed rates (`-q`) and coordinated-omission
   correction (`--latency-correction`). Alternatives: k6 (scriptable, but a new tool to install and its JS VM costs CPU
   on the same machine), autocannon (Node: the load generator would compete with the Node side in a similar way to
   the server), wrk2 (no native Windows build).

4. **Two phases per scenario.**
   - Throughput: closed loop, `-c 100` connections (same as `bench/run.sh`) for 15 s after a 5 s warmup, three runs;
     report the median requests per second.
   - Latency: open loop at a fixed rate `R = 0.5 × median throughput of the original` for that scenario, same `R` for
     both sides, `-q R --latency-correction`, 30 s, three runs; report the median of each percentile (p50, p90, p99,
     max). At half the original's capacity both sides are below saturation, so the percentiles compare service time,
     not queue length. The throughput phase of the original runs first so `R` is known.
   Corvis is measured twice, with all cores (default runtime) and with `TOKIO_WORKER_THREADS=1` (per-core comparison
   with one Node process), as in `bench/run.sh`.

5. **Memory from the operating system, per scenario.** Each scenario runs on a freshly started server, so the
   process-lifetime peak counter is the scenario's peak: `PeakWorkingSet64` on Windows (read with
   `Get-Process -Id <pid>`), `VmHWM` from `/proc/<pid>/status` on Linux. Also reported: resident memory at idle (after
   startup and the setup requests, before the warmup) and after the load, plus a 500 ms sample series kept in the raw
   data. Only the server process is measured (Postgres is shared and reported as an environment note).
   Alternative: one long-lived process per side with sampling only (current `bench/run.sh`), rejected because peaks
   between samples are lost and the order of scenarios would change the numbers.

6. **Same database pool size on both sides for the headline numbers.** The original's URL gets `connection_limit=10`,
   matching corvis-orm's default of 10, so the comparison is not a pool-size comparison. `POOL=default` runs both with
   their shipped defaults and is reported as a secondary table. Both values are recorded in `summary.json`.
   Alternative: shipped defaults only (closer to "what you get", but on a 24-core machine the original would open 49
   connections against 10). See open question 2.

7. **Orchestrator in Node, no dependencies, separate from `bench/run.sh`.** `bench/port-vs-original/run.mjs` spawns
   the servers itself (real PIDs on Windows and Linux, no `/proc/<pid>/winpid`), resets the databases with
   `docker exec corvis-pg psql`, runs apidiff and oha as child processes and stops every process it started on exit.
   Paths come from options with defaults: `--tests-dir` (`../corvis-tests` next to the repository), `--app facten`.
   It never builds: a missing port binary or `dist/main.js` stops the run with the build commands
   (port: `CARGO_TARGET_DIR="$PWD/target" cargo build --release`, as the corvis-parity skill does; original:
   `npm run build`). Alternative: extend `run.sh` (Bash plus Windows-only process handling; the app-specific setup is
   easier in JavaScript, and apidiff scenarios are JavaScript already).

8. **What gets committed.** `bench/port-vs-original/results/<YYYY-MM-DD>-<label>/summary.json` and `results.md`
   (English, generated), so every published number has its data next to it. Raw oha JSON and logs go to
   `bench/port-vs-original/raw/` (git-ignored, large). `summary.json` records: date, OS, CPU model and logical cores,
   Node, rustc, oha version and options, Postgres version, Corvis commit and dirty flag (from the path dependency in the
   port's `Cargo.toml`), port binary timestamp, the original's package versions, pool sizes, scenario definitions,
   `R` per scenario, and every run's numbers.

9. **Publishing in English and Spanish.** A new page `docs/faq/port-vs-original-benchmark.md` (and
   `docs/es/faq/port-vs-original-benchmark.md`) holds the method, the environment, the tables and how to read them.
   `report.mjs --docs <summary.json>` rewrites only the blocks between `<!-- bench:port-vs-original:start -->` and
   `<!-- bench:port-vs-original:end -->` in both pages, with column labels per language and the same numbers; the prose
   is written by hand and translated. The page describes the application as "an ERP API (invoicing, inventory,
   accounting) migrated from Node.js", follows the public naming rule ("the original Node.js application", "its ORM"),
   and links to `/faq/corvis-vs-fastify-express-hono` for the synthetic numbers. `docs/http/performance.md` and the
   comparison page link to it.

## Risks / Trade-offs

- [Client, servers and Postgres share one machine] → oha limited to 8 worker threads (as in `bench/run.sh`); the page
  states it; the latency phase runs below saturation so contention matters less there.
- [Docker Desktop on Windows adds latency to every query] → both sides pay it equally; recorded as an environment note.
- [Results depend on the facten data volume] → 100 products fixed in the setup scenario and recorded in `summary.json`.
- [The ports are private] → the method, the scenario file and the raw summary are public in the repository; only the
  rerun needs access (open question 1).
- [Run time] → 4 scenarios × 3 targets × (3 × 20 s + 3 × 30 s) plus restarts is about 35 minutes; `--quick` (one run,
  shorter windows) exists for trying the setup, and its output is marked as not publishable.
- [A port change between runs makes numbers drift] → the Corvis commit and binary timestamp are in `summary.json`; the
  docs page shows the date and commit of the published run.

## Open questions

1. The facten original and port live in private repositories (`randygil/facten-api` is private; `corvis-tests` is not
   a git repository). Is it acceptable to publish numbers that outsiders cannot rerun, describing the app without its
   name? Recommendation: yes, with the method and `summary.json` public; issue #105 can decide whether to publish the
   app.
2. Headline with equal pools (10 and 10) or with shipped defaults? Recommendation: equal pools as the headline,
   defaults as a secondary table.
3. Should the `login` scenario be in the default set? Recommendation: no; it measures the password hash.
4. Keep the Spanish `bench/results.md` of the synthetic benchmark as is, or switch `bench/report.js` to English in the
   same change? Recommendation: leave it for the English README work (issue #3), which already touches repository
   text.

## Review notes (2026-11-08)

- Same pool size (10 connections) on both sides for the headline numbers.
- Login is not a default scenario (optional, as proposed).
- `bench/results.md` (synthetic benchmark) is translated by #22, which already edits the benchmark texts.
- **Pending maintainer decision:** facten is a private app, so whether the published numbers name it or describe it
  generically is the maintainer's call; the change can be implemented up to the measurement and the docs page waits
  for that answer.
- `bench/run.sh` building in its own `bench/target`, the missing `connection_limit` URL parameter in corvis-orm, and a
  startup warning for CORS origin reflection with credentials are filed as separate issues.
