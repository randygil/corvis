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

Results pending the publishable run.

<!-- bench:port-vs-original:end -->

## How to read them

- Everything runs on one machine: the load generator, both servers and PostgreSQL (in a Docker container, which adds
  latency to every query equally for both sides). Absolute numbers depend on the machine; the ratios are what carries
  over.
- A throughput ratio above 1 means Corvis served more requests per second than the original under the same load.
- Latency was measured at one fixed rate per scenario. If a target's achieved rate is below that rate, it was saturated and its percentiles include queueing; read them as such.
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
