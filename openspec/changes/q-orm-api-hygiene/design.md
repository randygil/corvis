# Design

## Context

Source: the engineering-quality review of `crates/corvis-orm` and `crates/corvis-orm-macros` filed under epic #187, read
on `dev` at `d46758a` (2026-11-11). Line numbers are those of `d46758a`; this change is implemented after
`q-orm-correctness` and `q-orm-hot-path`, which edit `query.rs`, `raw.rs`, `db.rs`, `client.rs` and the engine, so **task
1.1 re-checks every line below at implementation time**.

The parity rule of the module applies: what the original client returns (errors included) is settled by a recording, made
the way the existing topic fixtures are (a recorder under `corvis-tests/tmp/orm/<topic>/cases.js` with scratch clients
generated for Postgres and SQLite, writing a JSON object with `recorded_with` and `script` into
`crates/corvis-orm/tests/fixtures/`).

### The findings, confirmed against the code

| Finding | Where now | Verdict |
|---|---|---|
| S9 `migrate deploy` takes no lock | `migrate.rs:234-304` (reads the history, then per pending migration: insert a row, run the script on a closed-afterwards connection, mark it finished) | Confirmed, and worse than reported: the second of two concurrent deploys fails on the already-applied script and records a failed row, so every later deploy stops with `P3009` until the row is resolved by hand |
| N5 two reads of a migration | `migrate.rs:98` (checksum from `std::fs::read`), `migrate.rs:255` (script from `read_to_string`) | Confirmed: a file changed between the reads is recorded with a checksum that does not match what ran |
| N5 dead code | `migrate.rs:329` (`let _ = &r.id;`), `Row::id` (`migrate.rs:137`) read nowhere else | Confirmed |
| S10 no `source()` | `error.rs:362` (`impl std::error::Error for OrmError {}`), `error.rs:281` (`other => Self::unknown(other.to_string())`), `error.rs:201-217` (unmapped database errors become text) | Confirmed |
| S10 large errors | `error.rs:14-49`: `KnownRequestError` is two `String`, a `Value` and a `&str` (96 bytes), so `OrmError` is about 104 bytes; `#[allow(clippy::result_large_err)]` in `tests/fuzz_filters.rs:34`, `json_filters.rs:22`, `nul_characters.rs:27`, `pg_json_paths.rs:30` | Confirmed |
| S10 40P01 not mapped | `error.rs:179-183` maps only 40001 to `P2034` | Confirmed; what the original returns for 40P01 is settled by recording (Decision 2) |
| S10 codes as strings | `error.rs:17` (`pub code: String`) | Not taken: codes are strings in the original and the apps compare them as such (`e.code() == Some("P2002")`, 50+ sites in the port apps); an enum would add a conversion everywhere for no behavior. Dropped |
| N1 aliases | `client.rs:682-689` (`_query_raw_unsafe`, `_execute_raw_unsafe` call `query_raw`, `execute_raw`) | Confirmed for these two. **Wrong for `_transaction*`, `_query_raw`, `_execute_raw`** (`client.rs:522,537,605,610,647,652,658`): they are not aliases but the port's names for the original's `$transaction`, `$queryRaw` and `$executeRaw` (the typed forms), distinct from `query_raw`/`execute_raw` (the `Unsafe` forms); renaming them is a naming decision outside this audit |
| N1 internal variants | `db.rs:58-64` (`SqlValue::Cast`, `SqlValue::OutOfRange`, public through `RawArg::Value` and `RawSql::values()`) | Confirmed |
| N1 `pub mod util` | `lib.rs:54`, `util.rs` (id generators, base64) | Confirmed; not used by the port apps or the generated code |
| N1 enums that cannot grow | `db.rs:22,38`, `error.rs:44`, `meta.rs:8,51,76,89`, `raw.rs:27`, `transaction.rs:19`, `query.rs:148`, `decimal.rs:44` | Confirmed (none is `#[non_exhaustive]`) |
| N3 ids with fewer random bits | `util.rs:7-9` (`Uuid::new_v4().as_u128()`), `util.rs:72-84` (ULID: low 80 bits of that value) | Confirmed, with a corrected count: in `as_u128()` the version nibble is bits 76-79 and the variant bits 62-63, both inside the low 80 bits, so a ULID has 74 random bits, not 78. The nanoid and cuid generators read the same value 6 bits at a time, so the characters that cover those bits take fewer values |
| N4 `0 s` in the message | `db.rs:410-418` (`limit.as_secs()`), asserted by `db.rs:1168,1176` | Confirmed, but only reachable with sub-second limits, which only the unit tests use: the URL parameter is whole seconds. Kept as a nice-to-have |
| N7 large functions | `corvis-orm-macros/src/lib.rs:517-1157` (`expand`, about 640 lines), `engine/write.rs:604-907` (`nested_relation_writes`, about 300) | Confirmed |
| N7 duplicated doc | `db.rs:960-962` (two summary lines on `coerce_field_d`) | Confirmed |
| N7 decimal.js aliases | `decimal.rs:692-700,793-805` | Not a defect: `Decimal` ports decimal.js's API (its docs say so) and `tests/decimal_js.rs` compares it with recorded decimal.js output; the aliases are decimal.js's own. Dropped |
| Tests on wall-clock time | `db.rs:1132-1177` (fake futures timed with `Instant`), `pg_features.rs:36,53` (sleeps to outlive the transaction timeout), `pg_features.rs:81` (a holder sleeps 800 ms while another request waits), `client_pool_tests.rs:170-178` (5-second default) | Confirmed for the first three. The last one follows the rule of `orm/connection-pool` ("tight lower bound and an upper bound below the nearest value an incorrect implementation would produce") against a real silent socket, where a paused clock does not drive sqlx's own timers; kept, with a comment citing the rule |
| Filter fuzz checks syntax only | `tests/fuzz_filters.rs` | Confirmed: no result comparison |

## Goals / Non-Goals

**Goals:**

- Concurrent deploys that cannot leave the database in a failed state.
- Errors that keep their cause and cost two words.
- A public surface that can grow without breaking, with no internals or duplicates in it.
- Tests that do not depend on time, and a fuzz that compares results.

**Non-Goals:**

- Typed error codes; renaming the `$`-method family; removing decimal.js's aliases.

## Decisions

### 1. `migrate deploy` under the original's advisory lock

- Recording first (task 2.1, `corvis-tests/tmp/orm/migrate/cases.js` → `tests/fixtures/migrate_lock.json`): while a
  separate session holds `pg_advisory_lock(<id>)`, the original's `migrate deploy` is started; the recorder observes in
  `pg_locks` whether the deploy waits on an advisory lock and with which `classid`/`objid`, then releases the lock and
  records that the deploy finishes. The same on SQLite (does the deploy wait on anything when another connection holds an
  exclusive transaction?). The review names `72707369`; the recording decides.
- Postgres: `migrate_deploy` opens one direct connection (bounded by `connect_timeout`, like `ensure_database`), runs
  `SELECT pg_advisory_lock(<id>)` on it, then reads the history, applies the pending migrations as today (scripts still run
  on connections that are closed afterwards), and runs `SELECT pg_advisory_unlock(<id>)` and closes the connection at the
  end, on success and on error. Holding the lock on its own connection means a script that resets the session cannot drop
  it. Using the original's id means a deploy by the original and one by Corvis exclude each other too, which matters while a
  fleet is being migrated.
- SQLite: what the recording shows (if the original takes no lock, neither does Corvis, and the docs say that concurrent
  deploys against one SQLite file are not supported).
- The history is read only after the lock is held, so the second deploy sees the first one's rows and has nothing to apply.
- `read_migrations` keeps the bytes it hashed; `migrate_deploy` runs exactly those bytes (`String::from_utf8`, with an
  initialization error naming the file when they are not UTF-8). `Row::id` and the `let _` are removed.

Verified by: `tests/pg_migrate_lock.rs`: two `migrate_deploy` calls started together on the same database apply each
migration once, both succeed, the history has one finished row per migration and no failed row; a session holding
`pg_advisory_lock(<id>)` makes `migrate_deploy` wait (observed in `pg_locks` with a condition wait), and releasing it lets the
deploy finish; an error in a migration still releases the lock (a second deploy afterwards reports `P3009`, not a hang).
`tests/pg_schema_url.rs`'s "Scripts do not change the pooled sessions" scenario keeps passing.

### 2. Errors: boxed, with a cause, mapped as recorded

- `OrmError` keeps its four variants, each holding a `Box`: `KnownRequest(Box<KnownRequestError>)`, and so on. Size: two
  words. The `#[allow(clippy::result_large_err)]` attributes go.
- The four structs become `#[non_exhaustive]`; their fields stay public for reading; construction goes through the existing
  `OrmError::known`, `validation`, `init`, `unknown` and new `KnownRequestError::new` style constructors. A private
  `cause: Option<Arc<sqlx::Error>>` (`Arc` because `OrmError: Clone`) is filled by `from_sqlx`, `from_sqlx_raw` and
  `from_sqlx_other`; `impl std::error::Error for OrmError` returns it from `source()`.
- `Display` and `code()`, `message()`, `meta()`, `name()` are unchanged; `Debug` shows the same fields (boxes are
  transparent in `Debug` output).
- Recording (task 3.1, `corvis-tests/tmp/orm/errors/cases.js` → `tests/fixtures/errors.json`): a deadlock between two
  interactive transactions that lock two rows in opposite order (one of them gets 40P01), a serialization failure
  (40001, already mapped, as a control), and a statement timeout (57014) outside a transaction. The error class, code, meta
  and message corvis-orm returns for each must equal the recording; `error.rs` gains the mappings the recording shows.

Verified by: `tests/errors.rs`: `size_of::<OrmError>() <= 2 * size_of::<usize>()`; an unmapped Postgres error's `source()`
downcasts to `sqlx::Error` and its SQLSTATE is readable; the three recorded cases; all existing error assertions (the
oracles, `pg_*` suites) unchanged.

Alternative considered: an enum of codes (the review's proposal). Rejected (see the findings table).

### 3. A public surface that can grow

- `#[non_exhaustive]` on `OrmError`, `FieldKind`, `DefaultValue`, `RelationKind`, `ReferentialAction`, `Dialect`,
  `SqlValue`, `RawArg`, `IsolationLevel`, `SortOrder` and `Rounding`. The derive constructs variants (allowed) and does
  not match on them outside the crate (checked in task 4.1). A `compile_fail` doctest per enum pins it.
- `SqlValue::Cast` and `SqlValue::OutOfRange`: `#[doc(hidden)]`; with `#[non_exhaustive]` on the enum, code outside the
  crate cannot rely on the variant list. They stay variants (the engine needs them) but leave the documented API.
- `util` becomes `pub(crate)`; nothing outside the crate uses it (the derive and the generated code do not).
- `_query_raw_unsafe` and `_execute_raw_unsafe` are removed; their docs pointed to `query_raw` and `execute_raw` already.

### 4. Ids from the operating system's generator

`util::random_bytes(n)` reads from `getrandom` 0.4 (already in the tree through `uuid`). `cuid`, `cuid2`, `nanoid` and
`ulid` draw from it: a ULID's 80 random bits are 80 random bits; a nanoid takes each character from 6 fresh bits (rejection
not needed: the alphabet has 64 symbols). Format, length and alphabet are unchanged (the oracle and the app parity compare
shapes, not values). A `getrandom` failure (no entropy source) is reported as an `OrmError` by `default_value`, never a
panic.

Verified by: unit tests that generate 10,000 ids of each kind and check that every random character position takes every
symbol of its alphabet at least once (for the ULID's random part, all 32 symbols in every one of its 16 positions); format
tests unchanged.

### 5. Small items

- `connect_timed_out` prints whole seconds when the limit is a whole number of seconds (every production case: the URL
  parameter is whole seconds) and milliseconds otherwise; the two unit tests assert the new text.
- The duplicated summary line of `coerce_field_d` goes.
- `expand` is split into functions per concern (attribute parsing, field metadata, field helper modules, compound unique
  helpers, the `Count`/`WithCount` types, the `impl Model`), and `nested_relation_writes` into one function per nested
  operation (`create`, `createMany`, `connect`, `connectOrCreate`, `disconnect`, `set`, `delete`, `deleteMany`, `update`,
  `updateMany`, `upsert`), each a pure move. Check for the macro: a unit test in `corvis-orm-macros` that runs `expand` on the
  models of `tests/generated/{oracle,consi,facten,houseplan}.rs` and compares the token stream's text with a snapshot taken
  before the split (task 6.1 takes it first). Check for the engine: both oracles and the nested-write tests unchanged.

### 6. Tests that do not depend on time, and a fuzz that compares results

- `db.rs` unit tests with fake futures run on `#[tokio::test(start_paused = true)]`: the timeouts elapse on the virtual clock
  and the assertions keep their bounds without wall-clock time.
- `pg_features.rs`: the transaction that must outlive its timeout waits until the server shows the transaction gone (no
  backend of the test's transaction is `idle in transaction`, observed with a condition wait) instead of sleeping; the holder
  of the `max_wait` test keeps its transaction open until the test sends a release message after the second request failed.
- `client_pool_tests.rs:170-178` keeps its bounds, with a comment citing the requirement "Pool rules are verified without
  timing races".
- `fuzz_filters.rs` gains a differential property: the same generated filters (strings with every hostile character,
  integers, floats, booleans, `Decimal` values, nested `AND`/`OR`/`NOT`) run on SQLite and on Postgres over the same seeded
  rows must return the same ids, except for the differences documented by `q-orm-correctness` (case folding of non-ASCII
  text, which the generator then avoids under `mode: insensitive`). It runs when Postgres is available, with the same case
  count as the existing property.

## Security invariants that must hold

| Invariant | Source | Decisions near it | Pinned by |
|---|---|---|---|
| Migration scripts run on connections that never return to the pool; the history table is schema-qualified | `schema` URL parameter change | 1 | `pg_schema_url.rs` "Migration that resets the search path" unchanged |
| Migration commands are bounded by `connect_timeout` | connect-timeout change | 1 (the lock connection) | `pg_connect_timeout.rs` "Migration commands are bounded"; new: the lock connection to a silent server fails with `P1001` |
| No credentials in errors | connect-timeout change | 2 (cause kept) | existing `topsecret` assertions; new: `Display` of an error with a cause never prints the URL (the cause is reachable through `source()`, and sqlx's own errors do not carry the password) |
| Hostile schema names quoted | `schema` URL parameter change | 1 | `pg_schema_url.rs` hostile-schema test |
| Generated ids unpredictable | hardening review | 4 | new randomness tests |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| Two concurrent `migrate deploy` | Second fails and blocks later deploys with `P3009` | Second waits, then has nothing to apply | Safe parallel starts |
| A deadlock (40P01) | Unknown request error with the SQLSTATE in text | As recorded from the original | Retry logic keyed on the code works |
| `OrmError` | ~104 bytes, no `source()` | Two words, `source()` returns the driver error | Smaller `Result`s; causes inspectable |
| Public enums | Exhaustive | `#[non_exhaustive]` | Exhaustive matches outside the crate need a wildcard arm |
| `KnownRequestError` and siblings | Constructible with struct literals | Constructors only | Breaking for code that built them by hand |
| `corvis_orm::util`, `_query_raw_unsafe`, `_execute_raw_unsafe` | Public | Gone | Use `query_raw`, `execute_raw` |
| Timeout message for sub-second limits | `0 s` | Milliseconds | Text only |

## Risks / Trade-offs

- The advisory lock id differs from the original's → the recording decides it; the test then also checks interoperability.
- A deploy blocked behind a crashed holder → session advisory locks are released when the holder's connection ends; the
  docs say so.
- Boxing errors adds one allocation per error → errors are the exceptional path; the benefit is on every `Result`.
- `#[non_exhaustive]` breaks exhaustive matches downstream → stated; pre-1.0.
- The differential fuzz may find dialect differences not yet documented → each is either fixed (if it is a bug) or added to
  the documented table and excluded from the generator with a comment.

## Open questions

1. Lock on SQLite if the original takes none? Recommendation: follow the recording and document.
2. Keep `pub code: String` readable on `KnownRequestError`? Recommendation: yes (fields readable, structs
   `#[non_exhaustive]`).
3. Should `SqlValue` leave the public API entirely (it is reachable through `RawArg` and `RawSql::values()`)?
   Recommendation: no; hide the two internal variants and mark the enum `#[non_exhaustive]`.

## Review notes

These settle the open questions and override the design where they differ.

1. SQLite deploys follow the recording; if the original takes no lock, the docs say concurrent SQLite deploys are not
   supported.
2. Error struct fields stay readable; construction is through constructors.
3. `SqlValue` stays public with two hidden variants.
4. Implemented after `q-orm-hot-path` is merged; task 1.1 is mandatory and written in "Implementation notes" before any other
   task.
5. The security review covers the lock connection, the error cause (no secrets in `Display`) and the id generator.
