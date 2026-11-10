# Corvis roadmap

Corvis aims to be the way to move Node.js backends built on the NestJS + Prisma model to Rust **with verified behavioral parity**: the same API, the same database effects, a fraction of the resources. This roadmap is the big plan; the work happens in small, verifiable steps.

## How work is organized

- **Order of work:** the phases of [#111](https://github.com/randygil/corvis/issues/111) set which issues are worked next; each phase is merged before the next one starts.
- **Phases** are GitHub milestones. **Epics** are parent issues; their **tasks** are sub-issues, each sized to a few sessions at most.
- When a task starts, it becomes an **OpenSpec change** (`openspec/changes/<change>/`): `/opsx:propose` writes the proposal, spec deltas, design and task list; after review, `/opsx:apply` implements it and `/opsx:archive` merges the specs into `openspec/specs/`. The change name is shown in each issue.
- Features with an original Node library are verified with a differential test against it (same inputs to both, compare outputs and state), like the three ports verified with apidiff.
- Sizes: **S** about one session, **M** two or three, **L** more (split it when starting).

## Definition of done

- Builds with no warnings; `cargo clippy` and `cargo test --workspace` pass.
- Regression tests for the new behavior, plus a differential test against the original Node library when there is one.
- Documented in `docs/` (English, with the Spanish translation) and, for user-facing features, an example.
- The three port apps keep 0 differences in apidiff.

## Phase 0 — Foundations

Finish pending work, fix what the docs review found, CI, bilingual docs and route posture before adding features.

### E0 · Foundations for publishing — #1

Close what the security audit left open and get the project ready to grow: fuzzing, CI, rename, English README, bilingual docs, typed API and publishable crates.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #2 | Finish fuzzing: serve_static end to end and ORM filters | S | `e0-finish-fuzzing-serve-static-end-to-end-and-orm-filters` |
| #3 | English README | S | `e0-english-readme` |
| #4 | Bilingual documentation site (English primary, Spanish translation) | L | `e0-bilingual-documentation-site-english-primary-spanish` |
| #5 | Publish the documentation site | S | `e0-publish-the-documentation-site` |
| #6 | CI in GitHub Actions | M | `e0-ci-in-github-actions` |
| #7 | Rename the folder nestrs → corvis | S | `e0-rename-the-folder-nestrs-corvis` |
| #8 | Decide compatible vs strict mode _(spike)_ | S | `e0-decide-compatible-vs-strict-mode` |
| #9 | Typed API as the main path | M | `e0-typed-api-as-the-main-path` |
| #10 | Crate packaging | S | `e0-crate-packaging` |
| #11 | Reserve the names on crates.io _(decision)_ | S | `e0-reserve-the-names-on-crates-io` |
| #12 | Reproducible benchmark: port vs original | M | `e0-reproducible-benchmark-port-vs-original` |
| #106 | Smaller and faster dev builds | S | `e0-smaller-and-faster-dev-builds` |
| #107 | Crate documentation MCP for agents _(decision)_ | S | `e0-crate-documentation-mcp-for-agents` |

### E13 · Core fixes from the documentation review — #13

The docs review compared every documented API with the code and found APIs that do nothing, untested DI features, confusing defaults and missing pieces users expect. Fix them before adding features.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #14 | DI: misleading messages and no-op attributes | S | `e13-di-misleading-messages-and-no-op-attributes` |
| #15 | DI test coverage | M | `e13-di-test-coverage` |
| #16 | Standalone application context runs events and scheduled jobs | S | `e13-standalone-application-context-runs-events-and-scheduled` |
| #17 | Enhancers and routing edge cases | M | `e13-enhancers-and-routing-edge-cases` |
| #18 | Consistent units and options that do nothing | S | `e13-consistent-units-and-options-that-do-nothing` |
| #19 | Honest health checks | S | `e13-honest-health-checks` |
| #20 | CLI defaults that contradict the docs | S | `e13-cli-defaults-that-contradict-the-docs` |
| #21 | Fix the examples | S | `e13-fix-the-examples` |
| #22 | Remove the remaining brand mentions | S | `e13-remove-the-remaining-brand-mentions` |
| #23 | Missing core features users expect _(spike)_ | L | `e13-missing-core-features-users-expect` |

### E1 · Route posture: no route left unprotected by mistake — #24

The application refuses to start when a route has no guard and is not explicitly marked public, turning a silent mistake into a startup error.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #25 | Route posture design _(spike)_ | S | `e1-route-posture-design` |
| #26 | `#[public]` metadata and startup check | M | `e1-public-metadata-and-startup-check` |
| #27 | Route posture for WebSockets and SSE | S | `e1-route-posture-for-websockets-and-sse` |
| #28 | Posture report in the CLI | S | `e1-posture-report-in-the-cli` |
| #29 | Route posture docs and example | S | `e1-route-posture-docs-and-example` |

## Phase 1 — What almost every app uses

Socket.IO, authentication strategies, BullMQ-compatible queues, mail and file storage.

### E2 · Socket.IO — #30

Gateways compatible with socket.io (the default WebSocket adapter of the original model), so the official `socket.io-client` behaves the same against Corvis.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #31 | Spike: Engine.IO v4 / Socket.IO v5 in Rust _(spike)_ | S | `e2-spike-engine-io-v4-socket-io-v5-in-rust` |
| #32 | Engine.IO: handshake, polling, upgrade and heartbeat | L | `e2-engine-io-handshake-polling-upgrade-and-heartbeat` |
| #33 | Socket.IO: namespaces, events and acks | M | `e2-socket-io-namespaces-events-and-acks` |
| #34 | Rooms and broadcast | M | `e2-socket-io-rooms-and-broadcast` |
| #35 | IoAdapter for gateways | M | `e2-ioadapter-for-gateways` |
| #36 | Redis adapter for multiple instances | M | `e2-redis-adapter-for-multiple-instances` |
| #37 | Differential tests against Node socket.io | M | `e2-differential-tests-against-node-socket-io` |
| #38 | Socket.IO docs and example | S | `e2-socket-io-docs-and-example` |

### E3 · Authentication strategies (Passport style) — #39

The strategy model almost every app uses: `AuthGuard("jwt")`, strategies with `validate()`, local, JWT, OAuth2 and API keys.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #40 | Strategy module design _(spike)_ | S | `e3-strategy-module-design` |
| #41 | Local strategy and password hashing | M | `e3-local-strategy-and-password-hashing` |
| #42 | JWT strategy | M | `e3-jwt-strategy` |
| #43 | Generic OAuth2, Google and GitHub | L | `e3-oauth2-strategies` |
| #44 | Sessions and signed cookies | M | `e3-sessions-and-signed-cookies` |
| #45 | API key strategy and custom strategies | S | `e3-api-key-strategy-and-custom-strategies` |
| #46 | Authentication security review | S | `e3-authentication-security-review` |
| #47 | Authentication docs and example | S | `e3-authentication-docs-and-example` |

### E4 · BullMQ-compatible queues — #48

Redis queues that interoperate with Node BullMQ: a Node producer can enqueue jobs that Corvis processes and vice versa. Key to migrating services one at a time.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #49 | Spike: BullMQ data format in Redis _(spike)_ | M | `e4-spike-bullmq-data-format-in-redis` |
| #50 | Redis connection and BullModule | S | `e4-redis-connection-and-bullmodule` |
| #51 | Queue: add, add_bulk and job options | M | `e4-queue-add-add-bulk-and-job-options` |
| #52 | Workers: `#[processor]` and `#[process]` | L | `e4-workers-processor-and-process` |
| #53 | Queue events and QueueEvents | S | `e4-queue-events-and-queueevents` |
| #54 | Repeatable jobs and rate limiting | M | `e4-repeatable-jobs-and-rate-limiting` |
| #55 | Flows (FlowProducer) | M | `e4-flows-flowproducer` |
| #56 | Interoperability tests with Node BullMQ | M | `e4-interoperability-tests-with-node-bullmq` |
| #57 | Queues docs and example | S | `e4-queues-docs-and-example` |

### E5 · Mail — #58

Sending email like the usual mailer module: SMTP transport, templates, attachments and a test transport.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #59 | MailerModule with SMTP transport | M | `e5-mailermodule-with-smtp-transport` |
| #60 | Templates and attachments | M | `e5-templates-and-attachments` |
| #61 | Test transport | S | `e5-test-transport` |
| #62 | Deferred sending through a queue | S | `e5-deferred-sending-through-a-queue` |
| #63 | Mail docs and example | S | `e5-mail-docs-and-example` |

### E6 · File storage — #64

Store and serve files on local disk or S3-compatible storage (AWS, MinIO, Cloudflare R2), integrated with multer.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #65 | StorageService with local disk and S3 | M | `e6-storageservice-with-local-disk-and-s3` |
| #66 | S3 storage engine for multer | M | `e6-s3-storage-engine-for-multer` |
| #67 | Signed URLs and streaming downloads | S | `e6-signed-urls-and-streaming-downloads` |
| #68 | Tests against MinIO | S | `e6-tests-against-minio` |
| #69 | File storage docs and example | S | `e6-file-storage-docs-and-example` |

## Phase 2 — Architecture and reliability

CQRS, reliability patterns and MongoDB.

### E7 · CQRS — #70

The CQRS module: command, query and event buses, aggregate roots and sagas, with the API of the original module.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #71 | CommandBus, QueryBus and EventBus | M | `e7-commandbus-querybus-and-eventbus` |
| #72 | AggregateRoot and EventPublisher | M | `e7-aggregateroot-and-eventpublisher` |
| #73 | Sagas | M | `e7-sagas` |
| #74 | Parity with the original module | S | `e7-parity-with-the-original-module` |
| #75 | CQRS docs and example | S | `e7-cqrs-docs-and-example` |

### E8 · Reliability patterns — #76

Building blocks for services that move money or critical data: idempotency, outbox, locks, retries, circuit breaker and signed webhooks.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #77 | Idempotency | M | `e8-idempotency` |
| #78 | Transactional outbox | M | `e8-transactional-outbox` |
| #79 | Distributed locks | S | `e8-distributed-locks` |
| #80 | Retries with backoff and circuit breaker | M | `e8-retries-with-backoff-and-circuit-breaker` |
| #81 | Signed outgoing webhooks | M | `e8-signed-outgoing-webhooks` |
| #82 | Reliability patterns docs | S | `e8-reliability-patterns-docs` |

### E9 · MongoDB — #83

MongoDB support for apps that use it, with the API most common in the source ecosystem.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #84 | Spike: Mongoose-style module or MongoDB connector for corvis-orm _(spike)_ | S | `e9-spike-mongoose-style-module-or-mongodb-connector-for` |
| #85 | Connection and model registration | M | `e9-connection-and-model-registration` |
| #86 | Schemas, validation and hooks | M | `e9-schemas-validation-and-hooks` |
| #87 | Queries, populate and transactions | L | `e9-queries-populate-and-transactions` |
| #88 | MongoDB differential tests | M | `e9-mongodb-differential-tests` |
| #89 | MongoDB docs and example | S | `e9-mongodb-docs-and-example` |

## Phase 3 — Deployment and product

AWS Lambda, corvis-orm as a standalone product and the migration tool.

### E10 · Deployment on AWS Lambda — #90

Run a Corvis app on AWS Lambda behind API Gateway or Function URLs, with a low cold start.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #91 | HTTP adapter for Lambda | M | `e10-http-adapter-for-lambda` |
| #92 | Cold start | S | `e10-cold-start` |
| #93 | Non-HTTP events | M | `e10-non-http-events` |
| #94 | Deployment template | S | `e10-deployment-template` |
| #95 | Lambda deployment docs | S | `e10-lambda-deployment-docs` |

### E11 · corvis-orm as a standalone product — #96

Publish corvis-orm as "the Prisma Client API for Rust", usable without the rest of Corvis. It fills the gap prisma-client-rust left when it was archived.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #97 | Decouple corvis-orm from corvis | M | `e11-decouple-corvis-orm-from-corvis` |
| #98 | README and compatibility table | S | `e11-readme-and-compatibility-table` |
| #99 | Generator and migrations documented | S | `e11-generator-and-migrations-documented` |
| #100 | Example with Axum, without Corvis | S | `e11-example-with-axum-without-corvis` |
| #101 | Integrations with other frameworks _(decision)_ | S | `e11-integrations-with-other-frameworks` |

### E12 · Migration tool — #102

Turn the method of the three ports (apidiff: the same requests to both backends, comparing responses and the database state) into a tool and a guide for teams migrating their backend.

| Issue | Task | Size | OpenSpec change |
|---|---|---|---|
| #103 | Extract apidiff as a publishable tool | M | `e12-extract-apidiff-as-a-publishable-tool` |
| #104 | Step-by-step migration guide | M | `e12-step-by-step-migration-guide` |
| #105 | Case study with benchmark | S | `e12-case-study-with-benchmark` |
