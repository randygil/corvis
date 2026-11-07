// Corvis roadmap: phases (milestones), epics (parent issues) and tasks (sub-issues).
// Sizes: S = one session, M = two or three sessions, L = more (split it when starting).

export const phases = [
  { key: 'F0', title: 'Phase 0 — Foundations', description: 'Finish pending work, fix what the docs review found, CI, bilingual docs and route posture before adding features.' },
  { key: 'F1', title: 'Phase 1 — What almost every app uses', description: 'Socket.IO, authentication strategies, BullMQ-compatible queues, mail and file storage.' },
  { key: 'F2', title: 'Phase 2 — Architecture and reliability', description: 'CQRS, reliability patterns and MongoDB.' },
  { key: 'F3', title: 'Phase 3 — Deployment and product', description: 'AWS Lambda, corvis-orm as a standalone product and the migration tool.' },
];

export const areas = {
  infra: 'CI, packaging and publishing', security: 'Security', docs: 'Documentation', core: 'DI, modules and application core',
  http: 'HTTP and routing', ws: 'WebSockets and Socket.IO', auth: 'Authentication', queues: 'Queues', mail: 'Mail',
  storage: 'File storage', cqrs: 'CQRS', reliability: 'Reliability patterns', mongodb: 'MongoDB', lambda: 'AWS Lambda',
  orm: 'corvis-orm', migration: 'Migration tool', cli: 'CLI',
};

export const dod = [
  'Builds with no warnings; `cargo clippy` and `cargo test --workspace` pass.',
  'Regression tests for the new behavior, plus a differential test against the original Node library when there is one.',
  'Documented in `docs/` (English, with the Spanish translation) and, for user-facing features, an example.',
  'The three port apps keep 0 differences in apidiff.',
];

const t = (title, size, body, extra = {}) => ({ title, size, body, ...extra });

export const epics = [
  {
    key: 'E0', phase: 'F0', area: 'infra', priority: 'high',
    title: 'Foundations for publishing',
    goal: 'Close what the security audit left open and get the project ready to grow: fuzzing, CI, rename, English README, bilingual docs, typed API and publishable crates.',
    tasks: [
      t('Finish fuzzing: serve_static end to end and ORM filters', 'S', 'There are 13 proptest properties already (JSON, qs, body parser, multipart, JWT, router, trust proxy). Missing: `serve_static` (no path reads outside the root) and corvis-orm `where` (no filter produces an SQL syntax error).\n\n- [ ] `tests/fuzz_static.rs`\n- [ ] ORM filter fuzzing on in-memory SQLite\n- [ ] long run with `PROPTEST_CASES=100000`', { area: 'security' }),
      t('English README', 'S', 'Rewrite the root README in English and fix the examples the docs review found broken: `#[global]` goes below `#[module]`, `register_as` lives in `corvis::config`, `ServeStaticModule::for_root` takes a `Vec`, `#[on_event("cat.*")]` needs wildcards enabled, and the remaining brand mentions.', { area: 'docs' }),
      t('Bilingual documentation site (English primary, Spanish translation)', 'L', 'The VitePress site in `docs/` (173 pages, same section structure as the reference docs, own design) is written in Spanish. Make English the primary locale and keep Spanish as a translation (VitePress i18n), with a language switcher.', { area: 'docs' }),
      t('Publish the documentation site', 'S', 'Build in CI and deploy to GitHub Pages (or similar) once the repo is public; check every page against the code before publishing.', { area: 'docs' }),
      t('CI in GitHub Actions', 'M', 'Build, tests, `clippy -D warnings`, `fmt --check`, `cargo deny check`, `cargo audit` and a short fuzzing run on every PR, on Linux and Windows.'),
      t('Rename the folder nestrs → corvis', 'S', 'Run `corvis-tests/tools/rename-nestrs-to-corvis.ps1` with the session closed, update the `path =` entries of the ports and move the project memory.'),
      t('Decide compatible vs strict mode', 'S', 'Corvis copies Express quirks today: exceeding the multipart nesting limit answers 500, and the HTTP parser accepts bare LF and leading zeros in chunk sizes (no smuggling found). Decide whether to add a strict mode (default or opt-in) and document it.', { kind: 'spike', area: 'security' }),
      t('Typed API as the main path', 'M', 'Examples and docs should use typed DTOs and models instead of `serde_json::Value`. Find what is missing in the API so the typed path is just as convenient (responses, typed `find_many`, errors).', { area: 'http' }),
      t('Crate packaging', 'S', 'LICENSE and THIRD_PARTY_NOTICES inside every crate, Cargo metadata (description, repository, keywords, categories, rust-version) and a clean `cargo package` for all of them.'),
      t('Reserve the names on crates.io', 'S', 'Publish 0.0.x of `corvis`, `corvis-orm`, `corvis-macros`, `corvis-cli`, `corvis-swagger` (and their macros). Public and irreversible: only with explicit approval.', { kind: 'decision' }),
      t('Smaller and faster dev builds', 'S', 'The shared target dir reached 40 GB. Measure the effect of `debug = "line-tables-only"` in the dev profile, no debug info for dependencies, and `rust-lld` as the Windows linker on build time, link time and target size; keep what helps.'),
      t('Crate documentation MCP for agents', 'S', 'When phase 1 starts (socketioxide, lettre, AWS SDK...), evaluate and install an MCP server that serves current docs.rs documentation (rust-docs-mcp, mcp-rust-docs) so agents stop relying on stale crate APIs.', { kind: 'decision' }),
      t('Reproducible benchmark: port vs original', 'M', 'Measure memory, latency (p50/p99) and throughput of one port against its Node original under the same load. Reproducible script in `bench/` and results in the docs.'),
    ],
  },
  {
    key: 'E13', phase: 'F0', area: 'core', priority: 'high',
    title: 'Core fixes from the documentation review',
    goal: 'The docs review compared every documented API with the code and found APIs that do nothing, untested DI features, confusing defaults and missing pieces users expect. Fix them before adding features.',
    tasks: [
      t('DI: misleading messages and no-op attributes', 'S', '- The circular dependency error suggests `forwardRef()`, which does not exist (`di/error.rs`).\n- The `Scope` docs mention a `Request` scope that does not exist (`di/provider.rs`).\n- `#[optional]` is accepted and does nothing.\n- `Provider::use_class_as` loses lifecycle hooks.'),
      t('DI test coverage', 'M', 'Tests for transient scope, `ModuleRef`, `use_existing`, `use_class_as`, cycle detection, global modules and user-defined dynamic modules.'),
      t('Standalone application context runs events and scheduled jobs', 'S', '`CorvisFactory::create_application_context` does not register `#[on_event]`, `#[cron]`, `#[interval]` or `#[timeout]` handlers (only `init()` does), so a standalone worker never runs its crons.'),
      t('Enhancers and routing edge cases', 'M', '- `override_guard` in tests does not cover global enhancers (overriding an `app_*` provider drops the global guard).\n- Several `Provider::app_filter` run in reverse registration order.\n- `set_global_prefix_with_exclude` only matches the controller path.\n- With URI versioning, `for_routes("cats")` does not match `/v1/cats`.\n- An optional param `:id?` loses the `?` and becomes required.', { area: 'http' }),
      t('Consistent units and options that do nothing', 'S', '- `Cookie::max_age` uses seconds and `CookieOptions::max_age` milliseconds.\n- `ValidationPipeOptions::transform` is accepted and does nothing.', { area: 'http' }),
      t('Honest health checks', 'S', '`HttpHealthIndicator::ping_check` only opens a TCP connection, `check_rss` only measures on Linux, and indicators have no timeout.'),
      t('CLI defaults that contradict the docs', 'S', '- `corvis build` builds debug unless `--release` (the README says otherwise).\n- With `--path`, the generator looks for `<dir>/<name>_module.rs` and registers the controller in `AppModule`.\n- `start --watch` needs `cargo-watch` installed: detect and explain.', { area: 'cli' }),
      t('Fix the examples', 'S', '- `cats-app` falls back to the JWT secret `"change-me"`.\n- `blog-orm` uses `z::coerce::boolean()` for `?published=false`, which yields `true`.\n- No example uses `corvis-swagger` or `#[derive(ApiSchema)]`.', { area: 'docs' }),
      t('Remove the remaining brand mentions', 'S', 'Code comments (`http/sse.rs`, `jwt.rs`, `modules/serve_static.rs`), crate descriptions (`Cargo.toml`, `lib.rs`), `bench/results.md` title, and decide the default `prisma/` folder and `_prisma_migrations` table (kept for compatibility, configurable?).'),
      t('Missing core features users expect', 'L', 'REQUEST scope, exception filters and global guards on gateways, TLS, compression, async variants (`for_root_async`) for Config, Cache and Throttler, and `Retry-After` / `X-RateLimit-*` headers in the throttler. Split into separate changes when starting.', { kind: 'spike' }),
    ],
  },
  {
    key: 'E1', phase: 'F0', area: 'security', priority: 'high',
    title: 'Route posture: no route left unprotected by mistake',
    goal: 'The application refuses to start when a route has no guard and is not explicitly marked public, turning a silent mistake into a startup error.',
    tasks: [
      t('Route posture design', 'S', 'Define `#[public]`, the `require_route_posture()` application option, how global, controller and method guards count, and what happens with middleware, `serve_static`, gateways and SSE.', { kind: 'spike' }),
      t('`#[public]` metadata and startup check', 'M', 'On `listen`/`init`, walk the route table and fail with the list of routes that have neither a guard nor `#[public]`.'),
      t('Route posture for WebSockets and SSE', 'S', 'The same check for gateway handlers and SSE endpoints.'),
      t('Posture report in the CLI', 'S', 'A command that lists every route with its guards (or `public`) for code review.', { area: 'cli' }),
      t('Route posture docs and example', 'S', 'A Security page, and enable it in the `tertulia` example.', { area: 'docs' }),
    ],
  },
  {
    key: 'E2', phase: 'F1', area: 'ws', priority: 'high',
    title: 'Socket.IO',
    goal: 'Gateways compatible with socket.io (the default WebSocket adapter of the original model), so the official `socket.io-client` behaves the same against Corvis.',
    tasks: [
      t('Spike: Engine.IO v4 / Socket.IO v5 in Rust', 'S', 'Evaluate `socketioxide` against an own implementation (parity, maintenance, integration with the Corvis pipeline). Set up the oracle: Node socket.io server + official client.', { kind: 'spike' }),
      t('Engine.IO: handshake, polling, upgrade and heartbeat', 'L', 'HTTP long-polling transport, upgrade to WebSocket, ping/pong, packet framing and size limits.'),
      t('Socket.IO: namespaces, events and acks', 'M', 'Namespaces, emit/on, acknowledgements and binary attachments.'),
      t('Rooms and broadcast', 'M', 'join/leave, `to`, `except`, broadcast per namespace and per room.'),
      t('IoAdapter for gateways', 'M', 'socket.io in `#[websocket_gateway]` with namespace and CORS, `#[subscribe_message]`, `#[connected_socket]`, `#[message_body]`, `WsException`, and guards, pipes and interceptors on WS.'),
      t('Redis adapter for multiple instances', 'M', 'Compatible with `@socket.io/redis-adapter` to scale horizontally.'),
      t('Differential tests against Node socket.io', 'M', 'The same scenario with the official client against both servers: events, acks, rooms, disconnects and errors.'),
      t('Socket.IO docs and example', 'S', 'Extended WebSockets page, and move `tertulia` (or a new example) to socket.io.', { area: 'docs' }),
    ],
  },
  {
    key: 'E3', phase: 'F1', area: 'auth', priority: 'high',
    title: 'Authentication strategies (Passport style)',
    goal: 'The strategy model almost every app uses: `AuthGuard("jwt")`, strategies with `validate()`, local, JWT, OAuth2 and API keys.',
    tasks: [
      t('Strategy module design', 'S', 'Equivalent of `PassportStrategy` and `AuthGuard(name)`: named strategy registry, `validate()`, `req.user`, chained strategies and 401 errors identical to the original.', { kind: 'spike' }),
      t('Local strategy and password hashing', 'M', 'Username/password with bcrypt (compatible with bcryptjs hashes) and argon2; constant-time comparison.'),
      t('JWT strategy', 'M', 'Parity with passport-jwt: extractors (header, cookie, query), `ignoreExpiration`, `secretOrKeyProvider`, issuer/audience.'),
      t('Generic OAuth2, Google and GitHub', 'L', 'Authorization code with `state` and PKCE, refresh tokens and normalized profiles.'),
      t('Sessions and signed cookies', 'M', 'Sessions compatible with express-session (memory, Redis and ORM stores) for the strategies that use them.'),
      t('API key strategy and custom strategies', 'S', 'API key in header or query, and how to write a new strategy.'),
      t('Authentication security review', 'S', 'OAuth CSRF, session fixation, timing, user enumeration and login rate limiting.', { area: 'security' }),
      t('Authentication docs and example', 'S', 'Authentication and Authorization pages with an example of each strategy.', { area: 'docs' }),
    ],
  },
  {
    key: 'E4', phase: 'F1', area: 'queues', priority: 'high',
    title: 'BullMQ-compatible queues',
    goal: 'Redis queues that interoperate with Node BullMQ: a Node producer can enqueue jobs that Corvis processes and vice versa. Key to migrating services one at a time.',
    tasks: [
      t('Spike: BullMQ data format in Redis', 'M', 'Keys, states, Lua scripts and BullMQ versions. Decide whether to reuse its Lua scripts (MIT) to guarantee interoperability.', { kind: 'spike' }),
      t('Redis connection and BullModule', 'S', '`BullModule::for_root` / `register_queue`, configurable connection and `#[inject_queue]`.'),
      t('Queue: add, add_bulk and job options', 'M', 'delay, attempts, backoff, priority, jobId, removeOnComplete/removeOnFail.'),
      t('Workers: `#[processor]` and `#[process]`', 'L', 'Concurrency, retries with backoff, stalled jobs, graceful shutdown and progress.'),
      t('Queue events and QueueEvents', 'S', 'completed, failed, progress, with listeners in the same or another process.'),
      t('Repeatable jobs and rate limiting', 'M', 'Cron or interval repetition and jobs-per-time limits.'),
      t('Flows (FlowProducer)', 'M', 'Parent/child jobs and dependencies.'),
      t('Interoperability tests with Node BullMQ', 'M', 'Node produces and Corvis consumes, and the other way round, including retries and failures.'),
      t('Queues docs and example', 'S', 'Queues page and an example with a worker.', { area: 'docs' }),
    ],
  },
  {
    key: 'E5', phase: 'F1', area: 'mail', priority: 'medium',
    title: 'Mail',
    goal: 'Sending email like the usual mailer module: SMTP transport, templates, attachments and a test transport.',
    tasks: [
      t('MailerModule with SMTP transport', 'M', 'Based on `lettre`: `for_root`/`for_root_async`, TLS, authentication and connection pooling.'),
      t('Templates and attachments', 'M', 'Handlebars templates with layouts and partials, attachments and inline content.'),
      t('Test transport', 'S', 'In-memory capture for tests and a log-only mode for development.'),
      t('Deferred sending through a queue', 'S', 'Optional integration with queues (E4) for retries.', { depends: 'E4' }),
      t('Mail docs and example', 'S', '', { area: 'docs' }),
    ],
  },
  {
    key: 'E6', phase: 'F1', area: 'storage', priority: 'medium',
    title: 'File storage',
    goal: 'Store and serve files on local disk or S3-compatible storage (AWS, MinIO, Cloudflare R2), integrated with multer.',
    tasks: [
      t('StorageService with local disk and S3', 'M', 'Common API (put, streaming get, delete, exists, list) with local and S3-compatible implementations.'),
      t('S3 storage engine for multer', 'M', 'Direct uploads to S3 from `FileInterceptor`, with parity with multer-s3.'),
      t('Signed URLs and streaming downloads', 'S', 'Presigned upload/download URLs and downloads with `StreamableFile`.'),
      t('Tests against MinIO', 'S', 'Integration tests with MinIO in Docker.'),
      t('File storage docs and example', 'S', '', { area: 'docs' }),
    ],
  },
  {
    key: 'E7', phase: 'F2', area: 'cqrs', priority: 'medium',
    title: 'CQRS',
    goal: 'The CQRS module: command, query and event buses, aggregate roots and sagas, with the API of the original module.',
    tasks: [
      t('CommandBus, QueryBus and EventBus', 'M', '`CqrsModule` with `#[command_handler]`, `#[query_handler]` and `#[events_handler]`, resolved by the DI container.'),
      t('AggregateRoot and EventPublisher', 'M', 'Events accumulated in the aggregate, `commit()` and `merge_object_context`.'),
      t('Sagas', 'M', 'Event streams that dispatch commands (streams with basic operators).'),
      t('Parity with the original module', 'S', 'Port its behavior tests (handler order, errors, unhandled events).'),
      t('CQRS docs and example', 'S', '', { area: 'docs' }),
    ],
  },
  {
    key: 'E8', phase: 'F2', area: 'reliability', priority: 'medium',
    title: 'Reliability patterns',
    goal: 'Building blocks for services that move money or critical data: idempotency, outbox, locks, retries, circuit breaker and signed webhooks.',
    tasks: [
      t('Idempotency', 'M', 'Interceptor keyed by `Idempotency-Key`, memory/Redis/ORM storage, and the same response replayed for the same key (generalize the one in the consi port).'),
      t('Transactional outbox', 'M', 'Record events in the same corvis-orm transaction and publish them afterwards (queue or webhook) with at-least-once delivery.'),
      t('Distributed locks', 'S', 'Locks on Redis and on Postgres advisory locks, with expiry and renewal.'),
      t('Retries with backoff and circuit breaker', 'M', 'For outgoing calls (HTTP and others): exponential backoff with jitter, timeouts and circuit breaker.'),
      t('Signed outgoing webhooks', 'M', 'HMAC signature, retries, delivery log and manual redelivery (generalize the consi pattern).'),
      t('Reliability patterns docs', 'S', '', { area: 'docs' }),
    ],
  },
  {
    key: 'E9', phase: 'F2', area: 'mongodb', priority: 'medium',
    title: 'MongoDB',
    goal: 'MongoDB support for apps that use it, with the API most common in the source ecosystem.',
    tasks: [
      t('Spike: Mongoose-style module or MongoDB connector for corvis-orm', 'S', 'Choose between a module with the Mongoose API (`for_root`, `for_feature`, `#[inject_model]`), a MongoDB connector for corvis-orm (the schema format also supports MongoDB), or both, based on what the apps to migrate use.', { kind: 'spike' }),
      t('Connection and model registration', 'M', '`for_root`/`for_root_async`, multiple connections and per-module model registration.'),
      t('Schemas, validation and hooks', 'M', 'Schema definition, validation, defaults, timestamps and pre/post hooks.'),
      t('Queries, populate and transactions', 'L', 'Filters, projections, sorting, pagination, populate and transactions.'),
      t('MongoDB differential tests', 'M', 'The same scenario against the Node library and against Corvis, comparing results and the final database state.'),
      t('MongoDB docs and example', 'S', '', { area: 'docs' }),
    ],
  },
  {
    key: 'E10', phase: 'F3', area: 'lambda', priority: 'low',
    title: 'Deployment on AWS Lambda',
    goal: 'Run a Corvis app on AWS Lambda behind API Gateway or Function URLs, with a low cold start.',
    tasks: [
      t('HTTP adapter for Lambda', 'M', 'API Gateway REST (v1), HTTP API (v2) and Function URLs with `lambda_http`, without changing app code.'),
      t('Cold start', 'S', 'Initialize the app once per container and measure the cold start.'),
      t('Non-HTTP events', 'M', 'Handlers for SQS, EventBridge (scheduled tasks) and S3.'),
      t('Deployment template', 'S', 'Guide with `cargo lambda` and a SAM or CDK template.'),
      t('Lambda deployment docs', 'S', '', { area: 'docs' }),
    ],
  },
  {
    key: 'E11', phase: 'F3', area: 'orm', priority: 'medium',
    title: 'corvis-orm as a standalone product',
    goal: 'Publish corvis-orm as "the Prisma Client API for Rust", usable without the rest of Corvis. It fills the gap prisma-client-rust left when it was archived.',
    tasks: [
      t('Decouple corvis-orm from corvis', 'M', 'Make the Corvis integration an optional feature, so the crate builds and works on its own.'),
      t('README and compatibility table', 'S', 'Which parts of the Prisma Client API are covered and how they were verified.'),
      t('Generator and migrations documented', 'S', 'From `schema.prisma` to Rust models, and the migration workflow.'),
      t('Example with Axum, without Corvis', 'S', ''),
      t('Integrations with other frameworks', 'S', 'Offer small integrations (for example, a crate for another NestJS-style framework) to gain visibility.', { kind: 'decision' }),
    ],
  },
  {
    key: 'E12', phase: 'F3', area: 'migration', priority: 'medium',
    title: 'Migration tool',
    goal: 'Turn the method of the three ports (apidiff: the same requests to both backends, comparing responses and the database state) into a tool and a guide for teams migrating their backend.',
    tasks: [
      t('Extract apidiff as a publishable tool', 'M', 'Move it out of `corvis-tests`, document its options and scenarios, and package it.'),
      t('Step-by-step migration guide', 'M', 'From the original backend to Corvis with the ports method: route inventory, module-by-module port and verification with apidiff.', { area: 'docs' }),
      t('Case study with benchmark', 'S', 'One of the ports with its numbers (parity, memory and latency) as a published case.', { depends: 'E0' }),
    ],
  },
];
