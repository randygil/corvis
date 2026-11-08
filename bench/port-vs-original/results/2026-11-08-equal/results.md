# Benchmark: a migrated application versus its original

Date: 2026-11-08. Pool mode: equal (the original and the port both use 10 database connections).
Protocol: 3 run(s) per measurement, 100 connections, warmup 5 s, throughput 15 s (closed loop), latency 30 s (open loop at half of the original's median throughput, with latency correction). 100 products seeded.

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
