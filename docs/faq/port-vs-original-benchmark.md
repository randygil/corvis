---
status: documented
---

# Benchmark: a migrated application versus its original

The [synthetic benchmark](/faq/corvis-vs-fastify-express-hono) measures the framework's overhead on four
trivial endpoints. This page measures what a team gets when it migrates a real backend: the same
application, written once in Node.js and once in Corvis, running the same requests against the same
database on the same machine.

## What was measured

The application is a real invoicing API (invoicing, inventory and accounting) with 74 routes on PostgreSQL, migrated from
Node.js. The Corvis version passes the differential test suite against the original (the same requests give the same
status, headers and bodies, and both leave the database in the same state), so both sides do the same work. The
application is private: this page publishes the numbers and the method, not its name or its code.

Four scenarios, all read-only:

- **Health check**: the framework path with its global guards and interceptors; no authentication, no database.
- **Product list**: authentication and role guards, an ORM query and a large JSON response (100 products).
- **One product by id**: id validation, an ORM lookup and a small JSON response.
- **Invoice preview**: body validation, several database reads and decimal fiscal arithmetic for 5 items and one payment;
  nothing is persisted.

Login is left out of the default set because it measures the cost of the password hash, not the framework.

## How it was measured

- **Same data, same requests.** Both databases are created from one template and seeded through the HTTP API with one
  scenario. Every measured request is sent once to both servers and the responses are compared before anything is
  measured; a difference stops the run.
- **One server at a time, fresh process per scenario**, with a warmup. The load generator is
  [oha](https://github.com/hatoo/oha), limited to 8 threads.
- **Throughput**: closed loop, 100 connections, 15 s, three runs; the median is reported.
- **Latency**: open loop at a fixed rate, with latency correction. The rate is half of what the original sustains in the
  throughput phase and is the same for every target, so both sides run below saturation and the percentiles compare
  service time, not queue length. Three runs of 30 s; the median of each percentile is reported.
- **Memory**: the resident set of the server process at idle, its peak during the scenario (the operating system's
  peak counter plus a 500 ms sample series, so a peak between samples is not missed) and after the load. The database
  is shared and not counted.
- **Same database pool.** The headline numbers give both sides a pool of 10 connections, so the comparison is not a
  pool-size comparison. The second table lets each side keep the pool size it ships with (the original's ORM opens
  `2 x cores + 1` connections by default).
- Corvis is measured twice: with all cores (the default runtime) and with a single worker thread, which is the per-core
  comparison with one Node.js process.

## Results

The tables are generated from the committed `summary.json` of each run (`bench/port-vs-original/results/`) by
`report.mjs --docs`; do not edit them by hand.

<!-- bench:port-vs-original:start -->

Run of 2026-11-08 on Intel(R) Core(TM) i9-14900KF (32 threads), Windows_NT 10.0.26300 (win32 x64). Corvis commit 1eb6156784c8. 3 runs per measurement, 100 connections.

### Headline: the same database pool size on both sides (10 connections)

**Throughput (requests per second, median of the runs)**

| Scenario | Original (Node.js) | Corvis (1 worker) | Corvis (all cores) | Corvis (all cores) / original |
| --- | ---: | ---: | ---: | ---: |
| Health check: no authentication, no database | 24221 | 38121 | 120479 | 5.0x |
| Product list (100 products): authentication, ORM query, large JSON | 1512 | 1136 | 7592 | 5.0x |
| One product by id: id validation, ORM lookup, small JSON | 4467 | 6149 | 21140 | 4.7x |
| Invoice preview (5 items, 1 payment): body validation, several reads, decimal arithmetic, nothing persisted | 1075 | 1450 | 4445 | 4.1x |

**Latency at a fixed rate (ms, median of the runs)**

| Scenario | Rate (req/s) | Target | Achieved (req/s) | p50 | p90 | p99 | max |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| Health check: no authentication, no database | 12111 | Original (Node.js) | 11087 | 4.57 | 807 | 2322 | 2537 |
|  |  | Corvis (1 worker) | 12110 | 1.76 | 3.28 | 5.36 | 15.9 |
|  |  | Corvis (all cores) | 12105 | 0.98 | 1.44 | 1.85 | 9.73 |
| Product list (100 products): authentication, ORM query, large JSON | 756 | Original (Node.js) | 756 | 2.27 | 3.19 | 6.63 | 29.4 |
|  |  | Corvis (1 worker) | 756 | 1.52 | 8.30 | 211 | 285 |
|  |  | Corvis (all cores) | 756 | 1.93 | 3.00 | 3.95 | 21.6 |
| One product by id: id validation, ORM lookup, small JSON | 2233 | Original (Node.js) | 2232 | 2.88 | 4.74 | 40.6 | 58.2 |
|  |  | Corvis (1 worker) | 2233 | 2.76 | 5.12 | 10.4 | 21.1 |
|  |  | Corvis (all cores) | 2233 | 1.77 | 2.26 | 2.89 | 28.2 |
| Invoice preview (5 items, 1 payment): body validation, several reads, decimal arithmetic, nothing persisted | 538 | Original (Node.js) | 538 | 2.40 | 4.36 | 6.36 | 29.1 |
|  |  | Corvis (1 worker) | 538 | 2.37 | 2.77 | 4.11 | 22.4 |
|  |  | Corvis (all cores) | 538 | 2.93 | 3.59 | 5.16 | 23.2 |

**Memory of the server process (MB)**

| Scenario | Target | Idle | Peak | After the load |
| --- | --- | ---: | ---: | ---: |
| Health check: no authentication, no database | Original (Node.js) | 174.1 | 234.7 | 234.7 |
|  | Corvis (1 worker) | 20.0 | 24.2 | 20.5 |
|  | Corvis (all cores) | 20.7 | 31.3 | 28.0 |
| Product list (100 products): authentication, ORM query, large JSON | Original (Node.js) | 176.3 | 456.3 | 449.6 |
|  | Corvis (1 worker) | 21.7 | 30.3 | 25.1 |
|  | Corvis (all cores) | 21.9 | 60.0 | 45.4 |
| One product by id: id validation, ORM lookup, small JSON | Original (Node.js) | 176.5 | 388.0 | 387.9 |
|  | Corvis (1 worker) | 20.8 | 29.1 | 23.8 |
|  | Corvis (all cores) | 21.9 | 46.3 | 37.4 |
| Invoice preview (5 items, 1 payment): body validation, several reads, decimal arithmetic, nothing persisted | Original (Node.js) | 178.5 | 427.6 | 426.8 |
|  | Corvis (1 worker) | 21.5 | 31.8 | 25.4 |
|  | Corvis (all cores) | 22.3 | 52.1 | 42.1 |

**Errors and unexpected statuses**

None in any run.

### Secondary: each side with its shipped pool size

Run of 2026-11-08 on Intel(R) Core(TM) i9-14900KF (32 threads), Windows_NT 10.0.26300 (win32 x64). Corvis commit 1eb6156784c8. 3 runs per measurement, 100 connections.

**Throughput (requests per second, median of the runs)**

| Scenario | Original (Node.js) | Corvis (1 worker) | Corvis (all cores) | Corvis (all cores) / original |
| --- | ---: | ---: | ---: | ---: |
| Health check: no authentication, no database | 22090 | 40674 | 131837 | 6.0x |
| Product list (100 products): authentication, ORM query, large JSON | 1556 | 1192 | 7656 | 4.9x |
| One product by id: id validation, ORM lookup, small JSON | 4511 | 7503 | 21296 | 4.7x |
| Invoice preview (5 items, 1 payment): body validation, several reads, decimal arithmetic, nothing persisted | 2042 | 1633 | 4600 | 2.3x |

**Latency at a fixed rate (ms, median of the runs)**

| Scenario | Rate (req/s) | Target | Achieved (req/s) | p50 | p90 | p99 | max |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| Health check: no authentication, no database | 11045 | Original (Node.js) | 11041 | 2.33 | 4.23 | 6.31 | 14.2 |
|  |  | Corvis (1 worker) | 11041 | 1.71 | 3.10 | 4.22 | 11.9 |
|  |  | Corvis (all cores) | 11039 | 0.99 | 1.50 | 1.92 | 7.67 |
| Product list (100 products): authentication, ORM query, large JSON | 778 | Original (Node.js) | 778 | 2.20 | 3.89 | 7.06 | 17.8 |
|  |  | Corvis (1 worker) | 778 | 1.96 | 6.41 | 27.4 | 55.7 |
|  |  | Corvis (all cores) | 778 | 1.96 | 3.04 | 4.18 | 19.1 |
| One product by id: id validation, ORM lookup, small JSON | 2256 | Original (Node.js) | 2255 | 2.84 | 4.69 | 7.27 | 18.1 |
|  |  | Corvis (1 worker) | 2256 | 2.57 | 3.61 | 4.54 | 16.2 |
|  |  | Corvis (all cores) | 2255 | 1.67 | 2.13 | 2.60 | 13.2 |
| Invoice preview (5 items, 1 payment): body validation, several reads, decimal arithmetic, nothing persisted | 1021 | Original (Node.js) | 1021 | 1.73 | 2.26 | 3.55 | 11.2 |
|  |  | Corvis (1 worker) | 1021 | 2.61 | 3.10 | 4.07 | 33.3 |
|  |  | Corvis (all cores) | 1021 | 2.58 | 2.98 | 3.60 | 30.5 |

**Memory of the server process (MB)**

| Scenario | Target | Idle | Peak | After the load |
| --- | --- | ---: | ---: | ---: |
| Health check: no authentication, no database | Original (Node.js) | 174.4 | 233.6 | 233.6 |
|  | Corvis (1 worker) | 19.8 | 24.1 | 20.3 |
|  | Corvis (all cores) | 21.0 | 32.3 | 29.0 |
| Product list (100 products): authentication, ORM query, large JSON | Original (Node.js) | 176.5 | 476.6 | 470.7 |
|  | Corvis (1 worker) | 21.3 | 30.4 | 24.5 |
|  | Corvis (all cores) | 21.8 | 59.1 | 43.2 |
| One product by id: id validation, ORM lookup, small JSON | Original (Node.js) | 177.2 | 390.3 | 389.4 |
|  | Corvis (1 worker) | 20.9 | 28.9 | 24.2 |
|  | Corvis (all cores) | 21.9 | 46.3 | 38.2 |
| Invoice preview (5 items, 1 payment): body validation, several reads, decimal arithmetic, nothing persisted | Original (Node.js) | 177.4 | 432.6 | 432.0 |
|  | Corvis (1 worker) | 21.2 | 31.5 | 25.1 |
|  | Corvis (all cores) | 22.2 | 52.7 | 42.4 |

**Errors and unexpected statuses**

None in any run.

<!-- bench:port-vs-original:end -->

## How to read them

- Everything runs on one machine: the load generator, both servers and PostgreSQL (in a Docker container, which adds
  latency to every query equally for both sides). Absolute numbers depend on the machine; the ratios are what carries
  over.
- A throughput ratio above 1 means Corvis served more requests per second than the original under the same load.
- Latency was measured at one fixed rate per scenario. If a target's achieved rate is below that rate, it was saturated and its percentiles include queueing; read them as such.
- In this run the original did not sustain the fixed rate of the health-check scenario (achieved rate below the rate
  in both tables), so its p90 and p99 there include queueing and say little about service time. The Corvis
  single-worker product list shows a long tail (p99) at the same rate that its median does not.
- Memory peaks include process startup and the warmup, because the peak is the operating system's lifetime counter of a
  process started for that scenario.
- Any error or unexpected status appears next to the affected cell and in the errors list; "none" means there were none
  in any run.

## Limits

- One application, one machine, one operating system (Windows 11). Linux numbers may differ.
- The application is private, so you cannot rerun this exact benchmark. The method, the scenario and the raw
  `summary.json` of every published run are in the repository, in `bench/port-vs-original/`, and the script takes the
  application as a parameter.
- Read-only scenarios: writes (issuing invoices) are not measured yet.
- A change in the framework between runs changes the numbers; each table shows the date and the Corvis commit of its run.

## Repeat it

```bash
node bench/port-vs-original/run.mjs --pool equal
node bench/port-vs-original/run.mjs --pool default
node bench/port-vs-original/report.mjs --docs <equal summary.json> --secondary <default summary.json>
```

The full run takes about 35 minutes per pool mode; `--quick` runs a short version to check the setup and its output is
marked as not publishable. The requirements and options are in `bench/port-vs-original/README.md`.
