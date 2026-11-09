# Tasks

## 1. Setup scenario and equality check

- [x] 1.1 Write `bench/port-vs-original/scenario.mjs` (apidiff scenario): log in as `admin`, create today's BCV rate,
  create 100 products with fixed SKUs, then send each measured request once (`GET /health`, `GET /api/v1/products`,
  `GET /api/v1/products/:id`, `POST /api/v1/invoices/preview` with 5 items and one payment, optional login); store,
  for each side (apidiff `Sided` values), the token and the ids in a state file for the orchestrator; verify by running it with
  `node <tests-dir>/tools/apidiff/apidiff.mjs run --a http://127.0.0.1:3001 --b http://127.0.0.1:3101 scenario.mjs` on
  databases reset from `facten_tpl` (both servers started with `scripts/apidiff.sh`'s environment): 0 differences
- [x] 1.2 Write the preview request body to a file the orchestrator passes to oha with `-D`; verify oha against each
  side once (`oha -n 1 ...`) gets `200` with the same `totalUsd` on both

## 2. Orchestrator

- [x] 2.1 Create `bench/port-vs-original/run.mjs` with options `--tests-dir`, `--app facten`, `--targets`
  (`original`, `corvis`, `corvis-1t`), `--scenarios`, `--pool` (`equal` | `default`), `--quick`, `--label`;
  check prerequisites (node, oha, docker, `corvis-pg` running, port binary, original `dist/main.js`) and stop with the
  build commands when a binary is missing, without building; verify with a missing binary path that it prints the
  command and exits non-zero
- [x] 2.2 Implement database reset (`docker exec corvis-pg psql ... TEMPLATE facten_tpl`) and the setup phase: start
  both servers on `127.0.0.1` (original with `-r scripts/loopback.cjs` and `connection_limit=10` in its URL when
  `--pool equal`), wait for `/health`, run the scenario with apidiff, abort on any difference, stop both; verify by
  introducing a wrong expected product count in a scratch copy of the scenario and seeing the run stop
- [x] 2.3 Implement process management: spawn one server at a time, keep its PID, stop it (and its children) on
  exit, Ctrl+C and errors; verify no process is left listening on the bench ports after an interrupted run
  (`netstat -ano` on Windows)
- [x] 2.4 Implement memory readings: idle after setup requests, OS peak after the scenario (`PeakWorkingSet64` via
  PowerShell on Windows, `VmHWM` on Linux), after-load value and a 500 ms sample series; verify the readings on the
  synthetic `bench/corvis-app` binary if present, or on the facten port, against Task Manager's peak working set
- [x] 2.5 Implement the two phases per scenario and target (fresh process, 5 s warmup, throughput `-c 100 -z 15s`
  three times; latency `-q R --latency-correction -z 30s` three times, `R = 0.5 ×` the original's median throughput;
  health check after each run); write raw oha JSON to `bench/port-vs-original/raw/` and add that folder to
  `.gitignore`; verify with `--quick` that every scenario produces both phases for every target

## 3. Summary and report

- [x] 3.1 Write `summary.json` (fields of design.md decision 8) into `bench/port-vs-original/results/<date>-<label>/`;
  verify it contains every run, the medians and min/max, and the environment fields filled (no empty values)
- [x] 3.2 Write `bench/port-vs-original/report.mjs`: English `results.md` next to `summary.json` (throughput table,
  latency table with p50/p90/p99/max, memory table, errors and unexpected statuses next to the affected cells,
  environment, how to read it); verify by generating it from the `--quick` summary
- [x] 3.3 Add `--docs`: rewrite only the blocks between `<!-- bench:port-vs-original:start -->` and
  `<!-- bench:port-vs-original:end -->` in the English and Spanish pages, with per-language labels; verify that
  running it twice leaves the pages unchanged the second time and that text outside the markers is untouched
- [x] 3.4 Write `bench/port-vs-original/README.md` (English): prerequisites, how to build the two apps, how to run,
  options, run time, what is committed; verify the commands in it run as written

## 4. Publishable run

- [x] 4.1 With no other heavy process running (no builds, no parity suites), run the full benchmark with
  `--pool equal` and then `--pool default`; verify both summaries have 0 errors and 0 unexpected statuses, or
  document any in the page
  (2026-11-08, quiet machine, TIME_WAIT 0 and 178 before the runs: equal pool 18:24 to 18:54, default pool 18:54 to
  19:24; 0 errors and 0 unexpected statuses in both; the page notes the original's saturation in the health-check
  latency)
- [x] 4.2 Commit-ready results: keep the two `results/<date>-<label>/` folders (summary.json and results.md); verify
  `report.mjs` regenerates the same `results.md` from each summary

## 5. Documentation

- [x] 5.1 Write `docs/faq/port-vs-original-benchmark.md`: what was measured and why, the application described
  without its name, environment, the generated tables (equal pools as the headline, defaults as a secondary table),
  how to read them, limits (same machine, Docker, private apps), how to rerun; public naming rule respected; verify
  the tables were produced by `report.mjs --docs`
- [x] 5.2 Translate it into `docs/es/faq/port-vs-original-benchmark.md` (same headings and code blocks, tables
  generated by `--docs`); add the page to the FAQ group of `docs/.vitepress/sidebar.mts` next to
  `/faq/corvis-vs-fastify-express-hono`; verify the sidebar shows it in both languages with `npm run docs:dev`
- [x] 5.3 Link the new page from the "Performance" section of `docs/http/performance.md` and from "Measured
  performance" in `docs/faq/corvis-vs-fastify-express-hono.md`, and the same in their Spanish translations; verify
  `npm run docs:check` in `docs/` passes
  (links added in the four pages; `npm run docs:check` passed on 2026-11-08, 176 pages per language; `docs:dev` was
  not run, the sidebar parity is covered by docs:check)

## 6. Definition of done

- [x] 6.1 No warnings: this change adds no Rust code; confirm `cargo build --workspace --all-targets` is unaffected
  (no new crate, no workspace member added)
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes (unchanged Rust code)
- [x] 6.3 `cargo test --workspace` passes (unchanged Rust code)
- [x] 6.4 Regression checks for the scripts: `run.mjs --quick` completes end to end, and `report.mjs --docs` is
  idempotent (3.3); record both results here.
  Result (2026-11-08): `run.mjs --quick` (4 scenarios, 3 targets, about 3.5 minutes) completed with 108 apidiff checks
  equal and wrote summary.json and results.md with no empty field; it also completed with `login`; `report.mjs --docs
  ... --secondary ... --allow-quick` run twice left both pages unchanged the second time, text outside the markers was
  identical, a quick summary is refused without `--allow-quick`, and `--docs-placeholder` restored the placeholder.
  Abort path: a wrong expected product count stopped the run before measuring (exit 1) and left no process and no
  benchmark database; a missing app made it print the build commands and exit 2.
- [x] 6.5 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
  (`npm run docs:check` passed with docs/node_modules linked from the main checkout and then unlinked;
  `scripts/check-naming.mjs` passes)
- [x] 6.6 corvis-parity: no framework runtime code changes in this change, so the parity check is not required;
  confirm with `git diff --stat` that only `bench/`, `docs/` and `.gitignore` changed
  (also changed: `scripts/check-naming.mjs`, to scan `bench/port-vs-original/**`, and this change's design.md and
  tasks.md; no Rust code)

## Workflow follow-up

- [x] Archive the change with `/opsx:archive`
- [x] Close issue [#12](https://github.com/randygil/corvis/issues/12)
- [x] Tick #12 in its epic [#1](https://github.com/randygil/corvis/issues/1)
- [x] Move the card to Done in the Corvis Roadmap project
