# Benchmark: a migrated application versus its original

Date: 2026-11-08. Pool mode: default (each side uses its shipped pool size).
Protocol: 3 run(s) per measurement, 100 connections, warmup 5 s, throughput 15 s (closed loop), latency 30 s (open loop at half of the original's median throughput, with latency correction). 100 products seeded.

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

## Environment

- OS: Windows_NT 10.0.26300 (win32 x64)
- CPU: Intel(R) Core(TM) i9-14900KF, 32 logical cores, 32 GB of memory
- Node 24.13.0, rustc 1.99.0 (b940084d7 2026-09-28), oha 1.16.0
- Postgres: 17.11 (Debian 17.11-1.pgdg12+2) (Docker container on 127.0.0.1)
- Corvis commit: 1eb6156784c8; port binary built 2026-11-08T22:24:03.705Z
- Original: framework 11.2.1 on express, ORM 6.19.3
- Load generator: closed and open loop, 100 connections, 8 worker threads, keep-alive, wait for in-flight requests

## How to read it

- The load generator, both servers and the database run on one machine; absolute numbers depend on it, ratios are what carries over.
- Throughput is a closed loop at saturation. Latency is measured separately at a fixed rate that the original sustains, so it compares service time, not queueing.
- If a target's achieved rate is below the fixed rate, it was saturated and its percentiles include queueing. An error such as "os error 10048" (Windows) is a failure to open a new client connection (the load generator ran out of local ports); check the raw data for how often.
- Memory is the resident set of the server process only (the database is shared). Peak is the operating system counter of the process lifetime, including startup and the warmup.
- Each cell of the throughput table is the median of the runs; `summary.json` has every run with its minimum and maximum.
- Every measured request was sent once to both servers before measuring, and their responses were compared (status, headers, body).
