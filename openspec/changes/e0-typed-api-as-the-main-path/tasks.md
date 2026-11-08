# Tasks

## 1. Typed responses: pin current behavior

- [ ] 1.1 Add regression tests in `crates/corvis/tests/app.rs` (or a new `tests/typed_responses.rs`): a handler
  returning `HttpResult<CatDto>` answers the struct as JSON with a JSON content type; a struct and the equivalent
  `json!` give byte-identical bodies; `(HttpStatus::ACCEPTED, Json(dto))` answers 202; `#[http_code(201)]` with a
  struct answers 201; verify with `cargo test -p corvis --test <file>`

## 2. `HttpError`

- [ ] 2.1 Add `pub trait HttpError` (`status()`, provided `body()` returning the `Display` text) and
  `HttpException::from_http_error` in `crates/corvis/src/http/exception.rs`, using `with_error` and `with_cause`;
  export the trait from `corvis` and the prelude (`crates/corvis/src/lib.rs`); unit tests for the default body, a
  custom `ExceptionBody::Object` body, `is_internal() == false` and `downcast_cause::<AppError>()`; verify with
  `cargo test -p corvis --lib exception`
- [ ] 2.2 Add `pub trait ResultHttpExt<T> { fn http(self) -> HttpResult<T>; }` for `Result<T, E: HttpError>`, exported
  from the prelude; unit test; verify that a scratch file importing both `corvis::prelude::*` and
  `corvis_orm::OrmResultExt` calls `.http()` on `Result<_, OrmError>` and on `Result<_, AppError>` without ambiguity
  (add it as a test in `crates/corvis-orm/tests/corvis_module.rs`)
- [ ] 2.3 Add the `HttpError` level to the handler error conversion in `crates/corvis/src/__private.rs` (and the call
  in `crates/corvis-macros/src/controller.rs:378-383` if the receiver depth changes), keeping the existing order;
  integration tests, one route per error kind: `Result<T, AppError>` (404 with the built-in body),
  `HttpResult<T>` with `NotFoundException::new`, `Result<T, NotFoundException>` (unit type), `Result<T,
  std::io::Error>` (500 internal), `anyhow::Result<T>` (500 internal), and `HttpResult<T>` with `.http()?` and with
  plain `?` on an `AppError` (404 and 500); verify with `cargo test -p corvis --test <file>`
- [ ] 2.4 Check that `#[handlers]` (`crates/corvis-macros/src/handlers.rs:283-288`) still compiles and behaves the same
  (no `HttpError` level there); verify with `cargo test -p corvis --test websockets --test schedule`

## 3. ORM: typed operations and DTO data

- [ ] 3.1 Add `CreateManyAndReturn` and `UpdateManyAndReturn` to `actions!` (`Vec<M>`) and `create_many_and_return` /
  `update_many_and_return` to `delegate_ops!` in `crates/corvis-orm/src/query.rs`; tests in
  `crates/corvis-orm/tests/client.rs` (SQLite) and `pg_features.rs` (Postgres) comparing with `query_json` results;
  verify with `cargo test -p corvis-orm --test client` (and `--test pg_features` with Postgres running)
- [ ] 3.2 Add `pub struct Data<T>(pub T)` with conversions into `Args<M>` (`{ data }`) and `DataArg<M>`, for `T:
  Serialize` and `&T`; carry a serialization error to a validation `OrmError` raised before any SQL; export from
  `corvis_orm`; tests: create from a DTO equals the `json!` version, partial update with `skip_serializing_if` fields
  changes only the present ones, a type whose `Serialize` fails returns a validation error and runs no query; verify
  with `cargo test -p corvis-orm --test client`

## 4. ORM: relation counts

- [ ] 4.1 Add `pub struct WithCount<M, C>` (`#[serde(flatten)] pub model: M`, `#[serde(rename = "_count")] pub count:
  C`) to `corvis-orm`, and make `Query::include` merge `_count.select` objects; unit test of the merged arguments
  (`.args()`); verify with `cargo test -p corvis-orm --lib query`
- [ ] 4.2 In `crates/corvis-orm-macros/src/lib.rs`, generate for each to-many relation `<m>::<f>::count()`, and per
  model `<m>::Count` (one `Option<i64>` per to-many relation, `#[serde(default)]`, renamed like the field) and
  `<m>::WithCount`; models without to-many relations get an empty `Count`; regenerate nothing by hand (the generated
  test models under `crates/corvis-orm/tests/generated/` come from the derive); verify with `cargo test -p
  corvis-orm --test codegen_schemas`
- [ ] 4.3 Tests on SQLite and Postgres: counts per user equal the `.exec_json()` result, two counts merge, a typed
  result serializes to the same string as `.exec_json()` for a model with `corvis_orm::DateTime`, and models with
  `i64`, `Decimal` and `Option` relation fields round-trip through `WithCount`; verify with `cargo test -p corvis-orm
  --test client` (and `--test pg_features`)

## 5. Examples

- [ ] 5.1 Before changing anything, record the responses of every endpoint touched below (a small script with
  `curl` against each example on 127.0.0.1, saved under the scratchpad, not the repository); verify the recordings
  exist
- [ ] 5.2 `examples/blog-orm`: models to `corvis_orm::DateTime`; `UpdateUserDto` fields get
  `skip_serializing_if = "Option::is_none"`; `create`/`update` with `Data`; filters with typed helpers; users list with
  `user::posts::count()` and `Paginated<user::WithCount>`; posts list with `Paginated<Post>`; stats with
  example-local structs read with `.exec_as::<T>()`; no `HttpResult<Value>` left; verify against the recordings
  (only the timestamp format of `createdAt`/`updatedAt` may differ, see design decision 8)
- [ ] 5.3 `examples/cats-app`: DTOs for sign-in, count and `find_one_v2`; keep `Value` in health, SSE, WS, filter and
  interceptor code; verify the responses are byte-identical to the recordings
- [ ] 5.4 `examples/tertulia`: DTOs for login, rooms list, announce and stats; typed helpers and `Data` in
  `rooms_service.rs`; keep WS payloads; verify the recordings match and `cargo test -p tertulia` passes

## 6. Documentation

- [ ] 6.1 `docs/controllers.md`: lead the return-value section with a struct example and the status-tuple form with
  `Json`; replace the fixed-shape `HttpResult<Value>` handlers (lines 212, 257-259, 297, 311-314) with structs; keep
  `Value` as the documented escape hatch; mirror in `docs/es/controllers.md`; verify with `npm run docs:check` in
  `docs/`
- [ ] 6.2 `docs/exception-filters.md`: new "Domain errors" section (`HttpError`, direct `Result<T, AppError>` return,
  `.http()?`, the default body, the warning about `Display` text in 5xx, filters with `downcast_cause`); update the
  `?` sentence at line 26; mirror in `docs/es/exception-filters.md`; verify with `npm run docs:check`
- [ ] 6.3 `docs/data/corvis-orm.md`: typed helpers first in "Use `OrmService`", `Data`, relation counts with
  `WithCount`, the two `_and_return` operations (line 197 becomes true), `aggregate`/`group_by` with `.exec_as::<T>()`;
  mirror in `docs/es/data/corvis-orm.md`; verify with `npm run docs:check`
- [ ] 6.4 Update `crates/corvis-orm/README.md` sections that list delegate operations or `_count`, if any; verify by
  grepping it for `createManyAndReturn` and `_count`

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 7.3 `cargo test --workspace` passes (with Postgres available for the `pg_*` suites)
- [ ] 7.4 Regression tests from tasks 1.1, 2.1-2.4, 3.1-3.2 and 4.1-4.3 are in place and pass
- [ ] 7.5 Docs updated in English and Spanish (`docs/` and `docs/es/`), and `npm run docs:check` in `docs/` passes
- [ ] 7.6 corvis-parity: framework runtime code changes (`corvis` error conversion, `corvis-orm` query builder and
  derive), so run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #9, tick it in epic #1 (and in the execution plan #111), and move its card to Done (see the
  `corvis-task` skill).
