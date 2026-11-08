# Tasks

This spike changes no code. Group 1 closes the spike. Groups 2–9 are the plan for the follow-up issues of epic #48,
in the recommended order (design.md, Decision 10). Each follow-up becomes its own OpenSpec change (`/opsx:propose`
with the change name from `ROADMAP.md`), copies the requirements it implements from this change's `specs/queues/*`
into its own delta (Decision 9), and refines these tasks in its own `tasks.md`.

## 1. Spike close-out (#49)

- [x] 1.1 Maintainer reviews design.md and answers the Open questions (target version, module name, dispatch
  default, CI frequency, `meta.version`, dependency pinning); record the answers in design.md; verify no question is
  left without an answer or an owner issue
  - Result: answered in design.md, Review notes (2026-11-08).
- [x] 1.2 Open the "Out of scope" items the maintainer wants tracked as new issues (legacy `repeat` options, Sentinel,
  queue telemetry, Redis-backed cache/throttler stores); verify each has an issue linked from the epic or is
  explicitly dropped
  - Result: dropped for now (not needed by phase 1); revisit if a port needs them.
- [x] 1.3 Report the `meta` `library`/`version` divergence of `bullmq-official` upstream (only if the maintainer
  agrees to file it); verify the link is added to design.md
  - Result: not filed upstream; Corvis writes `meta.version` itself (Review notes, point 5).

## 2. #50 Redis connection and BullModule (`e4-redis-connection-and-bullmodule`)

- [ ] 2.1 Add the optional `queues` feature to `crates/corvis/Cargo.toml` with `bullmq = { package =
  "bullmq-official", version = "~1.5", optional = true }`; verify `cargo tree -p corvis --features queues -d` and
  record build-time and duplicate crates (`sha1`) in the change's design
- [ ] 2.2 Create `crates/corvis/src/queues/mod.rs` (behind `#[cfg(feature = "queues")]`) with `BullRootOptions`
  (URL or typed fields, prefix default `bull`, default job options) mapped to the crate's `RedisConnectionOptions`;
  verify unit tests of the mapping, including a URL with credentials whose `Debug`/`Display` and errors redact the
  password
- [ ] 2.3 Implement `BullModule::for_root`, `for_root_async` (pattern of `ScheduleModule::for_root*`,
  `crates/corvis/src/modules/schedule/mod.rs:557-584`) and `register_queue`; verify a test module resolves a `Queue`
  per registered name
- [ ] 2.4 Add `#[inject_queue("name")]` to `#[injectable]` (`crates/corvis-macros/src/injectable.rs:31`, next to
  `#[inject]`), resolving the queue token; verify an injection test and a bootstrap-failure test for an unregistered
  name with the message from the `queues/module` spec
- [ ] 2.5 Check the Redis server version at startup (minimum 6.2) and close connections in `close()`; verify tests
  gated by `CORVIS_REDIS_URL` (skip when unreachable) and a `CLIENT LIST` check after close
- [ ] 2.6 Decide and implement the unreachable-Redis bootstrap behavior and the `meta.version` question; verify the
  decision is in the change's design and tested

## 3. #56 (part 1) Interop harness skeleton (`e4-interoperability-tests-with-node-bullmq`)

- [ ] 3.1 Create `crates/corvis/tests/interop/bullmq/package.json` pinning `bullmq` 6.7.2 and `bullmq5` (npm alias of
  5.81.5) exactly, plus `package-lock.json`; verify `npm ci` in that folder installs both
- [ ] 3.2 Write `driver.mjs`: JSON-lines protocol on stdin/stdout with `add`, `addBulk`, `startWorker` (scripted
  outcomes per job name), `stopWorker`, `listen`, `getJob`, `pause`, `resume`, `quit`; verify by hand with
  `echo '{"cmd":"add",...}' | node driver.mjs` against a local Redis on 127.0.0.1
- [ ] 3.3 Write the Rust side in `crates/corvis/tests/bullmq_interop.rs`: spawn and talk to the driver, per-test
  random prefix, key cleanup, skip logic (`CORVIS_REDIS_URL`, `CORVIS_BULLMQ_NODE_DIR`, `CORVIS_BULLMQ_REQUIRED=1`);
  verify the test is reported as skipped with a reason when Redis is down, and fails when `REQUIRED=1`
- [ ] 3.4 Write the snapshot dumper and normalizer (types and contents of every `<prefix>:<q>:*` key and the events
  stream; placeholders for UUIDs and tokens; ordinal timestamps; parsed JSON fields; `stacktrace` by length; `meta`
  version fields excluded); verify unit tests on canned dumps, and that two Node-only runs of one scenario normalize
  to equal snapshots
- [ ] 3.5 Add the script-hash check of the `queues/redis-format` spec (SHA1 of each script the crate loads vs the
  pinned Node package); verify it passes, and fails when one script file in the Node folder is edited
- [ ] 3.6 First scenario: one job added and completed, in the four producer×worker combinations against the Node-only
  reference; verify `cargo test -p corvis --features queues --test bullmq_interop` passes locally with Redis

## 4. #51 Queue: add, add_bulk and job options (`e4-queue-add-add-bulk-and-job-options`)

- [ ] 4.1 Define `JobOptions` (delay, attempts, backoff fixed/exponential/custom with jitter, priority, lifo, job_id,
  remove_on_complete/remove_on_fail as bool/count/`{age, count}`, size_limit, deduplication) and its mapping to the
  crate's options; verify unit tests that the serialized `opts` equals Node's JSON for a table of option sets
- [ ] 4.2 Implement `Queue::add<T: Serialize>` and `add_bulk`, returning a `Job` handle with the id; verify interop
  scenarios for standard, delayed, prioritized, lifo and existing-`job_id` adds (snapshot equal to the reference)
- [ ] 4.3 Implement the validation table of the `queues/producer` spec before any Redis call; verify the differential
  validation test (both sides reject the same cases, no keys written)
- [ ] 4.4 Implement default job options merging (root and per queue); verify the override scenario
- [ ] 4.5 Add the queue getters needed by tests and docs (`get_job`, `get_job_counts`, `pause`, `resume`, `drain`)
  as thin wrappers; verify each against Node on the same queue

## 5. #52 Workers: `#[processor]` and `#[process]` (`e4-workers-processor-and-process`)

- [ ] 5.1 Confirm #16 (`e13-standalone-application-context-runs-events-and-scheduled`) is merged, or merge it first;
  verify a standalone context registers `#[handlers]`
- [ ] 5.2 Add `HandlerKind::Process { queue, job_name: Option<String>, call }` and the `#[processor("q", ...)]` /
  `#[process]` attributes (`crates/corvis/src/discovery.rs:61-67`, `crates/corvis-macros/src/handlers.rs:97`);
  verify macro expansion tests (trybuild or unit) for named and catch-all methods, and a compile error message when
  the `queues` feature is off
- [ ] 5.3 Wire workers in `register_handlers` / bootstrap / close (`crates/corvis/src/modules/mod.rs:19-83`,
  `crates/corvis/src/application.rs:349-357,507-517`): one worker per processor, started before
  `on_application_bootstrap`, closed with a timeout before `before_application_shutdown`; verify a test that
  `close()` waits for an active job
- [ ] 5.4 Implement `Job<T>` (typed data, id, name, attempts, timestamps, `update_progress`, `log`) and the outcome
  mapping of Decision 6 (`Result<R, E>`, `JobError::{unrecoverable, delay_until, rate_limited}`, panic capture,
  deserialization failure as unrecoverable); verify unit tests plus interop scenarios for complete, fail, retry with
  fixed and exponential backoff, unrecoverable, wrong payload, panic
- [ ] 5.5 Stalled jobs: interop scenarios where a Corvis worker dies and a Node worker recovers, and the reverse, with
  short `lockDuration`/`stalledInterval`; verify `stc`, the `stalled` event and the final state match the reference
- [ ] 5.6 Worker options attribute arguments (concurrency, lock_duration, stalled_interval, max_stalled_count,
  limiter, close timeout) with Node's defaults; verify a test that reads back the effective defaults
- [ ] 5.7 Example-sized check: a processor with concurrency 5 processes 100 jobs added by Node; verify all complete
  and no job is processed twice

## 6. #53 Queue events and QueueEvents (`e4-queue-events-and-queueevents`)

- [ ] 6.1 Add `#[on_worker_event("...")]` (local events of a processor's worker); verify completed, failed, progress
  and active listeners run, and a failing listener is logged without changing the job
- [ ] 6.2 Add `#[queue_events_listener("q")]` and `#[on_queue_event("...")]` over the crate's `QueueEvents`, started
  at bootstrap and closed on shutdown; verify interop: Node worker → Corvis listener and Corvis worker → Node
  listener receive the same events
- [ ] 6.3 Differential event-sequence check for the scenarios of groups 3–5; verify the ordered `(event, jobId,
  fields)` lists equal the reference

## 7. #54 Repeatable jobs and rate limiting (`e4-repeatable-jobs-and-rate-limiting`)

- [ ] 7.1 Implement `Queue::upsert_job_scheduler`, `remove_job_scheduler`, `get_job_schedulers`; verify the interop
  scenarios of the `queues/schedulers` spec (Corvis creates, Node runs; upsert replaces)
- [ ] 7.2 Record the cron fixture from Node (`crates/corvis/tests/fixtures/bullmq_cron.oracle.mjs` writing
  `bullmq_cron.json`: patterns × time zones × next 10 runs from a fixed instant) and compare with the crate's
  computation; verify the test, and document or reject every differing pattern
- [ ] 7.3 Worker `limiter` and queue global rate limit / global concurrency setters; verify the shared-limit
  scenario with one Node and one Corvis worker, and the global-concurrency scenario
- [ ] 7.4 Manual rate limit from a processor (`JobError::rate_limited`); verify the job returns to `wait` without
  `atm` changing

## 8. #55 Flows (FlowProducer) (`e4-flows-flowproducer`)

- [ ] 8.1 Implement `FlowProducer` (wrapping the crate's) and `#[inject_flow_producer]`; verify a flow across two
  queues with Node workers completes children first and the parent reads their values
- [ ] 8.2 Expose `Job::children_values()` and the child failure options with Node's abbreviated keys; verify the
  Node flow → Corvis workers scenario and the `fail_parent_on_failure` scenario against the reference

## 9. #56 (part 2) Full matrix and CI, #57 docs and example

- [ ] 9.1 Run the whole matrix against `bullmq5` for the 5.x subset (no schedulers, no pause/resume); verify it passes
  and the excluded scenarios are listed in the test file
- [ ] 9.2 Add the CI job (after `e0-ci-in-github-actions` lands): Linux, `redis:7` service bound to
  `127.0.0.1:56379`, Node 22, `npm ci` in the harness folder, `cargo test -p corvis --features queues --test
  bullmq_interop` with `CORVIS_BULLMQ_REQUIRED=1`; verify on a pull request that the log shows the interop tests
  running, not skipped
- [ ] 9.3 (#57) Rewrite `docs/application/queues.md` (drop `status: unavailable`): setup, producer, processor,
  events, schedulers, rate limits, flows, compatibility section (BullMQ 6.x, 5.x subset, documented divergences,
  mixed-version pause warning), and a migration table from the Node decorators (`@Processor`, `@InjectQueue`, `@OnWorkerEvent`,
  `@OnQueueEvent`) to Corvis attributes, without naming the Node framework (naming rule); verify every snippet matches
  the final API
- [ ] 9.4 (#57) Translate it in `docs/es/application/queues.md`; update `docs/application/mail.md` and other pages
  that point to the old alternative; verify `npm run docs:check` in `docs/`
- [ ] 9.5 (#57) Add `examples/queues` (producer endpoint + processor + listener, Redis on 127.0.0.1 via
  `REDIS_URL`) and a README with a `docker run -p 127.0.0.1:6379:6379 redis:7` line; verify it runs end to end and
  that a Node script adding a job to it is processed

## 10. Definition of done

Applies to each follow-up change (#50–#57); this spike only needs 10.5 and 10.6.

- [ ] 10.1 `cargo build --workspace --all-targets` with no warnings, with default features and with
  `--features corvis/queues`
- [ ] 10.2 `cargo clippy --workspace --all-targets -- -D warnings` passes (also with `--features corvis/queues`)
- [ ] 10.3 `cargo test --workspace` passes, including the follow-up's regression tests and its interop scenarios
  (run with Redis and `CORVIS_BULLMQ_REQUIRED=1` before archiving)
- [ ] 10.4 corvis-parity: the three port apps (consi, facten, houseplan) keep 0 apidiff differences whenever framework
  runtime code changed (#50–#55; the feature is off by default, but discovery and lifecycle code is shared)
- [ ] 10.5 Docs updated in English and Spanish (`docs/` and `docs/es/`) when the change is user-facing; `npm run
  docs:check` in `docs/` passes. For this spike: no docs change; `docs/application/queues.md` stays `unavailable`
  until #57
- [ ] 10.6 `openspec validate e4-spike-bullmq-data-format-in-redis` passes

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive` using `--skip-specs` (design.md, Decision 9)
- [ ] Close issue [#49](https://github.com/randygil/corvis/issues/49)
- [ ] Tick #49 in its epic [#48](https://github.com/randygil/corvis/issues/48)
- [ ] Move the card to Done in the Corvis Roadmap project
