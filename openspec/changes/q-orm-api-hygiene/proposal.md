# Proposal

Implements the third issue filed from the engineering-quality review of `crates/corvis-orm` and
`crates/corvis-orm-macros` (epic [#187](https://github.com/randygil/corvis/issues/187), code quality audit), after
`q-orm-correctness` and `q-orm-hot-path`. Source: a read-only review of `dev` at `d46758a`; every finding was re-read
against the code (design.md "The findings, confirmed against the code"); task 1.1 re-checks the lines after the two earlier
changes merge.

## Why

- Two replicas that start at the same time both run `migrate deploy`: neither takes a lock, both insert a history row and
  run the same script, the second fails, and its failed row then blocks every later deploy with `P3009` until someone
  resolves it by hand. The checksum is also computed from one read of the file and the script run from another.
- Errors drop their cause (`OrmError` has no `source()`, and unmapped driver errors become text), every `Result` of the
  crate carries an error of about 100 bytes (four test files need `#[allow(clippy::result_large_err)]`), and a Postgres
  deadlock (40P01) is not mapped, while the message of the code it should map to says "or a deadlock".
- The public surface carries internals and duplicates: `SqlValue::Cast` and `SqlValue::OutOfRange` are public variants, a
  hand-written base64 and the id generators are a public `util` module, `_query_raw_unsafe` and `_execute_raw_unsafe` are
  aliases of `query_raw` and `execute_raw`, and no public enum can grow without a breaking change.
- `cuid()`, `nanoid()` and `ulid()` draw their randomness from UUID v4 values, whose version and variant bits are fixed:
  a ULID has 74 random bits instead of 80.
- Smaller items: a timeout message that prints `0 s` for sub-second limits (only tests use them), dead code in
  `migrate.rs`, a duplicated doc comment, two functions of 300 and 640 lines.
- Tests that synchronize with `sleep` or measure wall-clock time where a paused clock or a handshake would do, and a filter
  fuzz that checks only for syntax errors, so it could not catch a wrong result like the SQLite `Decimal` comparison.

## What Changes

- **Migration lock.** `migrate deploy` takes the same session advisory lock as the original's deploy (lock id confirmed by
  recording) on a dedicated connection for the whole deploy, reads the history after taking it, and releases it at the
  end; on SQLite it does what the recording shows the original does. The checksum is computed from the bytes that run.
- **Errors.** `OrmError` keeps its four variants, boxed (an `OrmError` is two words); the variant structs become
  `#[non_exhaustive]` with accessors and constructors; `source()` returns the driver error when there is one; 40P01 and any
  other code the recording covers are mapped as the original maps them. Error codes stay strings (`code()`), as the
  original's are and as the apps compare them.
- **Public surface.** `#[non_exhaustive]` on the public enums; `SqlValue::Cast` and `OutOfRange` hidden from the docs and
  from construction outside the crate; `util` private; the two aliases removed.
- **Ids.** The generators draw from `getrandom` directly; every random position of a cuid, nanoid and ULID uses its full
  alphabet.
- **Small items.** Milliseconds in the timeout message when the limit is not whole seconds; dead code and the second read in
  `migrate.rs`; the duplicated doc comment; `expand` and `nested_relation_writes` split into named functions, as pure moves.
- **Tests.** Paused clock for the unit tests with fake futures; handshakes instead of sleeps in `pg_features.rs`; a
  differential fuzz that runs the same generated filters on SQLite and Postgres and compares the rows.

## Capabilities

### New Capabilities

- `orm/migrations`: how `migrate deploy` behaves when several processes run it at once, and what the recorded checksum is.

### Modified Capabilities

- `orm/client`: added requirements for errors that keep their cause and stay small, unmapped database errors mapped as the
  original maps them, public enums that can grow, and ids that use the full randomness of their format.

## Impact

- Edited: `crates/corvis-orm/src/{error,migrate,db,client,util,lib,meta,raw,query,transaction,decimal}.rs`,
  `crates/corvis-orm/src/engine/write.rs`, `crates/corvis-orm-macros/src/lib.rs`, `crates/corvis-orm/Cargo.toml`
  (`getrandom`, already in the dependency tree through `uuid`), `crates/corvis-orm/tests/{pg_features,fuzz_filters,
  json_filters,nul_characters,pg_json_paths}.rs`, `docs/data/corvis-orm.md`, `docs/es/data/corvis-orm.md`.
- New: `crates/corvis-orm/tests/{pg_migrate_lock,errors}.rs`, `crates/corvis-orm/tests/fixtures/{migrate_lock,errors}.json`
  (recorded from the original).
- Public API (breaking, now, before 1.0): `#[non_exhaustive]` on `OrmError`, `FieldKind`, `DefaultValue`, `RelationKind`,
  `ReferentialAction`, `Dialect`, `SqlValue`, `RawArg`, `IsolationLevel`, `SortOrder`, `Rounding` (exhaustive matches
  outside the crate need a wildcard arm); `KnownRequestError`, `QueryValidationError`, `InitializationError` and
  `UnknownRequestError` become `#[non_exhaustive]` (fields readable, construction through constructors); `OrmError`
  variants hold `Box<..>`; `corvis_orm::util` is no longer public; `_query_raw_unsafe` and `_execute_raw_unsafe` are
  removed (use `query_raw` and `execute_raw`). None of the three port apps uses any of these (checked: no variant
  matching, no `util`, no alias); they use `code()`, `message()` and `downcast_cause`.
- Behavior visible to applications: concurrent deploys wait for each other; a deadlock maps to the recorded code; error
  `Debug` output shows boxes. Listed in design.md "Behavior visible to applications".
- Dependencies: `getrandom` 0.4 becomes a direct dependency (the version `uuid` already pulls in, `Cargo.lock` 0.4.3; no
  new crate in the tree). The parity check is run.

## Out of scope

- Typed error codes (an enum instead of strings): apps compare codes as strings, as with the original.
- Removing the decimal.js aliases of `Decimal` (`to_dp`/`to_decimal_places`, `eq`/`equals`, `cmp`): `Decimal` ports
  decimal.js's API on purpose, pinned by `tests/decimal_js.rs` against a recorded fixture.
- Renaming the `_transaction` / `_query_raw` / `_execute_raw` family: the port's names for the original's `$` methods, not
  aliases; a project-wide naming decision.
