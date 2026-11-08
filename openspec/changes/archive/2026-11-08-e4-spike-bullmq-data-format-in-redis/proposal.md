# Spike: BullMQ data format in Redis

Implements [randygil/corvis#49](https://github.com/randygil/corvis/issues/49) (epic
[#48](https://github.com/randygil/corvis/issues/48), follow-ups #50–#57).

## Why

Epic #48 promises Redis queues that interoperate with Node BullMQ, so a service can move to Corvis while a Node
producer (or worker) keeps running against the same Redis. Nothing in Corvis touches Redis today:
`docs/application/queues.md` is marked `status: unavailable`, `crates/corvis/Cargo.toml` has no Redis client, and
`Cargo.lock` contains none. Before the follow-up issues can be specified, someone has to find out exactly what
"interoperate" means in Redis terms (keys, hash fields, sorted-set scores, Lua scripts, stream events, version
differences), how to reproduce it from Rust, and how to prove it with a differential test against Node BullMQ.

The spike found two facts that change the plan:

- BullMQ 6 (6.0.0 on 2026-07-30, latest 6.7.2 on 2026-11-07) changed parts of the Redis format: the legacy
  repeatable-job keys are gone, and paused queues keep jobs in `wait` instead of moving them to `paused`. "Compatible
  with BullMQ" has to name a major version.
- Taskforce, the authors of BullMQ, publish an official Rust SDK in the same repository: crate `bullmq-official`
  1.5.0 (MIT, 2026-11-04). Its published crate ships 64 Lua scripts; 63 are byte-identical to the scripts of Node
  BullMQ 6.7.2 and the 64th (`getCounts-1`) differs only by an optional, read-only argument. It runs on Tokio and the
  `redis` crate.

## What Changes

This change produces research and a plan. It does not change code.

- `design.md` documents the BullMQ Redis data model as of Node BullMQ 6.7.2 (with the differences from 5.81.5): key
  names, job hash fields, score encodings, the Lua scripts and their roles, the job lifecycle, worker defaults, the
  events stream, and the validation rules a producer applies. Each fact cites the file it comes from.
- `design.md` compares four ways to implement the format in Rust (wrap `bullmq-official`, vendor the Lua scripts on
  our own client, write the format without the scripts, use a third-party crate) and three Redis clients, and
  recommends one of each with the evidence.
- `design.md` defines how the interoperability tests will run: a pinned Node BullMQ harness driven from Rust tests,
  a Redis service, normalized state snapshots and event sequences compared against a Node-only reference run.
- `specs/` states the requirements the epic's features must meet: the behavior that must match Node BullMQ and the
  differential test that verifies each one. They are the contract the follow-up changes implement.
- `tasks.md` is the plan for #50–#57, one group per issue, in the recommended order.

## Capabilities

### New Capabilities
- `queues/redis-format`: which BullMQ version and Redis format Corvis queues are compatible with, key layout, job
  hash, events stream, documented divergences.
- `queues/module`: Redis connection options, queue registration and injection.
- `queues/producer`: adding jobs (`add`, `add_bulk`) and job options, with the same validation and stored result as
  Node BullMQ.
- `queues/workers`: processing jobs: locks, concurrency, retries and backoff, stalled jobs, progress, graceful
  shutdown.
- `queues/events`: the events stream and in-process and cross-process listeners.
- `queues/schedulers`: job schedulers (cron and interval repetition).
- `queues/rate-limiting`: worker and global rate limits and global concurrency.
- `queues/flows`: parent/child jobs and their dependency keys.

## Impact

- No code, docs or dependency changes in this change.
- Follow-ups (#50–#55) will add an optional `queues` feature to `crates/corvis` (off by default) with a dependency on
  `bullmq-official`, new handler kinds in `crates/corvis/src/discovery.rs`, new attributes in `crates/corvis-macros`,
  and a Node interop harness under `crates/corvis/tests/`. #56 adds a Redis service to CI. #57 rewrites
  `docs/application/queues.md` and `docs/es/application/queues.md` and adds an example.
- Archive this change with `--skip-specs`: the requirements describe behavior that does not exist yet. Each
  follow-up copies the requirements it implements into its own delta (see design.md, Decision 9).

## Out of scope

- Any implementation. The tasks in `tasks.md` belong to #50–#57.
- Legacy `repeat` job options and `getRepeatableJobs`/`removeRepeatable` (removed in BullMQ 6): not planned. A new
  issue can add them if a migrating app still runs BullMQ 5 repeatable jobs.
- The original Bull (pre-BullMQ, package `bull`) Redis format: different library and format; possible new issue.
- Sandboxed processors (worker threads or child processes running a separate file): not applicable to Rust.
- OpenTelemetry for queues: worth its own issue once the queue API settles.
- BullMQ 6 "relay" scripts (`relay*-1.lua`) and the Postgres backend (`bullmq/postgres`): new in 6.x, unrelated to
  Redis job interop.
- Redis Sentinel: the official Rust SDK does not support it (its `FEATURE_PARITY.md`); new issue if needed.
- Using Redis for the cache store, throttler storage, WebSocket adapter or a microservices transport: the Redis
  client chosen here should be reused there, but each is its own issue.
- A queue dashboard (Bull Board and similar): works today against the same Redis keys once queues exist; no Corvis
  work needed.
