# Benchmark: a migrated application versus its original

Measures one real application written twice, as a Node.js original and as its Corvis port, under the same load, on the
same database and the same data. The results and how to read them are published in the documentation page
"Benchmark: a migrated application versus its original" (`docs/faq/port-vs-original-benchmark.md`).

`bench/run.sh` (the synthetic benchmark) is a separate tool: it isolates the framework overhead.

## Files

| File | What it is |
|---|---|
| `run.mjs` | The orchestrator: databases, servers, setup, load, memory, `summary.json` |
| `scenario.mjs` | The apidiff scenario that seeds both sides and compares every measured request |
| `report.mjs` | Writes `results.md` from a `summary.json` and rewrites the docs tables |
| `results/<date>-<label>/` | Committed: `summary.json` and the generated English `results.md` of a publishable run |
| `raw/` | Not committed: oha JSON, server logs, memory samples, quick runs |

## Prerequisites

- Node 22 or later (no npm dependencies) and [oha](https://github.com/hatoo/oha) (`cargo install oha --locked`).
- Docker with the Postgres container `corvis-pg` running (`docker start corvis-pg`), which holds the template database
  of the application.
- The application and its original, built beforehand. The script never builds anything and exits with the build command
  when an artifact is missing:
  - the port: `CARGO_TARGET_DIR="$PWD/target" cargo build --release` in the port's folder;
  - the original: `npm run build` in the original's folder.
- The apps are private. Their location comes from `--tests-dir` (or `CORVIS_TESTS_DIR`) and `--app`; `node run.mjs --help`
  shows the defaults.

Close other heavy processes (builds, test suites) before a publishable run: client, servers and database share the
machine.

## Run

```bash
node bench/port-vs-original/run.mjs --pool equal      # headline: 10 database connections on both sides
node bench/port-vs-original/run.mjs --pool default    # each side with its shipped pool size
node bench/port-vs-original/run.mjs --quick           # short run to check the setup; not publishable
node bench/port-vs-original/run.mjs --help
```

Options:

| Option | Meaning |
|---|---|
| `--targets original,corvis,corvis-1t` | What to measure (the original runs first; it fixes the latency rate) |
| `--scenarios health,products-list,product-get,invoice-preview` | What to measure; `login` is optional and off by default because it measures the password hash |
| `--pool equal\|default` | Pool sizes (see above) |
| `--quick` | One run per phase, 2/4/6 s windows; output goes to `raw/quick/`, never to `results/` |
| `--label <text>` | Suffix of the results folder (default: the pool mode) |
| `--rate <n>` | Fixed rate of the latency phase instead of half the original's median throughput |
| `--ports a,b` | Ports of the original and the port (default `3611,3612`, loopback only) |
| `--keep-db` | Keep the two benchmark databases (`bench_original`, `bench_port`) afterwards |

### What a run does

1. Resets the two benchmark databases from the application's template and starts both servers on `127.0.0.1`.
2. Runs `scenario.mjs` with apidiff: it seeds the data through the HTTP API and sends each measured request once to
   both sides. Any difference stops the run before anything is measured.
3. For every scenario and target, on a freshly started server: one request per target to check the response, a warmup,
   throughput (closed loop, 100 connections, 15 s) three times, then latency (open loop at half of the original's median
   throughput, with latency correction, 30 s) three times. A health check follows every run.
4. Reads the memory of the server process: idle, the operating system's peak (`PeakWorkingSet64` on Windows, `VmHWM` on
   Linux), after the load, and a 500 ms sample series.
5. Writes `summary.json` and `results.md`, drops the databases and stops every process it started (also on errors and
   Ctrl+C).

Expected duration of a full run: about 35 minutes per pool mode (4 scenarios, 3 targets, about 2.5 minutes each plus
startup). A `--quick` run takes about 4 minutes.

## Publishing

Only runs with `"publishable": true` can be published. The tables of the docs pages live between the
`bench:port-vs-original` markers and are regenerated from committed summaries:

```bash
node bench/port-vs-original/report.mjs --docs bench/port-vs-original/results/<equal run>/summary.json \
  --secondary bench/port-vs-original/results/<default run>/summary.json
node bench/port-vs-original/report.mjs <summary.json>        # regenerate results.md
node bench/port-vs-original/report.mjs --docs-placeholder    # empty the tables again
```

`--docs` refuses a quick summary unless `--allow-quick` is given (only to check the plumbing; never commit the result).
Running it twice leaves the pages unchanged, and it never touches text outside the markers.

The published material describes the application generically; its name and code stay out of the committed results
and the docs.
