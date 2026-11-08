---
status: documented
---

# corvis-orm

`corvis-orm` is the workspace ORM. It is built on `sqlx` 0.8, supports **PostgreSQL** and **SQLite**, and integrates with Corvis dependency injection through `OrmModule`, `OrmService` and `OrmExceptionFilter`. Its client accepts query arguments as JSON (`where`, `include`, `select`, `orderBy`, ...) or through typed helpers generated for each model, and it is compatible with projects that describe their database with a `schema.prisma` file: it can generate the Rust models from it and apply its SQL migrations.

This chapter covers the Corvis integration and the essentials of the client. The full ORM reference (all filters, nested writes, raw SQL, `Decimal`) is in `crates/corvis-orm/README.md`.

## Installation

Add the crate next to `corvis`. The `corvis` feature (enabled by default) includes the dependency injection module, and `tls-rustls` (also a default) enables TLS for Postgres:

```toml
[dependencies]
corvis = { path = "../../crates/corvis" }
corvis-orm = { path = "../../crates/corvis-orm" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

(The paths are the ones used by the repository examples; adjust them to your project.)

## Models

A model is a struct with `#[derive(Model)]`. This is the one for the rooms in `examples/tertulia`:

```rust
use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "rooms")]
pub struct Room {
    #[id]
    pub name: String,
    pub topic: String,
    #[default(50)]
    pub max_members: i64,
    pub created_by: String,
    #[default(now)]
    pub created_at: corvis_orm::DateTime,
}
```

And the relations, from `examples/blog-orm/src/models.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "users")]
pub struct User {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    #[unique]
    pub email: String,
    pub name: Option<String>,
    #[default(now)]
    pub created_at: DateTime<Utc>,
    #[relation(has_many)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<Post>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "posts")]
pub struct Post {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    pub title: String,
    pub content: Option<String>,
    #[default(false)]
    pub published: bool,
    #[default(0)]
    pub views: i64,
    #[default(now)]
    pub created_at: DateTime<Utc>,
    #[updated_at]
    pub updated_at: DateTime<Utc>,
    pub author_id: i64,
    #[relation(belongs_to, fields = "author_id", on_delete = "Cascade")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Box<User>>,
}
```

Field attributes: `#[id]`, `#[unique]`, `#[default(autoincrement | now | uuid | cuid | <literal>)]`, `#[updated_at]`, `#[map("column")]`, `#[db("Uuid")]`, `#[db_type(...)]` and `#[relation(has_many | has_one | belongs_to | many_to_many, fields = "...", references = "...", on_delete = "...", on_update = "...", name = "...")]`. On the struct: `#[model(table = "...", id(fields = [...]), unique(fields = [...]))]`.

Relations are declared as `Option` because they are only loaded when the query includes them: `Option<Vec<T>>` for to-many and `Option<Box<T>>` for to-one. With `skip_serializing_if`, a relation that was not included does not appear in the JSON.

The derive generates three things per model:

1. `impl Model for User` with the metadata.
2. The `UserDelegate` trait, which adds `db.user()` to the client. Import it together with the model: `use crate::models::{User, UserDelegate};`.
3. The `user` module with typed helpers: `user::email::contains(..)`, `user::id::equals(..)`, `user::posts::include()`, ...

### Generate the models from a schema

If you already have a `schema.prisma`, the `corvis-orm` binary writes the models for you:

```bash
cargo run -q -p corvis-orm --bin corvis-orm -- generate --schema db/schema.prisma --out src/models.rs
```

The generated file is not edited by hand: you regenerate it. Without `--schema`, it looks for the schema in the usual places of a `schema.prisma` project first and, if there is none, in `db/schema.prisma` and then `db/schema` (`corvis-orm --help` lists the full order); the migrations folder defaults to `migrations` next to the schema. Options: `--no-docs` (does not copy the `///` comments) and `--derive <Trait>` (an extra derive for each model, can be repeated).

## Register the module

`OrmModule` can be registered in three ways. The most common one reads the URL from `ConfigService` with an async factory, as in `examples/blog-orm`:

```rust
use corvis::prelude::*;
use corvis_orm::{OrmModule, OrmModuleOptions};

#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        OrmModule::for_root_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let url: String = config.get_or("DATABASE_URL", "file:./dev.db".to_string());
            Ok(OrmModuleOptions::new(url)
                .log_queries(config.get_or("LOG_QUERIES", false))
                .push::<(User, Post)>())
        }),
        UsersModule,
        PostsModule,
    ],
)]
pub struct AppModule;
```

| Form | Use |
|---|---|
| `OrmModule::for_root(OrmModuleOptions::new(url).global(true))` | Fixed options |
| `OrmModule::for_root_async(global, \|ctx\| async move { ... })` | Computed options; the factory can resolve other providers |
| `OrmModule::for_root_from_config(global)` | Reads `DATABASE_URL` from `ConfigService` (which must be global) |

The first argument of `for_root_async` and `for_root_from_config` says whether the module is global. With `true`, any module can inject `OrmService` without importing it.

`OrmModuleOptions` options:

| Method | Effect |
|---|---|
| `new(url)` | `file:./dev.db`, `sqlite::memory:` or `postgresql://...` |
| `global(bool)` | Registers the module as global |
| `log_queries(bool)` | Logs every SQL query with its parameters |
| `max_connections(n)` | Pool size (overrides `connection_limit` in the URL) |
| `explicit_connect(bool)` | Connect in `on_module_init` (default `true`) |
| `transaction_options(..)` | Default `maxWait`, `timeout` and `isolationLevel` for transactions |
| `push::<(A, B)>()` | Creates the missing tables and indexes at startup (useful in development and tests) |

`push` does not modify or drop existing columns: use migrations for production.

## Connection pool

The pool is sized and timed from the database URL, the same way the original client does it, so an app that keeps its `DATABASE_URL` keeps its pool:

```text
postgresql://user:pass@host/db?connection_limit=5&pool_timeout=20&connect_timeout=10
file:./dev.db?connection_limit=1
```

| Parameter | Meaning | Default |
|---|---|---|
| `connection_limit` | Maximum number of pooled connections. An integer of at least 1 | `2 * physical CPU cores + 1` |
| `pool_timeout` | Seconds a query waits for a free connection of the pool before it fails with `P2024`. `0` waits without limit | `10` |
| `connect_timeout` | Postgres only. Seconds to open a connection (connect, authentication, the `schema` setup statement and the liveness check of an idle connection) before the query fails with `P1001`. `0` waits without limit | `5` |

- They work for Postgres and for file-backed SQLite (`file:` and `sqlite:`). They are removed from the URL before it reaches the driver; every other parameter (`sslmode`, `application_name`, `options`, ...) is kept as written.
- In-memory SQLite always uses exactly one connection: `connection_limit` is accepted and ignored there. On SQLite, `pool_timeout` is applied only when it is a valid number; an invalid one is ignored, as the original client does.
- A repeated parameter keeps its last value, and values can be percent-encoded.
- On SQLite, `connect_timeout` is removed from the URL and ignored, whatever its value: opening a file does not wait on a network.
- An invalid value (`connection_limit=0`, `connection_limit=abc`, `pool_timeout=-1`, `connect_timeout=abc`) fails when the client is created, with an initialization error `P1013` that names the parameter and shows the value only when it is short and plain (letters, digits, `+ . _ -`), never the rest of the URL. `OrmClientOptions` prints its URL without the password in `Debug` output; a code `acquire_timeout` of `Duration::MAX` means no limit. The original client treats `connection_limit=0` as no limit; corvis-orm requires at least 1.
- The pool size is at most 262143 (the largest value Postgres accepts for its own `max_connections`), in the URL and in code: the driver reserves one slot per connection up front, so a larger value would exhaust memory. `connection_limit=262144` fails with `P1013`, and `max_connections(0)` or `max_connections(n)` above the bound fails when the client is created.
- Values set in code win over the URL: `OrmClientOptions::max_connections(n)` (and `OrmModuleOptions::max_connections(n)`) over `connection_limit`, and `OrmClientOptions::acquire_timeout` over `pool_timeout`. The URL wins over the defaults.
- `pool_timeout` and `acquire_timeout` bound only the wait for a free connection. Opening a connection is bounded by `connect_timeout` instead, so `pool_timeout=0` (wait without limit for a busy pool) never means waiting forever for a server that is down, silent or full: the query fails with `P1001` "Can't reach database server at `host:port`" after `connect_timeout`. The first connection of `connect()` and the checks of `migrate deploy` and `migrate status` are bounded the same way. `connect_timeout=0` removes the bound.
- A connection that cannot be set up because the database rejects the `schema` statement (for example a name the database encoding cannot represent) fails at once with `P1001` and the database's message, instead of retrying until a timeout.
- Differences from the original client: a refused connection is retried inside `connect_timeout` (the original fails at once), so an app that starts next to a database that is still booting keeps working; and every connection failure is reported as `P1001`, not with the original's per-cause codes (`P1000`, `P1003`, `P2037`): a role at its connection limit, which the original reports at once as "Too many database connections opened", fails with `P1001` after `connect_timeout`. A `connect_timeout` that does not fit 64 bits (`99999999999999999999`) is invalid, as in the original; one that fits but is larger than `4294967295` seconds means no limit.

::: warning Changed defaults
Before this change the pool had 10 connections on Postgres and 5 on SQLite files, and a query waited up to 30 seconds for a connection. The defaults are now `2 * physical cores + 1` connections and 10 seconds. Set `connection_limit` and `pool_timeout` in the URL (or `max_connections` in code) to restore any previous value. With several connections on a SQLite file, writers can contend for the lock; use `connection_limit=1` when your app needs a single writer.
:::

::: warning Changed: `acquire_timeout` no longer bounds connecting
A program that set `acquire_timeout` (or `pool_timeout`) to a few seconds to fail fast on an unreachable Postgres server now waits up to `connect_timeout` (5 seconds by default) instead. Add `?connect_timeout=2` to the URL to restore the old limit.
:::

Pool settings that newer client versions take in code map to `max_connections(n)` (maximum size) and `acquire_timeout` (wait for a free connection); their connection timeout maps to `connect_timeout` in the URL.

## Database schema

A Postgres URL can name the schema to use, as with the original client:

```text
postgresql://user:pass@host/db?schema=tenant_a
```

- Every pooled connection runs `SET search_path TO "tenant_a"` right after it opens, before any other statement. Generated queries, raw SQL, interactive transactions, `push` and migrations therefore resolve unqualified names in that schema, and only in that schema. The name is always quoted as an identifier (case and spaces are kept), so no value of `schema` can run another statement.
- Without `schema` nothing changes: the server's `search_path` applies (`"$user", public` by default), and no `SET` is sent.
- Models and enums with an explicit schema (`#[model(schema = "audit")]`) keep their qualified names.
- `migrate deploy`, `migrate status` and `push` create the schema when it does not exist, and `_prisma_migrations` lives in it. An existing schema is never re-created, so a role that only has rights on that schema can deploy. Creating a missing schema needs the `CREATE` privilege on the database.
- The path is exactly that one schema: objects of other schemas, such as extension functions installed in `public` or `extensions`, must be qualified, and `CREATE EXTENSION` in a migration installs the extension in the URL schema.
- Migration scripts and `db_execute` run on a connection that is closed afterwards instead of going back to the pool, so a script that changes the session (`SET search_path`, `RESET ALL`) cannot send later queries to another schema. `_prisma_migrations` is always written to the URL schema by its qualified name.
- Query parameters are read as form data: the query ends at the first `#`, keys and values are percent-decoded (`schem%61=t` is `schema=t`), and `+` in a value is a space (`My+Schema`).
- An empty name, `$user`, a name starting with `pg_`, a name with a NUL character, or one longer than 63 bytes fails when the client is created, with an initialization error `P1013` that names `schema` and never shows the URL. The original client accepts an empty name (and then finds no table) and lets the server truncate long names; corvis-orm rejects both.
- On SQLite the parameter is accepted and ignored. The 7.x adapter's `schema` option maps to this URL parameter.
- A session setting that your own SQL changes (`SET search_path ...` outside a transaction) stays on that pooled connection, as with any session setting.

## Use `OrmService`

`OrmService` implements `Deref` to the client, so each model's delegates are called directly on it. The typed helpers generated by `#[derive(Model)]` (`room::name::equals`, `room::created_at::order`, `user::posts::count()`, ...) are the usual way to build a query, and the results come back as the model types:

```rust
use corvis_orm::{OrmError, OrmService, SortOrder};
use crate::models::{Room, RoomDelegate, room};

#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

impl RoomsService {
    pub async fn list(&self) -> Result<Vec<Room>, OrmError> {
        self.db.room().find_many(()).order_by(room::created_at::order(SortOrder::Asc)).order_by(room::name::order(SortOrder::Asc)).await
    }

    pub async fn find(&self, name: &str) -> Result<Option<Room>, OrmError> {
        self.db.room().find_unique(room::name::equals(name)).await
    }
}
```

This is how `examples/blog-orm` paginates posts:

```rust
let posts = self
    .db
    .post()
    .find_many(filters)
    .order_by(post::created_at::order(SortOrder::Desc))
    .skip((page - 1) * limit)
    .take(limit)
    .include(post::author::include())
    .await?;
```

The same queries can be written with JSON arguments. They are the escape hatch for anything the helpers do not cover, and the form that mirrors the reference client:

```rust
self.db.room().find_many(json!({ "orderBy": [{ "createdAt": "asc" }, { "name": "asc" }] })).await
self.db.room().find_unique(json!({ "where": { "name": name } })).await
```

Operations available on each delegate: `find_many`, `find_first`, `find_unique`, `find_first_or_throw`, `find_unique_or_throw`, `create`, `create_many`, `create_many_and_return`, `update`, `update_many`, `update_many_and_return`, `upsert`, `delete`, `delete_many`, `count`, `aggregate` and `group_by`. Every query supports a direct `.await`, `.exec()`, `.exec_json()` (the JSON as is) or `.exec_as::<T>()`. `create_many_and_return` and `update_many_and_return` return the affected rows as `Vec<Model>`:

```rust
let created: Vec<User> = db.user().create_many_and_return(json!({ "data": [{ "email": "a@x.io" }, { "email": "b@x.io" }] })).await?;
let published: Vec<Post> = db.post().update_many_and_return(json!({ "where": { "published": false }, "data": { "published": true } })).await?;
```

::: warning Always filter `update_many_and_return`
Without a `where` it updates every row of the table and returns all of them, held in memory at once (the same as `updateManyAndReturn` in the reference client). Always pass a filter, and bound it when the matching set can be large.
:::

### Writing a DTO

`corvis_orm::Data` wraps any `Serialize` value (or a reference to one) as the data of a write, so a validated DTO needs no `json!`:

```rust
use corvis_orm::Data;

self.db.user().create(Data(&dto)).await;                                // { data: dto }
self.db.user().update(user::id::equals(id)).data(Data(&patch)).await;   // update from a partial DTO
```

Fields the value skips when serializing are not sent, so a patch DTO whose `Option` fields use `#[serde(skip_serializing_if = "Option::is_none")]` changes only the fields that are present. If the value fails to serialize, the query fails with a validation error that names the type, before any SQL runs. `Data` also works in the `.create(...)` and `.update(...)` branches of an `upsert`. It is not part of the prelude, so a glob import cannot clash with your own types.

### Relation counts

`user::posts::count()` asks for the number of posts of each user (`include: { _count: { select: { posts: true } } }`). Several `count()` includes in one query merge into a single `_count`. `#[derive(Model)]` also generates `user::Count`, one `Option<i64>` per to-many relation, and `user::WithCount`, the model with its counts:

```rust
let users = db
    .user()
    .find_many(())
    .include(user::posts::count())
    .exec_as::<Vec<user::WithCount>>()
    .await?;

users[0].model.email; // the model fields
users[0].count.posts; // Some(3)
```

`WithCount<M, C>` serializes the model fields followed by `_count`, which is what `.exec_json()` returns, so `Paginated<user::WithCount>` or `Vec<user::WithCount>` can be a handler's return type. Use `corvis_orm::DateTime` for the timestamps of the models if the JSON must match `.exec_json()` byte for byte: it serializes like a JavaScript `Date`.

### Typed aggregates

`aggregate` and `group_by` return JSON, because their shape depends on the arguments. Read them into a struct of your own with `.exec_as::<T>()`:

```rust
#[derive(Serialize, Deserialize)]
struct Totals {
    #[serde(rename = "_count")]
    count: CountAll,
    #[serde(rename = "_sum")]
    sum: ViewsSum,
}

let totals = self
    .db
    .post()
    .aggregate(json!({ "_count": { "_all": true }, "_sum": { "views": true } }))
    .exec_as::<Totals>()
    .await?;
```

An average comes back as a JSON number that may be an integer (`0`); read it as `serde_json::Number` when the response must keep the engine's exact text.

### Transactions

```rust
let (user, post) = db._transaction(|tx| async move {
    let user = tx.user().create(json!({ "data": { "email": "x@y.z" } })).await?;
    let post = tx.post().create(json!({ "data": { "title": "Hello", "authorId": user.id } })).await?;
    Ok::<_, OrmError>((user, post))
}).await?;
```

The closure commits if it returns `Ok` and rolls back if it returns `Err`. `_transaction_with(options, ..)` accepts `TransactionOptions::new().max_wait(ms).timeout(ms)` or the isolation level, and `_transaction_batch((a, b))` runs several queries in a single transaction. The methods start with `_` because they mirror the `$` functions of the reference JavaScript client, and `$` is not valid in a Rust identifier.

### Raw SQL

`sql!("SELECT ... WHERE id = {id}")` builds a parameterized query: every `{expr}` is a parameter, never concatenated text. It runs with `db._query_raw(..)`, `_query_raw_as::<T>(..)` or `_execute_raw(..)`.

## HTTP errors

Queries return `OrmError`, with the `P2xxx` codes of the reference client (`P2002` duplicate unique key, `P2025` record not found, ...). There are two ways to turn them into responses:

**Global filter.** `OrmExceptionFilter` recognizes the `OrmError`s that arrive as the cause of an `HttpException` and responds with the right status. With it, using `?` in the handler is enough:

```rust
// main.rs
app.use_global_filters(OrmExceptionFilter::new()); // P2002 → 409, P2025 → 404, validation → 400, rest → 500

// controller
#[get(":id")]
async fn find_one(&self, #[param("id", ParseIntPipe)] id: i64) -> HttpResult<User> {
    Ok(self.users_service.find_one(id).await?)
}
```

`OrmExceptionFilter::new().map_code("P2000", 422)` changes the status for a specific code.

**Explicit conversion.** The `OrmResultExt` trait adds `.http()`, which directly returns the right `HttpException` without depending on the filter:

```rust
use corvis_orm::OrmResultExt;

#[post]
async fn create(&self, #[body] dto: CreateUserDto) -> HttpResult<User> {
    self.users_service.create(dto).await.http()
}
```

There is no `impl From<OrmError> for HttpException`: Corvis already converts any `std::error::Error` into a `500`, and Rust does not allow both implementations at once. That is why `?` produces a `500` with the error as its cause, and the filter reinterprets it. `OrmError` does not implement [`HttpError`](/exception-filters#domain-errors) either, so a handler that returns `Result<T, OrmError>` directly still answers `500`; use `.http()` or the filter. Your own domain errors can implement `HttpError` and be returned directly.

## Migrations

The binary applies the SQL migrations in a folder (`<folder>/<name>/migration.sql`) and keeps the record in the same control table used by projects with `schema.prisma`, so a database migrated with the original tool can keep being migrated with `corvis-orm`, and the other way around:

```bash
cargo run -q -p corvis-orm --bin corvis-orm -- migrate deploy --migrations db/migrations --url "$DATABASE_URL"
cargo run -q -p corvis-orm --bin corvis-orm -- migrate status --migrations db/migrations --url "$DATABASE_URL"
```

With `?schema=name` in the URL (see [Database schema](#database-schema)), `migrate deploy`, `migrate status` and `push` create the schema when it does not exist, and `_prisma_migrations` lives inside it.

From code: `db.migrate_deploy(dir).await` and `db.migrate_status(dir).await`. There are no commands to generate migrations from model changes.

## Security

- In production with Postgres, use `sslmode=verify-full` in the URL; `prefer` (the default) does not protect against an attacker on the network. TLS comes from the `tls-rustls` feature (a default one); connection errors show only `host:port`, never the user, the password or the query string.
- Passing a request's JSON directly to a query gives the client all the operators, `select` and `include`. Build the arguments from the fields the route allows, or validate the body with a strict schema.
- Arguments nested more than 125 levels deep are rejected before they are processed (`max_query_depth`, adjustable with `OrmClientOptions::max_query_depth(n)`).
- A `where` with a NUL character (`\u0000`) in a string, a JSON `path` segment or a JSON key is a validation error on every database, before any SQL runs. PostgreSQL cannot store NUL in `text` or `jsonb`, and the original client answers with a raw database error; Corvis answers with a validation error (`400` through `OrmExceptionFilter`) instead.
- In unique inputs (`findUnique`, `update`, `upsert`, `delete`, ...) the `@id` and `@unique` fields only accept the value (`{ email: "a@b.c" }`), never a filter such as `{ equals }` or `{ not }`, nor `null`.
- Out-of-range numbers are errors, never truncated values: a `BigInt` beyond 64 bits, a `Bytes` byte outside `0..=255`, a `Decimal` that does not fit a Postgres `NUMERIC`, or a negative `limit` in `update_many`/`delete_many` (`P2019`).
- `update`, `delete` and the `*_many` operations repeat their `where` in the `UPDATE`/`DELETE`, so optimistic concurrency (`where: { id, version }`) has a single winner.
- `log_queries` writes the parameter values to the logs.

## What is not supported

MySQL, SQL Server, MongoDB and CockroachDB; migration generation (`migrate dev`, `diff`, `reset`); client middlewares and extensions; raw SQL inside `_transaction_batch`.
