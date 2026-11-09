# Reproducible benchmark: port vs original

Implements [randygil/corvis#12](https://github.com/randygil/corvis/issues/12) (epic
[#1](https://github.com/randygil/corvis/issues/1)).

## Why

The only numbers Corvis publishes today come from `bench/`: a hello-world style application with four endpoints
(`GET /`, `GET /json`, `GET /users/:id`, `POST /users`), written twice and measured with oha (`bench/run.sh`,
`bench/results.md`, quoted in `docs/faq/corvis-vs-fastify-express-hono.md`). They show the framework's overhead, not
what a team gets when it migrates a real backend: authentication guards, validation, a database through the ORM,
decimal arithmetic, large JSON responses. The positioning of the project is exactly that migration, and the three port
apps (consi, facten, houseplan) already prove behavioral parity with apidiff. One of them, measured against its Node
original under the same load and the same database, is the evidence the project is missing, and the basis for the
case study of issue #105.

## What Changes

- New `bench/port-vs-original/`: a Node orchestrator (`run.mjs`, no npm dependencies) that, for the facten port and
  its original:
  - resets one database per side from the same template, seeds both through the HTTP API with one apidiff scenario,
    and checks that the measured requests get equal responses from both before measuring anything;
  - starts one server at a time, each scenario on a fresh process, with a warmup;
  - measures **throughput** (closed loop, fixed connections) and **latency p50/p90/p99/max** (open loop, fixed rate
    the original can sustain, with latency correction), using oha;
  - measures **memory** of the server process: resident set at idle, peak during each scenario (OS peak counter plus
    sampling), and after the load;
  - repeats each measurement three times and reports the median and the spread;
  - writes the raw oha JSON, a `summary.json` with the environment, and an English `results.md`.
- A report step (`report.mjs`) that also rewrites the results table of the docs pages between markers, in English and
  Spanish, so published numbers always come from a committed `summary.json`.
- A new docs page, "Benchmark: a migrated application versus its original" (`docs/faq/port-vs-original-benchmark.md`
  and `docs/es/...`), linked from `docs/http/performance.md` and `docs/faq/corvis-vs-fastify-express-hono.md`.

## Capabilities

### New Capabilities

- `bench/port-vs-original`: what the port-versus-original benchmark measures, the conditions that make two runs
  comparable, and how its results are published.

### Modified Capabilities

None.

## Impact

- New files only: `bench/port-vs-original/{run.mjs,report.mjs,scenario.mjs,README.md,results/}` and the two docs
  pages; small link additions in `docs/http/performance.md`, `docs/faq/corvis-vs-fastify-express-hono.md` and their
  Spanish translations; a sidebar entry in `docs/.vitepress/sidebar.mts`; `.gitignore` entries for raw run output.
- No framework code changes, so no parity run is required.
- Prerequisites to run it (not to build the workspace): Node 22 or later, oha, Docker with the `corvis-pg` Postgres
  container, the facten port and original in `corvis-tests` (built beforehand; the script never builds and never
  creates a Cargo target directory).
- The port and the original are private repositories, so outsiders can read the method and the raw results but cannot
  rerun this benchmark (open question 1 in design.md).

## Out of scope

- Benchmarking consi and houseplan (external services, background jobs and a local embeddings model make them noisy);
  the script takes the app as a parameter so they can be added later.
- Write-heavy scenarios (issuing invoices consumes control numbers and needs unique data per request): a follow-up once
  the read scenarios are stable.
- The case study page with parity, memory and latency for a published audience: issue #105, which consumes these
  results.
- Linux or CI runs: the script is written to work on Linux, but the published numbers in this change come from the
  maintainer's Windows machine; running it in CI needs a dedicated machine (noisy shared runners).
- `bench/run.sh` builds into its own `bench/target` directory, against the shared-target rule: one-line follow-up
  (build `-p corvis-app` in the workspace target).
- corvis-orm does not read `connection_limit` from the database URL the way the original's ORM does (it only takes
  `OrmOptions::max_connections`, `crates/corvis-orm/src/client.rs:235`): a parity gap worth its own issue.
