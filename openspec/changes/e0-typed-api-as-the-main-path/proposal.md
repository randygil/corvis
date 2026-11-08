# Proposal

Implements [randygil/corvis#9](https://github.com/randygil/corvis/issues/9) (epic [#1](https://github.com/randygil/corvis/issues/1)).

## Why

Many handlers in the examples and the docs return `HttpResult<Value>` and build bodies with `json!`, and several ORM
calls go through `.exec_json()`. Part of that is habit, but part is forced by gaps in the API:

- **Domain errors cannot be typed.** `HttpException` has a blanket `impl<E: std::error::Error> From<E>` that turns any
  error into a 500 (`crates/corvis/src/http/exception.rs:257`), so an application cannot write
  `impl From<AppError> for HttpException` for its own `thiserror` enum. Today the only ways to give a domain error a
  status are `map_err` at every call site or an exception filter that downcasts the cause. The ORM worked around the
  same wall with its own `OrmResultExt::http()` (`crates/corvis-orm/src/module.rs:259-272`).
- **Relation counts force JSON.** `include: { _count: ... }` has no typed helper and no result type, so
  `examples/blog-orm` lists users with `.exec_json()` and returns `Value` (`users_service.rs:18-41`).
- **Documented operations are missing.** `docs/data/corvis-orm.md:197` lists `create_many_and_return` and
  `update_many_and_return`, but `Delegate` has neither (`crates/corvis-orm/src/query.rs:514-545`); they are only
  reachable through the untyped `OrmClient::query_json` (`client.rs:357-367`).
- **Writing a DTO needs `json!`.** `create`/`update` accept JSON or `Vec<SetParam>`, so passing a validated DTO is
  `create(json!({ "data": dto }))` (`examples/blog-orm/src/users/users_service.rs:14`), and an update from an
  optional-field DTO is a manual loop over `SetParam`s (`users_service.rs:53-62`).

Typed success responses already work (any `T: Serialize` is serialized as JSON by the route macro,
`crates/corvis/src/__private.rs:264-292`), but the examples and docs rarely show it.

## What Changes

- **`HttpError` trait** in `corvis` (exported from the prelude): an error type declares its status and, optionally,
  its body. A handler can return `Result<T, AppError>` with `AppError: HttpError` and get that status, with the error
  kept as the exception's cause. A `ResultHttpExt::http()` extension converts `Result<T, E: HttpError>` into
  `HttpResult<T>` for use with `?` inside handlers that return `HttpResult`.
- **ORM additions** in `corvis-orm`:
  - `Delegate::create_many_and_return` and `Delegate::update_many_and_return`, returning `Vec<M>`.
  - A `Data<T: Serialize>` wrapper accepted wherever `create`, `update`, `upsert` and `.data()` take data:
    `db.user().create(Data(&dto))`.
  - Relation counts: `#[derive(Model)]` generates `<field>::count()` include helpers for to-many relations, a
    `<model>::Count` struct, and a `<model>::WithCount` alias of a new generic `corvis_orm::WithCount<M, C>`, read
    with `.exec_as::<Vec<user::WithCount>>()`. Several `count()` includes merge into one `_count`.
- **Examples** move to typed handlers where the response has a fixed shape (inventory in design.md), with
  byte-identical JSON output.
- **Docs** (English and Spanish): the typed path becomes the first example in `controllers.md`,
  `exception-filters.md` and `data/corvis-orm.md`; `Value` stays documented as the escape hatch.

## Capabilities

### New Capabilities

- `http/responses`: how handler return values become responses (typed values, `Json`, status tuples).
- `http/exceptions`: typed domain errors through `HttpError`.
- `orm/client`: typed delegate operations, DTO data and relation counts.

### Modified Capabilities

None.

## Impact

- `crates/corvis`: `src/http/exception.rs` (trait, constructor), `src/__private.rs` (one more conversion level),
  `src/lib.rs` (prelude). `crates/corvis-macros/src/controller.rs` only if the conversion call needs a different
  receiver depth.
- `crates/corvis-orm`: `src/query.rs` (actions, `Data`, `WithCount`, `_count` merge), `src/lib.rs` (exports).
  `crates/corvis-orm-macros/src/lib.rs` (count helpers and types).
- `examples/blog-orm`, `examples/cats-app`, `examples/tertulia`.
- Docs pages listed above, in `docs/` and `docs/es/`.
- Additive public API only; existing code keeps compiling and behaving the same, except that a handler returning
  `Result<T, E>` where `E` implements the new trait gets `E`'s status (no existing type implements it).
- Runtime code changes, so the corvis-parity check runs before archiving.

## Out of scope

- Rewriting every docs page that uses `Value`: only the three pages above lead with the typed path; others follow as
  pages are touched.
- Implementing `HttpError` for `OrmError` and retiring `OrmResultExt` (changes what a handler returning
  `Result<T, OrmError>` answers without the filter): open question, possible follow-up.
- A `#[derive(HttpError)]` macro.
- Inferring OpenAPI response schemas from handler return types (`#[api_ok_response(type = T)]` is still needed).
- Typed `aggregate` / `group_by` builders and outputs (they stay `Value`; docs show `.exec_as::<T>()`).
- Typed payloads in WebSocket handlers (`WsResponse`, `WsException::object`), interceptors (`Response::map_data` takes
  `Value -> Value`, `crates/corvis/src/http/response.rs:191`) and exception filter bodies.
- `HttpResult<Option<T>>` returning `None` answers a JSON `null` body (the serialize path,
  `__private.rs:275-280`), while `Option<T: IntoResponse>` answers an empty body (`response.rs:368-375`); checking
  which one the original adapter does for `null`, and aligning both: new issue.
- `HttpError` in `#[handlers]` (events, schedule, WebSocket messages), which share the conversion helpers
  (`crates/corvis-macros/src/handlers.rs:283-288`).
