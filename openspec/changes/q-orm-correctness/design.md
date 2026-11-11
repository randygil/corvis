# Design

## Context

Source: the engineering-quality review of `crates/corvis-orm` and `crates/corvis-orm-macros` filed under epic #187, read
on `dev` at `d46758a` (2026-11-11), `sqlx` 0.8.6. The review was read-only (nothing compiled or run); every finding below
was re-read in the code, and the line numbers are those of `d46758a`. **Task 1.1 re-checks them at implementation time.**

The module's promise is parity with the original client, checked three ways: the Postgres oracle
(`tests/pg_oracle.rs`, 225 cases recorded with the original client on the schema `tests/fixtures/oracle/schema.prisma`),
topic fixtures recorded the same way (`connect_timeout.json`, `pool_url.json`, `schema_url.json`, `decimal_js.json`, each
with `recorded_with` and `script`), and the three port apps, which run on Postgres and are compared with their originals
after every change. SQLite is used by the examples (`examples/blog-orm`, `examples/tertulia`, both with
`OrmModuleOptions::push`) and by users; it has no recorded reference today.

### How the reference results are recorded

- **Postgres oracle.** `corvis-tests/tmp/orm/oracle/full/` (outside the repository): `schema.prisma`, `migration.sql`
  (the original's migration for that schema, plus a marked block of extras: a `CHECK` and a trigger), `cases.js` (an
  ordered list `[label, model, action, args]` run in sequence on one database, so later cases see the effects of earlier
  ones), `regen.sh` (drops and recreates `orm_test_oracle_full` in the `corvis-pg` container on `127.0.0.1:55432`, applies
  `migration.sql`, runs `node cases.js > results.json` with the original client 6.x, copies `results.json` and
  `migration.sql` into `crates/corvis-orm/tests/fixtures/oracle/`). `cases.js` revives typed markers (`$date`,
  `$decimal`, `$json`, `$type: DbNull|JsonNull|AnyNull`) and writes BigInts beyond 2^53 as strings. The runner compares
  results as text (key order included) and errors by class name, code, meta and core message (validation errors by class
  name only).
- **Topic fixtures.** One recorder per topic (`corvis-tests/tmp/orm/connect/cases.js`, ...) with scratch clients
  generated for a Postgres and a SQLite schema (`gen-pg`, `gen-sqlite`), writing one JSON object with
  `recorded_with: "@<original client> 6.19.3"` and `script`.

New cases follow these rules: Postgres cases are **appended at the end** of `cases.js` (earlier results must not change:
task 3.2 checks that the regenerated `results.json` differs from the committed one only by the appended entries); new
markers are added to `revive` in the recorder and to the runner; a fixture is never edited by hand.

### The findings, confirmed against the code

| Finding | Where now | Verdict |
|---|---|---|
| M1 `Decimal` on SQLite compared as text | `schema.rs:26` (`(Sqlite, _) => "TEXT"` covers `Decimal`), `engine/mod.rs:371-375` (bound as `Text(to_fixed_plain())`), `engine/filter.rs:329-340` (`col op ?`, no conversion), `engine/aggregate.rs:158` (`_sum`/`_avg` of `Decimal` uncast), `engine/write.rs:341-342` (`col op ?`) | **Confirmed for tables created by `db push`** (the examples' path): `TEXT` affinity column, `TEXT` parameter, so `"99.5" >= "100"` and `_max` is lexicographic; `to_fixed_plain` strips trailing zeros, so no padding saves it, and negatives sort wrong. **Not reachable on tables created by the original's SQLite migration**, which declares `DECIMAL` (NUMERIC affinity): SQLite then applies numeric affinity to the text parameter and compares numbers. Needs a failing test first (task 2.1) |
| M1 arithmetic and sums in floating point | same lines | Confirmed, but this is SQLite's numeric affinity, which the original has too on its own `DECIMAL` columns. Settled by the SQLite recording (Decision 2), not changed against it |
| M3 `db push` builds a wrong schema silently | `schema.rs:61,79` (`PRIMARY KEY (id.column)` where `id()` is `pk()[0]`, `meta.rs:322-323`), `schema.rs:135-149` (only single-column unique indexes), `schema.rs:45` (enum → `TEXT` while `engine/mod.rs:237` casts to `::"<enum>"`), `schema.rs:97,103` (no `@@schema`) | Confirmed, and wider than reported (siblings below). Needs failing tests first (task 2.2) |
| M3 siblings | `schema.rs:7-47`: `f.list` ignored (`Int[]` → `INTEGER`, `String[]` → `TEXT`); `@db.*` ignored except `Decimal` (`@db.Uuid` → `TEXT`, and `engine/mod.rs:238` compares with `$1::uuid`: `operator does not exist: text = uuid`); `DateTime` → `TIMESTAMPTZ(3)` where the reference migration has `TIMESTAMP(3)`; `literal_sql` (`schema.rs:49-57`) writes an enum default with its client name, not its `@map` value; `primary_key_map`, `UniqueMeta.map`, `RelationMeta.map` ignored; `ModelMeta.unsupported` columns not created; `@default(now())` / `dbgenerated(..)` not written | Found while confirming |
| N6 `db_push_models` not transactional | `client.rs:704-713` (one `execute` per statement on the pool) | Confirmed. Part of Decision 3 |
| M4 NaN or a serialization failure becomes `null` | `query.rs:25-27` (`to_value(v).unwrap_or(Value::Null)`), used by every generated helper (`corvis-orm-macros/src/lib.rs:918-942,956-979,1010-1021,1048-1058`); `filter.rs:218-220,270-271,279-282` turn `null` into `IS NULL` / `IS NOT NULL` | Confirmed: `serde_json::to_value(f64::NAN)` is `Ok(Value::Null)` (non-finite floats have no JSON form), so it is not even an error today. Impact narrower than reported on Postgres (`<> 'NaN'` and `IS NOT NULL` differ only by `NaN` rows), large on SQLite (a bound `NaN` is stored and compared as `NULL`, so the intended filter matches nothing while the generated one matches every non-null row). Needs a failing test first (task 2.3) |
| S1 `From<&str> for Decimal` panics | `decimal.rs:1499-1517` (`From<&str>`, `From<String>`, `From<&String>`) | Confirmed. Reachable: one port app converts a JSON string read from the database with `Decimal::from(s.as_str())` (`facten-api/src/fiscal-notes/fiscal-notes.service.rs:75`) |
| N2 `RawSql::join` panics on an empty list | `raw.rs:74-89` (`assert!` at 83) | Confirmed |
| S4 key lists past the parameter limit | `read.rs:580-599` (compound include: `OR` of one group per parent), `read.rs:716-736` (`count_related` compound), `read.rs:823` (`read_back_many`: `OR` of ids), `write.rs:44-48` (`ids_filter`, used at `write.rs:723,810,816,934`), `write.rs:571-590` (`m2m_unlink`: `IN (` one parameter per key), `filter.rs:314-323` (`in` with `mode: insensitive`) | Confirmed. Sibling: `write.rs:842` (`OR` of the caller's items for nested `delete`/`disconnect` lists). `push_in_list` (`filter.rs:395-433`) already handles single-column lists |
| S8 dialect differences | `filter.rs:248-262,446-449` (`LOWER` folds ASCII only in SQLite), `write.rs:406-412` (`/`: `NULL` in SQLite, error 22012 in Postgres), `engine/mod.rs:335-340` (`Int` bound as 64-bit on SQLite, so `increment` past 2^31 succeeds and fails later when read into `i32`), `filter.rs:66-77` (numeric segment always an index on SQLite) | Confirmed. Sibling: `search` (`filter.rs:350-381`) is `to_tsquery` on Postgres and an all-terms `INSTR` on SQLite. What the original does for each is settled by recording (Decision 7) |

No finding of this change is dropped. Two are narrowed: M1 is reachable only on `TEXT` columns (Corvis' own `db push`),
and its arithmetic half is the original's behavior on SQLite; M4's damage on Postgres is limited to `NaN` rows.

## Goals / Non-Goals

**Goals:**

- A recorded SQLite reference, so SQLite behavior is decided by the original's results, not by memory.
- No silently wrong result: SQLite `Decimal` comparison, `db push`, typed helpers.
- No panic on input a caller can pass at run time.
- No query that fails because the engine itself built a list longer than the parameter limit.
- Postgres SQL and results unchanged (the port apps run there).

**Non-Goals:**

- Exact decimal arithmetic on SQLite beyond what the original does.
- `db push` that alters existing tables.
- Performance (`q-orm-hot-path`) and API cleanup (`q-orm-api-hygiene`).

## Decisions

### 1. A SQLite reference oracle, and new Postgres cases

- Recorder `corvis-tests/tmp/orm/oracle/sqlite/` (outside the repository, like `full/`): `schema.prisma` with
  `provider = "sqlite"`, derived from the Postgres oracle schema by removing only what the original's SQLite connector
  rejects when it validates the schema (scalar lists, `@db.*` native types, and enums if rejected); the removed items are
  listed in the schema file's header. `migration.sql` is produced by the original's own tooling
  (`migrate diff --from-empty --to-schema-datamodel schema.prisma --script`), never written by hand. `cases.js` is the
  Postgres `cases.js` minus the cases that use removed features, plus the SQLite cases of Decisions 2, 4 and 7. It runs on a
  temporary database file and writes `{ recorded_with, script, cases: [...] }`. `regen.sh` copies `schema.prisma`,
  `migration.sql` and `results.json` into `crates/corvis-orm/tests/fixtures/oracle_sqlite/`.
- Models: `tests/generated/oracle_sqlite.rs`, generated from that schema with `corvis-orm generate` (as
  `generated/oracle.rs` is).
- Runner `tests/sqlite_oracle.rs`: the same comparison rules as `pg_oracle.rs`, run **twice**: on a database built from the
  recorded `migration.sql` (the layout of an app migrated from the original), and on one built by `db_push` of the
  generated models (the examples' path). Both must equal `results.json`. A `KNOWN_DIFFERENCES` table (label, which of the
  two databases, reason) lists accepted differences; it is empty at the end of this change unless a security reason
  applies, and each entry needs the maintainer's approval in the review.
- New markers: `{ "$float": "NaN" | "Infinity" | "-Infinity" }` (revived to the JS number by the recorder; the runner
  sends the engine's string form `"NaN"`, `"Infinity"`, `"-Infinity"`, which `engine/mod.rs:361-363` already accepts).
- Postgres: the cases of Decisions 4 and 7 are appended to `full/cases.js` and recorded; `pg_oracle.rs` gains the
  `$float` marker.

Alternative considered: compare SQLite against Postgres results. Rejected: the dialects legitimately differ (that is the
point of S8), and the original's SQLite behavior is what an app migrated from SQLite expects.

### 2. `Decimal` on SQLite follows the recorded original

The rule: on SQLite, `Decimal` comparisons, ordering, aggregates and arithmetic give the recorded original's results. The
original declares `DECIMAL` on SQLite, so SQLite stores and compares those values as numbers (numeric affinity, 64-bit
floating point, 15 significant digits). The expected implementation, confirmed or revised by the recording (task 3.1):

- `schema.rs`: `Decimal` is `DECIMAL` on SQLite, like the reference migration. New pushes store numbers.
- Engine: on SQLite only, every `Decimal` column that appears as an operand of a comparison (`=`, `<>`, `<`, `<=`, `>`,
  `>=`, `IN`, `NOT IN`, the cursor condition), in `ORDER BY`, in `MIN`/`MAX`/`SUM`/`AVG`, in `having`, and on the right of
  `SET col = col <op> ?`, is written `CAST(<col> AS NUMERIC)`. On a `DECIMAL` column this is a no-op; on a `TEXT` column
  created by an earlier `db push` it makes the comparison numeric, so existing databases are fixed without a migration.
  The bound value stays text (SQLite converts it under the cast's numeric affinity).
- The comment at `engine/mod.rs:465-469` ("SQLite has no such type (corvis-orm stores decimals as exact text)") is
  replaced: SQLite keeps 15 significant digits, like the original; `data_value`'s rounding and bound stay (they also bound
  what `"1e100000"` stores) unless the recording shows the original stores such values differently.
- Postgres: no change. `pg_cast`, binding and the SQL text are untouched; task 1.2 snapshots the Postgres SQL of every
  builder this decision edits, and task 4.2 checks it byte for byte.

Cost, stated: a cast column cannot use a plain index on that column for range predicates on SQLite. `Decimal` indexes on
SQLite are rare, and the alternative leaves wrong results; recorded in "Risks".

Verified by: `tests/sqlite_decimal.rs` (task 2.1, failing before): on a pushed database and on a migrated one, with
`99.5, 100, 1000, -5`: `gte 100`, `lt 0`, `in`, `orderBy` both ways, `_min`, `_max`, `having` on `_sum`; and the SQLite
oracle's `Decimal` cases (comparisons, `increment` by `0.01` of `12345678901234.56`, `0.1 + 0.2`, `_sum`, `_avg`), which
must equal the recording on both databases.

Alternative considered (the review's proposal): an exact decimal collation registered on every connection
(`SqliteConnectOptions::collation`), and `increment`/`multiply`/`_sum`/`_avg` computed in Rust. Rejected: it gives results
the original does not give (exact where the original rounds to 15 digits), on migrated databases too, and it moves
grouped aggregates, `having` and their ordering into Rust. If exact SQLite decimals are wanted, they are a documented
Corvis extension in a follow-up, not a parity fix.

### 3. `db push` builds the reference schema, or refuses

What `push_statements` creates, per dialect, follows the original's migration for the same models (Postgres: the oracle's
`migration.sql`, lines 1-187; SQLite: the recorded `oracle_sqlite/migration.sql`):

- **Order** like the reference: `CREATE SCHEMA IF NOT EXISTS` for every `@@schema` (Postgres), enum types, tables, unique
  and plain indexes, then foreign keys as `ALTER TABLE .. ADD CONSTRAINT` (Postgres; this also removes the need for
  `sorted()` and its cycle fallback). SQLite has no `ADD CONSTRAINT`: foreign keys stay inline and tables are ordered as
  today.
- **Keys.** The primary key is `pk()` (all fields), named `primary_key_map` or `<table>_pkey`; SQLite keeps
  `INTEGER PRIMARY KEY AUTOINCREMENT` for a single autoincrement id as the reference does.
- **Uniques and indexes.** Every `@unique` and `@@unique` as `CREATE UNIQUE INDEX` named `map` or
  `<table>_<cols>_key`; every `@@index` as `CREATE INDEX` named `map` or `<table>_<cols>_idx`, with `USING <type>` on
  Postgres for `Hash`, `Gin`, `Gist`, `SpGist`, `Brin`.
- **Implicit many-to-many.** `_<Relation>` with `A` and `B` typed as the ids of the two models, the primary key or unique
  index and the `B` index exactly as the reference names them, and both foreign keys `ON DELETE CASCADE ON UPDATE
  CASCADE`; `@@schema` of the owning model as `JoinTable.schema` already says.
- **Types.** Postgres: enums as `CREATE TYPE .. AS ENUM` with the `@map` values in schema order, qualified by the enum's
  `@@schema`; scalar lists as `<type>[]`; `@db.*` native types written as the reference writes them (`UUID`, `DATE`,
  `TIMESTAMPTZ(p)`, `TIMESTAMP(p)`, `TIME(p)`, `VARCHAR(n)`, `CHAR(n)`, `TEXT`, `SMALLINT`, `REAL`,
  `DOUBLE PRECISION`, `DECIMAL(p,s)`, `JSON`, `JSONB`, `BYTEA`, `INET`, `CITEXT`, `XML`, `MONEY`, `BIT(n)`, `VARBIT(n)`,
  `OID`); `DateTime` without a native type is `TIMESTAMP(3)`. SQLite: the reference's types (`DECIMAL` per Decision 2);
  scalar lists stay `TEXT` holding JSON (a Corvis extension the engine already reads and writes; the original rejects lists
  on SQLite).
- **Defaults.** Literals with the enum's `@map` value for enum fields; `now()` as `CURRENT_TIMESTAMP`; `dbgenerated("x")`
  as `DEFAULT x` (text from the developer's own model, like any `Unsupported` type); autoincrement as today.
- **`Unsupported("..")`** columns are created with their declared type and nullability.
- **Refusals.** Before any statement, `push_statements` returns a validation error naming the model, the field and the
  construct when it meets something the reference would build and this code cannot express (a native type outside the
  list, an index type outside the list on Postgres, an `@@index` with operator classes or sort orders). Views are skipped,
  as the original's push skips them.
- **Atomic.** `db_push_models` and `db_push_force_reset` run `ensure_schema` and all statements in one transaction
  (`_transaction_batch`-style, no timeout) on both dialects; a failure rolls everything back.

Existing tables are not altered (`IF NOT EXISTS`, as documented): a database pushed by an earlier version keeps its old
column types; `docs/data/corvis-orm.md` says how to re-create a development database (`db_push_force_reset`).

Verified by: `tests/db_push.rs` (task 2.2, failing before): `schema_sql` of the oracle `Pair` holds
`PRIMARY KEY ("a", "b")`; a nested `categories: { create: [..] }` on a pushed database succeeds (today `P2021`); on
Postgres, `user.create({ role: "ADMIN" })` on a pushed database succeeds (today `type "role_t" does not exist`); a
`String @db.Uuid` filter succeeds (today `operator does not exist`). Then the catalog comparison (task 5.5): two fresh
Postgres databases, one from the reference migration (without its extras block), one from `db_push` of the oracle
models; the listing of tables, columns (type, nullability, default), primary keys, unique and plain indexes
(`pg_indexes.indexdef`), foreign keys (`pg_get_constraintdef`) and enum types (labels in order) must be equal. Same for
SQLite (`sqlite_master`, `pragma_table_xinfo`, `pragma_index_list`, `pragma_foreign_key_list`) against the recorded
SQLite migration. Finally both oracles run on pushed databases (Decision 1). Where `corvis-tests` and the `vector`
extension are available, the houseplan models are compared with the houseplan migrations as well.

Alternative considered: return an error for every construct `db push` does not build today. Rejected: compound keys,
join tables and enums are ordinary schemas; refusing them would make `push` useless for most apps.

### 4. Typed helpers never turn a value into `null`

- `corvis_orm::to_json` becomes `fn to_json<T: Serialize + ?Sized>(v: &T) -> Result<Value, OrmError>`: a serialization
  error is a validation error naming the type. Non-finite `f32`/`f64` (alone, inside `Vec`, or inside `Option`) are
  handled by a `serde_json` serializer wrapper that sees them before `serde_json` turns them into `null`.
- `WhereParam`, `SetParam`, `OrderByParam`, `IncludeParam` and `SelectParam` gain a private `error: Option<OrmError>`;
  `new(key, value)` keeps its signature (no error), `try_new(key, Result<Value>)` sets it. `where_object`,
  `data_object`, `and`, `or`, `not`, the nested relation helpers (`is`, `some`, `connect`, `create`, ...) carry the
  first error up; `Args`, `WhereArg` and `DataArg` take it, and `Query::with_error` reports it before any SQL, as `Data`
  already does (`query.rs:212-226`).
- Non-finite floats, settled by the recording (task 3.2, cases on a row with `ratio = NaN` and a row with `ratio = null`,
  so `NaN` and `null` give different rows): if the original sends them as values (`<> 'NaN'` semantics), the helpers send
  `"NaN"`, `"Infinity"`, `"-Infinity"` (the engine's accepted form) and results must equal the recording on both
  dialects. If the original turns them into `null`, the helpers refuse them with a validation error ("`score`: NaN and
  infinite values cannot be compared or stored; use `is_null()` for null"): a Rust caller cannot mean `null` by `NaN`,
  and the JSON path keeps the original's `null` behavior, so parity of `query_json` is unchanged.
- The macro's helpers call `to_json` through `try_new`; the generated code otherwise stays the same.

Verified by: `tests/typed_params.rs` (task 2.3, failing before): `score::equals(f64::NAN)` and `score::not(f64::NAN)` do
not produce `null`; on SQLite, `delete_many(vec![score::not(f64::NAN)])` over rows `1.0`, `2.0`, `null` leaves the rows
the recorded original leaves; a value whose `Serialize` fails makes `find_many`, `update_many` and `delete_many` fail
with a validation error and run no statement; one test per helper family (`equals`, `not`, `in_vec`, `not_in_vec`,
`lt`/`lte`/`gt`/`gte`, `set`, `increment`/`decrement`/`multiply`/`divide`, `has`/`has_some`/`has_every`, `push`).

Alternative considered: make every helper return `Result`. Rejected: it changes every call site of every app; the deferred
error is the mechanism `Data` already uses.

### 5. No panic on caller input

- `Decimal`: remove `From<&str>`, `From<String>`, `From<&String>`; add `TryFrom<&str>` and `TryFrom<String>`
  (`Error = DecimalError`), next to `FromStr` and `Decimal::parse`. Helpers taking `impl Into<Decimal>` therefore stop
  accepting text at compile time. For literals, `corvis_orm::decimal!("1.50")` (a function-like proc macro in
  `corvis-orm-macros`, re-exported like `sql!`) parses the literal at compile time with the same grammar and fails the
  build on invalid text; it expands to a constructor that cannot fail at run time. A `compile_fail` doctest pins it.
- `RawSql::join` of an empty list returns a `RawSql` that carries the original's message ("Expected `join([])` to be called
  with an array of multiple elements, but got an empty array"); `_query_raw`, `_query_raw_as` and `_execute_raw` fail with
  a validation error carrying it, before any SQL; composition (`sql!` interpolation, `append`) propagates it.
- In-repository callers (`decimal.rs` tests, `tests/decimal_js.rs`, `tests/pg_features.rs`, `tests/pg_oracle.rs`, docs,
  README) move to `decimal!` or `parse`.

Breaking, now, before 1.0: code that passes `&str`/`String` where `impl Into<Decimal>` is expected stops compiling, with
the fix being `decimal!("..")` for literals or `.parse::<Decimal>()?` for input. The port apps' call sites are updated in
task 7.3.

Alternative considered: keep `From<&str>` and return NaN on invalid text. Rejected: a NaN filter silently matches
nothing or everything (Decision 4's problem again).

### 6. Key lists within the parameter limit

One internal helper, `Engine::key_condition(q, alias, fields, keys)`, writes "the row's key is one of these keys":

- One key field: through `push_in_list` (one parameter for long lists: `= ANY($1)` / `json_each(?)`, as today).
- Compound keys: an `OR` of `AND` groups, as today, but the caller splits the keys into chunks of at most
  `floor(budget / fields)` keys, where `budget` is half the dialect's parameter limit (32766 on SQLite, 65535 on Postgres),
  leaving the other half to the rest of the statement. Each chunk is a separate statement, inside the caller's
  transaction where there is one; results are merged in the caller's order.

Callers moved to it: `read_back_many` (ids in order, chunked), `ids_filter` and its four call sites (the update by ids
runs per chunk and sums the counts), `m2m_unlink`, the compound include (`read.rs:580-599`: children fetched per chunk of
parents; the non-paginated include has no `take`/`skip`, so per-parent order and contents are unchanged) and
`count_related` (counts merged). `in`/`notIn` with `mode: insensitive` (`filter.rs:314-323`) goes through `push_in_list`
with the folding inside the list (`LOWER(col) IN (SELECT LOWER(value) FROM json_each(?))` on SQLite,
`LOWER(col) = ANY(SELECT LOWER(x) FROM unnest($1::text[]) AS x)` on Postgres), which is the same `LOWER` per item as today.
The nested `delete`/`disconnect` lists of the caller (`write.rs:842`) are chunked the same way.

Verified by: `tests/parameter_limits.rs` (task 2.4, failing before with the database's "too many SQL variables" /
"too many arguments" error): on SQLite, `createManyAndReturn` of 40,000 rows returns 40,000 rows in input order;
`updateManyAndReturn` of 40,000 rows; an include over a two-column relation with 20,000 parents; a nested `set` of 40,000
children; `in` with `mode: insensitive` and 40,000 values. The Postgres versions run when Postgres is available, with
70,000 rows. Statement counts are asserted, not durations.

Alternative considered: row values `(a, b) IN (SELECT .. FROM json_each(?))` / `unnest`. Rejected for now: typed arrays per
column with enum, `uuid` and `date` casts on Postgres, and a different SQL shape per dialect, for a case (tens of thousands
of compound keys) that chunking already handles. `q-orm-hot-path` may revisit it with numbers.

### 7. Dialect differences settled by recording

- Recorded on both dialects (Postgres appended to `full/cases.js`, SQLite in the new recorder): `divide: 0` on an `Int`,
  a `Float` and a `Decimal` field (nullable and not); `increment` of an `Int` past `2147483647`; `contains` with
  `mode: insensitive` on non-ASCII text (`José` vs `JOSÉ`); a JSON filter whose path segment is `"0"` on an object; a
  `search` with two terms.
- Rule: corvis-orm's outcome (result, or error class, code and message) equals the recording on each dialect. Where today
  it differs, the engine changes on that dialect only (for example: if the original fails `divide: 0` on SQLite with a
  given error, `set_expr` checks a literal zero divisor there and raises that error before any SQL; if the original
  reports an out-of-range `Int` on SQLite, `increment` on an `Int` field adds the `CAST` and range check the recording
  implies). Where the recording shows the original's dialects differ too, the difference stays and is documented.
- A table "PostgreSQL and SQLite differences" in `docs/data/corvis-orm.md` (and the Spanish page) lists each item with
  both behaviors: case folding (ASCII only on SQLite), JSON path segments (a numeric segment in list form is an array
  index on SQLite; use the string form `$."0"` for an object key), `search`, `Decimal` precision (15 significant digits on
  SQLite), and the two arithmetic cases as recorded.

### Postgres stays unchanged

Every decision above that edits SQL generation (2, 6 and possibly 7) keeps Postgres output identical, except where
Decision 6 replaces a statement that failed. Task 1.2 records, before any change, the Postgres SQL text and parameter
types produced by `where_to_sql` and by the read, aggregate and update builders for every case of the Postgres oracle
(a unit test with a lazily connected Postgres client; nothing is sent); task 4.2 compares after. The Postgres oracle, the
houseplan test and the parity check of the three apps confirm the results.

## Security invariants that must hold

From the hardening review of the ORM and the archived changes that parameterized JSON paths, rejected NUL characters,
added the `schema` and `connect_timeout` URL parameters. Each decision that comes near one keeps a test that pins it:

| Invariant | Source | Decisions near it | Pinned by |
|---|---|---|---|
| Every identifier quoted, every value a parameter; no filter produces a syntax error | hardening review; `orm/query-filters` | 2 (`CAST` text), 3 (DDL), 6 | `tests/fuzz_filters.rs`, `tests/security.rs`; new: the `CAST` wraps only a quoted column, never a value; DDL identifiers only through `quote_ident` (unit test with a model and enum named with `"` and `;`) |
| JSON paths are one bound parameter; NUL rejected before SQL | parameterize JSON paths; NUL characters | 6 (insensitive lists) | `tests/pg_json_paths.rs`, `tests/nul_characters.rs` unchanged |
| Argument depth bounded before any recursion | hardening review | 4 (errors carried through nested helpers) | `deep_where_is_rejected_before_it_overflows_the_stack` |
| A write repeats its filter in the statement (optimistic concurrency) | hardening review | 6 (updates by id chunks) | `update_applies_its_filter_atomically_*`, `update_many_optimistic_concurrency_has_one_winner_postgres`; a chunked update keeps `AND (<where>)` in each statement (new unit test on the SQL) |
| Out-of-range numbers are errors, never truncated; SQLite decimals bounded | hardening review | 2 | `sqlite_decimal_columns_are_bounded_like_decimal_65_30`, `postgres_decimal_is_never_truncated`, oracle `sec.decimal_*` |
| Unique lookups take values only | hardening review | 4 | `where_unique_only_accepts_values_on_unique_fields`, oracle `sec.unique_*` |
| Long `in` lists use one parameter | hardening review | 6 | oracle `sec.in_long*`, `sec.include_long` |
| The `schema` name is always a quoted identifier | `schema` URL parameter | 3 (`CREATE SCHEMA` for `@@schema`) | `pg_schema_url.rs` hostile-schema test; new: a model `@@schema` with `"` in its name |
| A panic never crosses into the database driver | hardening review | 2 (no collation callback is added) | design: Decision 2 adds no callback |

`dbgenerated("..")` and `Unsupported("..")` texts are written into DDL as they are: they come from the developer's own model
definitions (the derive or the generated code), never from a request, as the original does with the same schema text.
The security review (task 10.2) checks that no run-time value can reach them.

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| SQLite `Decimal` comparison, `orderBy`, `_min`, `_max`, `having` on `db push` tables | Text order | Numeric, as the original | Results change on SQLite only; they become correct |
| `db push` on SQLite, `Decimal` column type | `TEXT` | `DECIMAL` (new tables only) | Values keep 15 significant digits, as with the original's migration |
| `db push` on Postgres | Partial schema (see table above) | The reference schema | New tables gain enum types, arrays, native types, `TIMESTAMP(3)`, join tables, compound keys, indexes, names; existing tables untouched |
| `db push` with a construct it cannot build | Silently wrong schema | Validation error before any statement | Apps that relied on a partial push see an error naming the field |
| `db push` failing midway | Half a schema | Nothing created | Re-running works |
| Typed helper with `NaN`/infinite floats | `null` (`IS NULL`/`IS NOT NULL`) | As recorded: value semantics, or a validation error | Filters mean what they say |
| Typed helper whose value fails to serialize | `null` | Validation error, no SQL | |
| `Decimal::from("..")`, `"..".into()` | Compiles, panics on bad text | Does not compile; `decimal!`, `parse`, `TryFrom` | Breaking (proposal "Impact") |
| `corvis_orm::to_json` | Returns `Value` | Returns `Result<Value, OrmError>` | Breaking for direct callers (generated code is regenerated by the derive) |
| `RawSql::join([])` | Panic | The query fails with a validation error | |
| Engine-built key lists past the parameter limit | Database error | Chunked; succeeds | |
| `divide: 0`, `Int` overflow in `increment` | Dialect-dependent | As recorded on each dialect | Documented table |

## Risks / Trade-offs

- The recording shows the original's SQLite `Decimal` behaves otherwise than expected → Decision 2's rule (equal the
  recording) still holds; the implementation is revised in task 4.1 and the change records why in "Implementation
  notes".
- `CAST` prevents index use for `Decimal` range predicates on SQLite → accepted; stated in the docs table.
- The catalog comparison is strict and may find more differences than listed → each is either fixed in group 5 or listed
  in the test with its reason; none is silently ignored.
- Removing `From<&str>` breaks downstream code → stated as breaking; `decimal!` keeps literals as short as before.
- Chunked statements make a very large `createManyAndReturn` read back in several statements → inside its transaction, so
  the result is the same; performance is `q-orm-hot-path`'s.
- `TIMESTAMP(3)` instead of `TIMESTAMPTZ(3)` for new pushed Postgres tables → model queries already bind `timestamp`
  (`engine/mod.rs:421`) and sqlx sets the session time zone to UTC, so model results are the same; raw queries on new
  tables see `timestamp` like an app migrated with the original's migration.

## Open questions

1. Should SQLite get exact decimals (the review's collation proposal) instead of the original's numeric behavior?
   Recommendation: no, parity first (Decision 2); a follow-up only if asked.
2. Should `db push` keep `TIMESTAMPTZ(3)` for `DateTime` to avoid a difference between old and new pushed databases?
   Recommendation: no, follow the reference migration.
3. Should scalar lists on SQLite be refused by `db push` since the original rejects them there? Recommendation: no; the
   engine supports them as JSON text and the examples may use them; document it as a Corvis extension.
4. If the original turns `NaN` into `null`, refuse non-finite floats in typed helpers (Decision 4) or reproduce `null`?
   Recommendation: refuse.
5. Update the port apps' `Decimal::from("..")` call sites in this change, or leave it to the maintainer? Recommendation: in
   this change (task 7.3), with backups, since the parity check cannot build them otherwise.

## Review notes

These settle the open questions and override the design where they differ.

1. SQLite `Decimal` follows the recorded original; no exact-decimal extension in this change.
2. `DateTime` is `TIMESTAMP(3)` in new pushed Postgres tables, as in the reference migration.
3. Scalar lists stay supported on SQLite as JSON text; the docs table marks them as a Corvis extension.
4. If the recording shows `null`, typed helpers refuse non-finite floats with a validation error.
5. Task 7.3 updates the port apps' call sites (backups first; the apps are not git repositories) and lists the files in
   "Implementation notes".
6. The three failing tests of the review's must-fix items (tasks 2.1, 2.2, 2.3) are written and seen failing before any fix,
   and their failure output is copied into "Implementation notes".
7. The security review (task 10.2) is mandatory: DDL generation, the `CAST` insertion, chunked statements and the deferred
   errors.

## Implementation notes

### Task 1.1: re-check against `dev`

- `dev` at implementation time (`926ffb6`, then `869de7c`) has no change under `crates/` since the review's `d46758a`
  (`git diff --stat d46758a HEAD -- crates/` is empty): every file and line of "The findings, confirmed against the
  code" still holds as written. Nothing was dropped.
- `CORVIS_ORM_PG_REQUIRED=1 cargo test -p corvis-orm --no-fail-fast` on the untouched tree (local `corvis-pg` on
  `127.0.0.1:55432`): 207 passed, 0 failed, 14 ignored (doc tests marked `ignore`).

### Recorders outside the repository (groups 3, 9 and the recorded parts of 4-6)

The Postgres oracle's recorder is not in this repository: it is `C:\Users\Usuario\Devel\corvis-tests\tmp\orm\oracle\full\`
(`cases.js`, `regen.sh`, `migration.sql`, `schema.prisma`, `client/` with the generated original client), and the
original client 6.x is installed in `C:\Users\Usuario\Devel\corvis-tests\tmp\orm\node_modules`. Only its outputs are
committed here (`tests/fixtures/oracle/`). The implementer of this change was told not to record anything there, so no
new recording was made: the tasks that depend on one (3.1, 3.2, 3.3, the oracle parts of 4.1, 5.3, 5.5 and 6.2, the
choice of 6.1 for non-finite floats, and 9.1) are left open, and each is marked below.

### Task 1.2: how the Postgres SQL snapshot is taken

`src/engine/pg_sql_snapshot.rs` runs the model cases of the Postgres oracle (the `$raw` cases send the caller's own
SQL and are left out) in order on a fresh database built from the recorded migration, and records every statement the
client sends for each case (text and parameter types, never values) through a test-only recorder
(`src/engine/recorder.rs`, called from `OrmClient::log`). Deviation from "a lazily connected client, nothing sent": a
client that sends nothing gets no rows back, so the statements that follow a read (the `UPDATE` after the lookup of the
row, read-backs, includes, counts, nested writes, `m2m` unlinks) would never be built, and those are the builders
Decision 6 edits. Recording the real run covers all of them: 199 cases, 345 statements. Like the other Postgres tests
it is skipped without a server unless `CORVIS_ORM_PG_REQUIRED=1`; `CORVIS_ORM_UPDATE_PG_SNAPSHOT=1` rewrites the
fixture. The same recorder counts statements in the unit tests of Decision 6.

### Group 2: the failing tests, as they failed on the untouched engine

- `tests/sqlite_decimal.rs` (2.1): on the pushed table and on a `TEXT` column (the layout of earlier pushes),
  `gte 100` returned `["99.5", "100", "1000"]` instead of `["100", "1000"]`; `db_push_declares_decimal_columns_as_decimal`
  failed (`"amount" TEXT NOT NULL`). Also found: on a `DECIMAL` column (the original's migration), `having` on
  `_sum` returned `[]` instead of `["a", "b"]`: the aggregate has no affinity and the operand is bound as text, so SQLite
  compared a number with a string. The fix casts the bound operand of a `Decimal` aggregate filter on SQLite.
- `tests/db_push.rs` (2.2): `schema_sql::<Pair>()` had `PRIMARY KEY ("a")`; two pairs sharing `a`: `P2002 Unique
  constraint failed on the fields: (`a`)` (both dialects); nested `categories: { create }`: `P2021 The table
  `_CategoryToPost` does not exist` (Postgres; on SQLite the table name in the message is empty); `User` with
  `role: "ADMIN"`: `42704 type "role_t" does not exist` (also for `scores`, since the default role is cast); a
  `@db.Uuid` filter on `Group`: `42883 operator does not exist: text = uuid`; every oracle model in one push:
  `42830 there is no unique constraint matching given keys for referenced table "Pair"` on Postgres, and `foreign key
  mismatch - "Entry" referencing "Pair"` on the first SQLite insert. The test asserts `PRIMARY KEY ("a","b")`, the
  reference migration's spelling (no space), rather than the task's `("a", "b")`.
- `tests/typed_params.rs` (2.3): `score::equals(NaN)` matched the `null` row; `delete_many(vec![score::not(NaN)])`
  left `[None]` (it deleted `1.0` and `2.0`); `update_many` by `equals(NaN)` changed the `null` row; `set(NaN)` stored
  `null`; every helper family with a value whose `Serialize` fails ran as a `null` filter or wrote `null`
  (`increment: expected a validation error, got Ok(BatchPayload { count: 1 })`, ...). `in_vec`/`not_in_vec` already
  failed with a validation error, by accident: the list became `null` and `in: null` is rejected.
- `tests/parameter_limits.rs` (2.4), SQLite: `createManyAndReturn` (40,000), `updateManyAndReturn` (40,000), the nested
  `set` and `in` with `mode: insensitive` failed with `too many SQL variables`; the compound include failed earlier with
  `foreign key mismatch - "entries" referencing "pairs"` and the `m2m` disconnect with `P2021` (both are the `db push`
  bugs of 2.2). On Postgres every test failed at the push (`42830`, the compound key). They were run again after group 5,
  before group 8, to see the parameter-limit errors themselves (below).

### Group 4: `Decimal` on SQLite

- One helper, `Engine::operand` (with `Engine::numeric_operand`), writes a column as an operand: `CAST(t0."c" AS
  NUMERIC)` for a non-list `Decimal` on SQLite, the plain quoted column otherwise. Used by `scalar_filter` /
  `scalar_ops` (`=`, `<>`, `<`, `<=`, `>`, `>=`, `in`, `notIn`), `cursor_sql`, `order_expr_sql` (fields, fields through
  to-one relations, `groupBy` aggregates other than `COUNT`), `agg_expr_sql` (`_sum`, `_avg`, `_min`, `_max`) and
  `set_list_sql` arithmetic. `IS NULL` / `IS NOT NULL` keep the plain column. `db push` declares `DECIMAL`.
- Beyond the design: a `having` filter on a `Decimal` aggregate binds its operand as `CAST(? AS NUMERIC)` on SQLite
  (`bind_agg_operand`). The aggregate has no affinity, so without it the number was compared with the text parameter,
  even on a `DECIMAL` column (the test of 2.1 found it). The cast wraps a bound parameter, never a value in the text.
- Not yet checked against the recording (3.1 is open): that the original's SQLite results for these cases are the
  numeric ones. The rounding and bound of `data_value` are kept; its comment now says what SQLite stores.
- Verified: `--test sqlite_decimal` (4 tests, on a pushed table, a `DECIMAL` column and a `TEXT` column of an earlier
  push), the Postgres SQL snapshot unchanged, `pg_oracle` and `pg_houseplan` (run with `CORVIS_TESTS_DIR`) pass; the
  whole `corvis-orm` suite fails only in the not yet fixed tests of group 2.

### Group 5: `db push`

- `schema.rs` builds a `PushPlan` (schemas, enum types, tables with their indexes and foreign keys, join tables) and
  refuses unknown constructs while building it, so nothing runs before a refusal. `PushPlan::statements` leaves out what
  exists: on Postgres the client looks each schema, enum type and table up first (`pg_namespace`, `to_regtype`,
  `to_regclass`, inside the push transaction, through the `search_path` like the queries), because `CREATE TYPE` and
  `ADD CONSTRAINT` have no `IF NOT EXISTS`; a table that exists keeps its indexes and foreign keys too. On SQLite every
  statement is `IF NOT EXISTS` and foreign keys stay inline.
- `db_push_models` and `db_push_force_reset` run `ensure_schema` and every statement in one transaction without
  `maxWait` / `timeout` (`ddl_transaction`). `db_push_force_reset` also drops the join tables between the given models;
  it does not drop enum types (other tables may use them).
- Decisions where the design left room: the native types refused on Postgres are those outside the original's Postgres
  list (all of `Text`, `Char`, `VarChar`, `Bit`, `VarBit`, `Uuid`, `Xml`, `Inet`, `Citext`, `Boolean`, `Integer`,
  `SmallInt`, `BigInt`, `Oid`, `DoublePrecision`, `Real`, `Decimal`, `Money`, `Timestamp`, `Timestamptz`, `Date`,
  `Time`, `Timetz`, `Json`, `JsonB`, `ByteA` are built; `Timetz`, `Integer`, `BigInt`, `Boolean` go slightly beyond the
  design's list because the engine already reads them). On SQLite native types and index types are ignored, as before:
  the oracle models (Postgres types) are pushed on SQLite by the tests and by apps that test on SQLite. An `@@index` with
  sort orders or operator classes cannot be refused at push time: `#[derive(Model)]` resolves index fields to field names,
  so `IndexMeta` never carries those options. A model without `@id` / `@@id` gets no primary key (only its unique
  indexes), like the reference. Scalar lists are nullable columns on both dialects, like the reference. A join table is
  created when both of its models are in the pushed set; it lives in the schema of model `A`.
- SQLite layout without a recording (5.3 open): types as before plus `DECIMAL`; join tables with a unique index
  `_<name>_AB_unique` and the `B` index; `DEFAULT CURRENT_TIMESTAMP` for `now()`, enum defaults by their mapped value.
- Behaviour changes, each with a test: `DateTime` is `TIMESTAMP(3)` on Postgres and `@default(now())` is written as
  `DEFAULT CURRENT_TIMESTAMP` (`tests/client.rs` `postgres_sql_generation` updated); enum types, native types, arrays,
  compound keys, join tables, indexes and names (`tests/db_push.rs`); refusals and atomicity (same file).
- Verified: `tests/db_push.rs` 21 tests, among them the catalog comparison with the reference migration (no difference at
  all), the houseplan models against the houseplan migrations (one accepted difference, listed in the test: the hand-written
  `embeddings_hnsw` index, which `schema.prisma` does not declare) and `pg_oracle`'s new run on a pushed database (all 225
  cases equal to the recording).

### Task 2.4 again, after group 5 and before group 8

With `db push` fixed, `tests/parameter_limits.rs` failed for the reason of the finding on both dialects. Postgres:
`PgConnection::run(): too many arguments for query: 70000` (`createManyAndReturn`, `updateManyAndReturn`, the
insensitive `in`), `70001` (nested `set`, `m2m` disconnect) and `140000` (compound include). SQLite: `too many SQL
variables` (insensitive `in`, nested `set`) and, new, `Expression tree is too large (maximum depth 1000)` for the
read-backs, the compound include and the nested `deleteMany`: SQLite parses `a OR b OR ...` as a left-deep tree and
rejects expressions deeper than 1000 levels (`SQLITE_MAX_EXPR_DEPTH`), so on SQLite an `OR` of key groups fails near
1,000 keys, long before the parameter limit. Group 8 sizes SQLite chunks for that too (below).

### Group 6: typed helpers

- `to_json` returns `Result<Value, OrmError>`: a `Serialize` failure is a validation error naming the type; a non-finite
  float anywhere in the value (alone, in a list, an `Option`, a struct or a map) is found first by a serializer that only
  checks floats (`query::finite::Check`), because `serde_json` turns it into `null` without an error.
- Non-finite floats are refused (Review note 4's branch), provisionally: the recording of 3.2 that decides between
  refusal and the engine strings `"NaN"` / `"Infinity"` was not made (see "Recorders outside the repository"). Refusal is
  the branch that is right in both outcomes as far as data goes: it never matches or writes rows the caller did not mean;
  if the original sends the values, the change is to map them to the engine strings in `to_json`'s check instead.
- `WhereParam`, `SetParam`, `OrderByParam`, `IncludeParam` and `SelectParam` carry `error: Option<OrmError>`; `new` keeps
  its signature, `try_new` and `WhereParam::try_field` take a `Result`, `error()` reads it, and `into_pair` now returns
  `Result<(String, Value)>` so that it cannot drop the error (no caller in the repository or in the port applications).
  `where_object` and `data_object` return `Result<Value>` (first error), for the same reason; `and`, `or`, `not`, the
  nested relation helpers (`some`, `every`, `none`, `is`, `is_not`, `connect`, `create`, `include_where`), compound unique
  helpers, `Args`, `WhereArg`, `DataArg` and the builder methods (`filter`, `cursor`, `order_by`, `include`, `select`,
  `data`, `create`, `update`) pass it on to `Query::with_error`, which fails the query before any SQL. Breaking for direct
  callers of `to_json`, `where_object`, `data_object` and `into_pair`: none in the repository or the port applications.
- Tests: `tests/typed_params.rs` (12: the `NaN` cases and one per helper family, relations and combinators included) and
  three unit tests in `query.rs`. Not changed: `or_undefined` (a JSON builder for `json!` arguments, not a typed helper)
  still turns a value that fails to serialize into `null`; proposed as a follow-up.

### Group 7: no panic on caller input

- `Decimal` loses `From<&str>`, `From<String>`, `From<&String>`; gains `TryFrom<&str>` and `TryFrom<String>` (`Error =
  DecimalError`), next to `FromStr` and `parse`. `DecimalCtor::of` takes `impl Into<Decimal>` too, so `D.of("..")` stops
  compiling as well (use `D.parse("..")`); this was not in the design's list and affects facten (see the port apps list).
- `decimal!("1.50")` (`corvis-orm-macros/src/decimal_literal.rs`) parses the literal at compile time with the grammar of
  `Decimal::parse` and expands to `Decimal::from_literal_parts(negative, &[limbs], exp)` (a hidden, infallible constructor
  taking the coefficient as 32-bit limbs), `Decimal::nan()` or `Decimal::infinity(neg)`: nothing is parsed at run time.
  A unit test checks that `decimal!(x)` and `Decimal::parse(x)` give the same value and sign for 24 literals (zeros with
  sign, exponents out of range, hex/binary/octal, long coefficients); a `compile_fail` doctest pins the invalid literal.
  One difference, on purpose: `Decimal::parse` hands `0x`/`0b`/`0o` digits to `num-bigint`, which also accepts `_` and a
  leading `+` (`"0x+1f"`, `"0x1_f"`); decimal.js rejects both, and so does `decimal!`. Proposed as a follow-up for `parse`.
- `RawSql` carries `error: Option<String>`: `join` of an empty list sets the original's message, `RawSql::new` with
  strings and values that do not alternate sets one too (it asserted before: a panic next to the one of the finding),
  nesting (`append`, so `sql!` interpolation) keeps the first error, and `_query_raw`, `_query_raw_as` and `_execute_raw`
  fail with a validation error before anything is sent (`to_sql` returns `Result`). New `RawSql::error()`.
- In-repository callers moved to `decimal!` or `parse` (`decimal.rs` tests and module docs, `tests/decimal_js.rs`,
  `tests/pg_features.rs`, `tests/pg_oracle.rs`). These two panics had no failing test written before the fix: the
  `panic!` of `From<&str>` and the `assert!` of `join` were plain in the code, and the first one is now a compile error.

### Group 8: key lists within the parameter limit

- `Engine::statement_chunks(items, params_of)` splits a list into the groups of one statement each: at most half the
  database's parameter limit (16383 on SQLite, 32767 on Postgres) and, on SQLite only, at most 500 `OR`ed groups.
  Deviation from Decision 6: the design sized chunks by parameters only, but SQLite also rejects an `OR` chain of about
  1,000 groups (`Expression tree is too large (maximum depth 1000)`, seen in 2.4), so its chunks are bounded by depth too.
- `Engine::key_filters(fields, keys, shape)` (the design's `key_condition`, at the level of `where` filters so that every
  caller keeps its own SQL): a list that fits in one statement is written by the caller's `shape`, exactly as before; a
  longer single-field list is one `in` filter (one parameter past 1,000 items, through `push_in_list`); a longer compound
  list is one statement per chunk. Hence Postgres SQL changes only for lists that did not fit one statement: compound
  lists past 16383 keys (they failed) and single-field lists past 32767 keys (they worked up to 65535 and are now one
  `= ANY($1)`); on SQLite single-field lists past 500 keys (they failed past about 1,000).
- Moved to it: `read_back_many` (rows of all chunks, then in the given order), the four `ids_filter` updates (now
  `update_by_ids`, which sums the counts of its statements; each statement carries its own part of the key list in its
  `WHERE`), the compound include (children of each chunk grouped by parent; no pagination there, so each parent gets the
  same children), the compound `count_related` (counts merged), the nested `deleteMany` items (chunked by the parameters
  each item compiles to), `m2m_unlink` (through `push_in_list`, the same text for short lists) and `in` / `notIn` with
  `mode: insensitive` past 1,000 values (`LOWER(col) IN (SELECT LOWER(value) FROM json_each(?))`,
  `LOWER(col) = ANY(SELECT LOWER(x) FROM unnest($1::text[]) AS x)` and their negations).
- Statement counts are asserted in `src/engine/key_list_tests.rs` (SQLite, 1,200 keys, with the recorder): one read-back
  statement with one parameter, three child and three count statements for the compound include (1000, 1000, 400
  parameters), three `UPDATE`s for a compound `set` whose `WHERE`s hold their own keys, three `DELETE`s for a nested
  `deleteMany`, one statement with one parameter for an insensitive `in`.
