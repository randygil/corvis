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
| `max_connections(n)` | Pool size |
| `explicit_connect(bool)` | Connect in `on_module_init` (default `true`) |
| `transaction_options(..)` | Default `maxWait`, `timeout` and `isolationLevel` for transactions |
| `push::<(A, B)>()` | Creates the missing tables and indexes at startup (useful in development and tests) |

`push` does not modify or drop existing columns: use migrations for production.

## Use `OrmService`

`OrmService` implements `Deref` to the client, so each model's delegates are called directly on it:

```rust
use corvis_orm::{OrmError, OrmService};
use crate::models::{Room, RoomDelegate};

#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

impl RoomsService {
    pub async fn list(&self) -> Result<Vec<Room>, OrmError> {
        self.db.room().find_many(json!({ "orderBy": [{ "createdAt": "asc" }, { "name": "asc" }] })).await
    }

    pub async fn find(&self, name: &str) -> Result<Option<Room>, OrmError> {
        self.db.room().find_unique(json!({ "where": { "name": name } })).await
    }
}
```

The same query can be written with the typed helpers. This is how `examples/blog-orm` paginates posts:

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

Operations available on each delegate: `find_many`, `find_first`, `find_unique`, `find_first_or_throw`, `find_unique_or_throw`, `create`, `create_many`, `create_many_and_return`, `update`, `update_many`, `update_many_and_return`, `upsert`, `delete`, `delete_many`, `count`, `aggregate` and `group_by`. Every query supports a direct `.await`, `.exec()`, `.exec_json()` (the JSON as is) or `.exec_as::<T>()`.

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

There is no `impl From<OrmError> for HttpException`: Corvis already converts any `std::error::Error` into a `500`, and Rust does not allow both implementations at once. That is why `?` produces a `500` with the error as its cause, and the filter reinterprets it.

## Migrations

The binary applies the SQL migrations in a folder (`<folder>/<name>/migration.sql`) and keeps the record in the same control table used by projects with `schema.prisma`, so a database migrated with the original tool can keep being migrated with `corvis-orm`, and the other way around:

```bash
cargo run -q -p corvis-orm --bin corvis-orm -- migrate deploy --migrations db/migrations --url "$DATABASE_URL"
cargo run -q -p corvis-orm --bin corvis-orm -- migrate status --migrations db/migrations --url "$DATABASE_URL"
```

From code: `db.migrate_deploy(dir).await` and `db.migrate_status(dir).await`. There are no commands to generate migrations from model changes.

## Security

- In production with Postgres, use `sslmode=verify-full` in the URL; `prefer` (the default) does not protect against an attacker on the network. TLS comes from the `tls-rustls` feature (a default one); connection errors show only `host:port`, never the user, the password or the query string.
- Passing a request's JSON directly to a query gives the client all the operators, `select` and `include`. Build the arguments from the fields the route allows, or validate the body with a strict schema.
- Arguments nested more than 125 levels deep are rejected before they are processed (`max_query_depth`, adjustable with `OrmClientOptions::max_query_depth(n)`).
- In unique inputs (`findUnique`, `update`, `upsert`, `delete`, ...) the `@id` and `@unique` fields only accept the value (`{ email: "a@b.c" }`), never a filter such as `{ equals }` or `{ not }`, nor `null`.
- Out-of-range numbers are errors, never truncated values: a `BigInt` beyond 64 bits, a `Bytes` byte outside `0..=255`, a `Decimal` that does not fit a Postgres `NUMERIC`, or a negative `limit` in `update_many`/`delete_many` (`P2019`).
- `update`, `delete` and the `*_many` operations repeat their `where` in the `UPDATE`/`DELETE`, so optimistic concurrency (`where: { id, version }`) has a single winner.
- `log_queries` writes the parameter values to the logs.

## What is not supported

MySQL, SQL Server, MongoDB and CockroachDB; migration generation (`migrate dev`, `diff`, `reset`); client middlewares and extensions; raw SQL inside `_transaction_batch`.
