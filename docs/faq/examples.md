---
status: documented
---

# Examples

The repository includes three complete applications in `examples/`. They are part of the Cargo
workspace, so they run from the repository root with `cargo run -p <name>`.

## `cats-app`: the full tour

```bash
cargo run -p cats-app        # http://127.0.0.1:3000/api/v1
```

A cats API that uses almost every piece of the framework in a single project:

- Modules with `imports`/`exports` (`AuthModule`, `UsersModule`, `CatsModule`) and included modules:
  `ConfigModule`, `EventEmitterModule`, `ScheduleModule`, `CacheModule`, `ThrottlerModule` and
  `TerminusModule`.
- JWT authentication with a global `AuthGuard`, public routes with `#[set_metadata(IS_PUBLIC_KEY, true)]`
  and a global `RolesGuard` (see [Authentication](/security/authentication) and
  [Authorization](/security/authorization)).
- Global `ValidationPipe` with `#[derive(Validate)]` DTOs, a custom exception filter,
  timing and transformation interceptors, and a logging middleware applied with
  `MiddlewareConsumer`.
- URI versioning (`/api/v1/cats/:id` and `/api/v2/cats/:id`), a `VERSION_NEUTRAL` endpoint,
  caching with `CacheInterceptor`, SSE at `/api/v1/cats/stream`, events with `#[on_event]`, tasks with
  `#[cron]` and `#[timeout]`, health checks and a WebSocket gateway at `/chat`.

The environment variables are in `examples/cats-app/.env.example` (`PORT`, `JWT_SECRET`,
`JWT_EXPIRES_IN`). The test users are in `src/users/users_service.rs`: `john`
(administrator) and `maria`.

```bash
curl -X POST http://127.0.0.1:3000/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"john","password":"changeme"}'
```

::: warning
If you do not define `JWT_SECRET`, `cats-app` signs with the default value `change-me`. It is an example: do not
deploy it like that.
:::

## `blog-orm`: REST API with a database

```bash
cargo run -p blog-orm        # http://127.0.0.1:3000 (DATABASE_URL=file:./dev.db by default)
```

Users and posts on SQLite with [corvis-orm](/data/corvis-orm): pagination, search, relations,
nested writes and statistics. The DTOs are validated with `corvis::z` (`zod_dto!` +
`ZodValidationPipe::auto()`) and `OrmExceptionFilter` translates database errors to `409` or
`404`. `OrmModule::for_root_async` reads the URL from `ConfigService` and syncs the tables at startup.

```bash
curl -X POST http://127.0.0.1:3000/users -H 'content-type: application/json' \
  -d '{"email":"ana@corvis.dev","name":"Ana"}'
curl 'http://127.0.0.1:3000/posts?search=hello&page=1&limit=10'
```

Variables: `DATABASE_URL`, `PORT` and `LOG_QUERIES=true` to print every SQL query.

## `tertulia`: real-time chat

```bash
JWT_SECRET="$(openssl rand -hex 32)" cargo run -p tertulia
# open http://127.0.0.1:5150
```

A chat with rooms, presence and typing indicators, with a bundled web client. It is the reference
example for:

- WebSockets: a `#[websocket_gateway(path = "/chat")]` with rooms, acks, `WsException`, guards,
  zod pipes and per-message interceptors; and a plain WebSocket with `corvis::ws` at `/ws/firehose`.
- JWT authentication on connect (token in `?token=`) and a single `AuthGuard` for HTTP and WebSocket.
- Injection across layers: the REST controller injects the gateway to read who is connected and
  send announcements.
- SSE at `/api/stats/stream`, persistence with corvis-orm and static files with
  `corvis::serve_static`.

`JWT_SECRET` is required: without it the application does not start. The remaining variables and the chat
protocol are in `examples/tertulia/README.md`. The login does not ask for a password, so do not expose it
as is.

## Example tests

`tertulia` has integration tests in `examples/tertulia/tests/` (startup and chat over
WebSocket). To run all the workspace tests:

```bash
cargo test --workspace
```
