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

### What `push` creates

`push` (and `db.db_push::<(A, B)>()`) creates what the models need and the database does not have yet, the way the original client's migration for the same models would:

- Tables with every column, its type, nullability and default (`now()` as `CURRENT_TIMESTAMP`, `dbgenerated(..)`, literals, enum defaults by their `@map` value), and the primary key over all `@id` / `@@id` fields, named like the reference (`<table>_pkey` or `map:`).
- Unique indexes for `@unique` and `@@unique`, plain indexes for `@@index` (with `USING HASH`, `GIN`, `GIST`, `SPGIST` or `BRIN` on Postgres), and the join table of every implicit many-to-many relation (`_CategoryToPost`, columns `A` and `B`), all named like the reference.
- Foreign keys with their `onDelete` / `onUpdate` actions (on SQLite inside `CREATE TABLE`).
- On Postgres: enum types (`CREATE TYPE "role_t" AS ENUM (...)`), the `@@schema` schemas, scalar lists as arrays (`INTEGER[]`), the `@db.*` native types (`UUID`, `DATE`, `VARCHAR(n)`, `TIMESTAMPTZ(p)`, `DECIMAL(p,s)`, `JSON`, ...) and `TIMESTAMP(3)` for a `DateTime` without one. On SQLite, `Decimal` columns are `DECIMAL` and scalar lists are JSON text (a Corvis extension: the original does not support lists on SQLite).

The whole push runs in one transaction: if a statement fails, nothing is left behind. A construct it cannot build (a native type or an index type it does not know) is a validation error that names the model, the field and the construct, before any statement runs. Tables that already exist are left as they are, so a database pushed by an earlier version of Corvis keeps its old column types (`TIMESTAMPTZ(3)`, `TEXT` for enums and decimals); re-create a development database with `db_push_force_reset::<Models>()`, which drops the tables of the models (and the join tables between them) and pushes them again, in one transaction.

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

`RawSql::join(values, ",", "(", ")")` of an empty list does not panic: the statement carries the original client's error ("Expected `join([])` to be called with an array of multiple elements, but got an empty array"), also when it is nested in another one, and running it fails with a validation error before anything is sent.

### Decimals

Text becomes a `Decimal` only through conversions that report invalid input: `Decimal::parse`, `"12.5".parse::<Decimal>()` and `Decimal::try_from(..)` for text that arrives at run time, and the `decimal!("1.50")` literal, checked when the program is compiled (an invalid literal does not compile). `From<&str>` and `From<String>` no longer exist: they panicked on invalid text, and since the typed helpers take `impl Into<Decimal>`, text from a request could crash the handler.

```rust
use corvis_orm::{decimal, Decimal};

let rate = decimal!("40.50");                   // was: Decimal::from("40.50")
let amount: Decimal = input.parse()?;           // was: Decimal::from(input.as_str()), which panicked
let users = db.wallet().find_many(vec![wallet::amount::gte(decimal!("100"))]).await?;
```

A `DecimalCtor` (`Decimal.clone({...})`) reads text with `D.parse("1234.5678")?` instead of `D.of("1234.5678")`. Parsing and `decimal!` accept what decimal.js accepts, checked input by input against it: `0x`, `0b` and `0o` numbers with a fraction and a binary exponent (`0x1.8p3`), and one `_` between two digits (`1_000`).

`"NaN"`, `"Infinity"` and `"-Infinity"` are not `Decimal` input in a query: a filter or a write that gives them to a `Decimal` field fails with a validation error on both databases, as with the original. On SQLite they used to be compared as text. A `Float` field takes no text at all (`"Infinity"` included), as in the original. A `Decimal` text longer than any `NUMERIC` can hold (more than 147,487 characters, or a `0x` / `0b` / `0o` text with more than 65,536 bits of fraction) is refused before it is parsed.

`corvis_orm::or_undefined(value)` writes a non-finite float as `null`, like the original sends `NaN`, but a value that fails to serialize makes the query fail with a validation error instead of becoming `null`.

### Json nulls

A `Json` field has two nulls: the database `NULL` and the JSON value `null`. Write them with `corvis_orm::db_null()` (the database `NULL`), `corvis_orm::json_null()` (the JSON `null`) and, in filters, `corvis_orm::any_null()` (either):

```rust
db.query_json(User::meta(), "update", json!({ "where": { "id": 1 }, "data": { "data": corvis_orm::db_null() } })).await?;
let empty = db.query_json(User::meta(), "findMany", json!({ "where": { "data": { "equals": corvis_orm::any_null() } } })).await?;
```

They are whole values only: the value of a Json field in `data`, or the operand of `equals` / `not` in a Json filter. Inside a Json value (`{ "deletedAt": corvis_orm::json_null() }`, `[corvis_orm::db_null()]`), as an item of a `Json[]` field, or given to another operator they make the query fail with a validation error. The original stores the name there (`{ "deletedAt": "JsonNull" }`); write `null` for the JSON `null` inside a value. Like `undefined()`, these values carry a secret of the process: arguments built with them are for the ORM only, never for a response or a log (see [Security](#security)).

::: warning Changed: `{ "$type": "DbNull" }` is JSON data
The plain object `{ "$type": "DbNull" }` (and `"JsonNull"`, `"AnyNull"`) used to mean these nulls. A request body can carry that object, so a client could turn its JSON value into a database `NULL` or into an `IS NULL` filter. It is now JSON data like any other object, as in the original client, where only `DbNull`, `JsonNull` and `AnyNull` themselves are the nulls. Replace `json!({ "$type": "DbNull" })` with `corvis_orm::db_null()`, and the same for the other two.
:::

### Typed helpers and non-finite floats

A value given to a typed helper (`user::score::equals(..)`, `set`, `increment`, `has`, `push`, ...) that fails to serialize, or an `f32` / `f64` that is `NaN` or infinite (alone, in a list or in an `Option`), makes the query fail with a validation error before any SQL runs. It used to become `null`, so `score::equals(f64::NAN)` read the rows where `score IS NULL` and `score::not(f64::NAN)` changed every row with a score. Write `is_null()`, `is_not_null()` or `set_null()` for `null`.

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
- The JSON form of a query is a query language, not data. A `serde_json::Value` taken from a request must never be forwarded as `where` or `data`: it gives the client every operator, `select` and `include`, and many shapes match every row, as in the original client: `{ "token": {} }`, `{ "token": { "notIn": [] } }`, `{ "AND": [] }`, or an object whose duplicated key leaves `{}`. A lookup by a secret such as `{ "reset_token": body["token"] }` then finds any user. Build the arguments from validated fields of the types the route expects (`body.token: String`), or validate the body with a strict schema first.
- Arguments nested more than 125 levels deep are rejected before they are processed (`max_query_depth`, adjustable with `OrmClientOptions::max_query_depth(n)`).
- A `where` with a NUL character (`\u0000`) in a string, a JSON `path` segment or a JSON key is a validation error on every database, before any SQL runs. PostgreSQL cannot store NUL in `text` or `jsonb`, and the original client answers with a raw database error; Corvis answers with a validation error (`400` through `OrmExceptionFilter`) instead.
- In unique inputs (`findUnique`, `update`, `upsert`, `delete`, ...) the `@id` and `@unique` fields only accept the value (`{ email: "a@b.c" }`), never a filter such as `{ equals }` or `{ not }`, nor `null`.
- Out-of-range numbers are errors, never truncated values: a `BigInt` beyond 64 bits, a `Bytes` byte outside `0..=255`, a `Decimal` that does not fit a Postgres `NUMERIC`, or a negative `limit` in `update_many`/`delete_many` (`P2019`).
- `update`, `delete` and the `*_many` operations repeat their `where` in the `UPDATE`/`DELETE`, so optimistic concurrency (`where: { id, version }`) has a single winner.
- `log_queries` writes the parameter values to the logs.
- `corvis_orm::undefined()` and the Json nulls are values no request can produce: each carries a random marker drawn when the process starts, one per kind. A string or an object that arrives in a request body is always data, never "leave this condition out" or "compare with `NULL`". The marker is never stored (a Json null is accepted only as a whole value, see [Json nulls](#json-nulls)), never sent to the database, and the ORM's errors and the `Debug` output of its typed parameters show only its kind (`undefined`, `DbNull`).
- **Arguments built with `undefined()`, `or_undefined(..)`, `db_null()`, `json_null()` or `any_null()` are for the ORM only.** Never serialize such a value into a response, a log line or a message: for example, a list endpoint that echoes "the filter applied" built as `json!({ "status": or_undefined(status) })` would send the marker's secret to the client, who could then send it back and drop conditions again. Build what you echo from your own validated fields (`status: Option<Status>`), and do not log the arguments (`{:?}` of a `json!` value shows the secret). This is the application's responsibility until the ORM takes its arguments as a type of its own: today they are `serde_json::Value`, a type the ORM cannot tell apart from any other JSON once it leaves the query, so it cannot redact a response or a log it does not write.
- The SQLite that `sqlx` 0.8 bundles is older than 3.54.0 and has a known flaw (RUSTSEC-2026-0459): opening a database file together with a crafted `-wal` file can return stale bytes from the process's memory in query results. Only open SQLite files your application created; never a database or `-wal` file that a less trusted party could have written. The upgrade is tracked in [#155](https://github.com/randygil/corvis/issues/155).

## PostgreSQL and SQLite differences

Most queries give the same results on both databases. These do not. Each row was recorded with the original client on
both databases (the oracles of `crates/corvis-orm/tests/`), and Corvis does what the original does, except where the
last column says otherwise.

| What | PostgreSQL | SQLite |
|---|---|---|
| `contains`, `startsWith`, `endsWith` | Case-sensitive | `LIKE`: folds ASCII letters only (`"ANA"` matches `"ana@x.io"`, `"JOSÉ"` does not match `"José"`). The original reads `%` and `_` of the input as wildcards there (`contains: "%"` matches every row); Corvis matches them literally, as on Postgres |
| `mode: "insensitive"` | Folds every letter (`ILIKE`, `LOWER`) | The original rejects `mode` on SQLite. Corvis accepts it as an extension: `LOWER` folds ASCII letters only |
| JSON `path` | A list (`["a", "0"]`) | The original takes one string (`"$.a[0]"`; `"$.0"` and `'$."0"'` are the key `"0"`) and rejects the list form and the `array_*` filters. Corvis also accepts both as an extension, a numeric segment of a list being an array index |
| `search` | Full-text search (`to_tsquery`; the original needs the `fullTextSearchPostgres` preview feature) | Not in the original. Corvis extension: every term must appear in the text (`INSTR`), case-insensitive |
| `createMany` with `skipDuplicates` | `ON CONFLICT DO NOTHING` | Not in the original. Corvis extension: `INSERT OR IGNORE` |
| `Decimal` | Exact, with the precision and scale of the column | A number with 15 significant digits (numeric affinity), like the original's SQLite migration; `_avg` keeps 16 significant digits. Comparisons, ordering and aggregates read the column with `CAST(.. AS NUMERIC)`, so they are numeric on `TEXT` columns of earlier pushes too; the cast keeps a plain index on the column from serving a range |
| Writing a `Decimal` beyond the column's precision | Fails (`numeric field overflow`) | The original stores it (`1e100`). Corvis refuses a value beyond `Decimal(65, 30)` (or the field's `@db.Decimal(p, s)`), the bound of its security review, and an arithmetic update (`increment`, `multiply`, ...) whose result leaves it: the statement fails with the error Postgres gives and SQLite undoes it, every row of it |
| Writing an `Int` beyond 32 bits | Fails (conversion error) | The original stores it, and then every read of the row fails with `P2023`, so one value breaks every list of the table. Corvis refuses it with the error Postgres gives, before any SQL runs (security over parity) |
| Filtering on an `Int` beyond 32 bits, or a `Decimal` beyond `NUMERIC` | Fails (conversion error) | Runs, comparing numbers |
| Scalar lists (`String[]`) | Arrays | JSON text (a Corvis extension; the original does not support lists on SQLite) |
| `NaN` and infinite floats | Sent as `null` by the original (`ratio: NaN` is `ratio IS NULL`) | The same. Corvis sends `null` too in JSON arguments; its typed helpers refuse such values instead (see above) |
| `divide: 0` | Fails (`22012`, division by zero); `null` stays `null` | The result is `null`: `P2011` on a required column, `null` stored in an optional one |
| `increment` past the range of an `Int` | Fails (`22003`) | The original stores the value (SQLite integers are 64-bit) and reading the row fails with `P2023` (`Value 2147483648 does not fit in an INT column`). Corvis fails the statement with the error Postgres gives (`22003`, `integer out of range`), and SQLite undoes it |
| An `Int` column holding a fraction or a number beyond 32 bits (written by another program) | Cannot happen | Reading the row fails with `P2023`, never a truncated or saturated value |
| `increment` / `multiply` past the range of a `BigInt` or to an infinite `Float` | Fails (`22003`, `bigint out of range` / `value out of range: overflow`) | The original stores a float in the `BigInt` column, or `inf`, and every read of the row then fails with `P2023`. Corvis fails the statement with the error Postgres gives, and SQLite undoes it. A `BigInt` column holding a float or a `Float` column holding an infinity (written by another program) reads as `P2023`, like in the original |

## What is not supported

MySQL, SQL Server, MongoDB and CockroachDB; migration generation (`migrate dev`, `diff`, `reset`); client middlewares and extensions; raw SQL inside `_transaction_batch`.
