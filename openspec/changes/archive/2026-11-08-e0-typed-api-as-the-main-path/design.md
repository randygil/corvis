# Design

## Context

### What the typed path already has

- **Responses.** The route macro converts a handler's return value with autoref specialization: `IntoResponse` first,
  then any `T: Serialize` through `serde_json::to_value` and `Response::json` (`crates/corvis/src/__private.rs:264-292`;
  the call is generated at `crates/corvis-macros/src/controller.rs:374-387`, which treats a return type whose last
  segment ends with `Result` as a result). So `HttpResult<User>` already answers JSON, as
  `examples/blog-orm/src/users/users_controller.rs:30-31` shows. `IntoResponse` covers `Value`, `String`, `()`,
  `Json<T: Serialize>`, `Option<T: IntoResponse>`, `(StatusCode, T)` and `(u16, T)` with `T: IntoResponse`
  (`crates/corvis/src/http/response.rs:316-400`); a status with a plain struct therefore needs `Json(...)`.
  `docs/controllers.md:140` documents "Any `T: Serialize` ... JSON".
- **Inputs.** `#[body]`, `#[query]`, `#[param]` deserialize into the declared type (`__private.rs:324-331`,
  `extract_arg`).
- **ORM results.** `Delegate<M>` operations return typed outputs: `find_many` → `Vec<M>`, `find_unique` →
  `Option<M>`, `*_or_throw`/`create`/`update`/`upsert`/`delete` → `M`, `*_many` → `BatchPayload`, `count` → `i64`
  (`crates/corvis-orm/src/query.rs:273-304`). `aggregate` and `group_by` return `Value`. Every query also has
  `.exec_json()` and `.exec_as::<T>()` (`query.rs:450-464`). Typed filters and setters are generated per field
  (`user::email::contains`, `post::views::increment`, `user::posts::include()`, ...;
  `crates/corvis-orm-macros/src/lib.rs:840-1050`).
- **ORM DateTime.** `corvis_orm::DateTime` serializes like a JavaScript `Date` (always three fraction digits and `Z`,
  `crates/corvis-orm/src/datetime.rs:1-11`); `chrono::DateTime<Utc>` does not, so a model using chrono serializes
  timestamps differently from `.exec_json()`.

### What is missing (each one pushes code to `Value`)

1. **Typed domain errors.** `impl<E: std::error::Error + Send + Sync + 'static> From<E> for HttpException`
   (`crates/corvis/src/http/exception.rs:256-261`) makes every error a 500 and, by coherence, forbids
   `impl From<AppError> for HttpException` for any `AppError: Error`. The handler conversion picks between
   `Into<HttpException>` and `Into<anyhow::Error>` (`__private.rs:294-318`); there is no hook for a status chosen by
   the error type. `docs/exception-filters.md:26,89-99` documents `?` and the 500 path; `docs/data/corvis-orm.md:245`
   explains the same coherence wall for `OrmError`, solved there with `OrmResultExt::http()` and
   `OrmError::to_http_exception()` (`crates/corvis-orm/src/module.rs:247-272`) plus `OrmExceptionFilter`.
2. **`createManyAndReturn` / `updateManyAndReturn`.** The engine runs them (`crates/corvis-orm/src/engine/mod.rs:48,51`)
   and `OrmClient::query_json` lists them (`client.rs:357-367`), but `delegate_ops!` (`query.rs:514-545`) has no
   method and `actions!` no action for them, while `docs/data/corvis-orm.md:197` lists both delegate methods.
3. **DTO as write data.** `Args<M>` and `DataArg<M>` convert from `Value`, `WhereParam`/`SetParam` vectors and `()`
   (`query.rs:170-233`); a `Serialize` DTO has to be wrapped in `json!`.
4. **Relation counts.** `include()` inserts one entry per key (`query.rs:402-412`), so two `_count` includes would
   overwrite each other; there is no generated count helper and no type for the `_count` object.

### Inventory: `Value` / `json!` in the examples (excluding `tests/`)

| Location | Use | Forced by the API? | Typed replacement |
|---|---|---|---|
| `blog-orm/src/users/users_service.rs:14` | `create(json!({ "data": dto }))` | yes (gap 3) | `create(Data(&dto))` |
| `blog-orm/src/users/users_service.rs:18-41`, `users_controller.rs:23` | list with `_count`, `.exec_json()`, `HttpResult<Value>` | yes (gap 4) | `Paginated<user::WithCount>` |
| `blog-orm/src/users/users_service.rs:22-26,65` | `where` filters in JSON | no (`user::email::contains_insensitive`, `or`, `user::id::equals` exist) | typed helpers |
| `blog-orm/src/users/users_service.rs:53-62` | manual `SetParam` loop for an optional-field DTO | partly (gap 3) | `.data(Data(&dto))` |
| `blog-orm/src/models.rs:71-74`, `posts_service.rs:30-51`, `posts_controller.rs:20` | `paginated(Value, ...)` envelope | no | example-local `Paginated<T>` struct |
| `blog-orm/src/posts/posts_service.rs:72-86`, `posts_controller.rs:25` | `aggregate`, `group_by` + combined stats | outputs are `Value` by design | example-local structs read with `.exec_as::<T>()` |
| `blog-orm/src/posts/posts_service.rs:17` | `create(json!({ "data": ... }))` | yes (gap 3) | `Data` or `SetParam`s |
| `cats-app/src/auth/auth_service.rs:14-21`, `auth_controller.rs:17` | `{ "access_token" }` | no | `TokenDto` |
| `cats-app/src/cats/cats_controller.rs:35-36` | `{ "count" }` | no | `CountDto` |
| `cats-app/src/cats/cats_controller.rs:65-67` | `{ "cat", "apiVersion" }` | no | struct |
| `cats-app/src/app_controller.rs:22-26` | health check result | the terminus API takes `Value` details | keep |
| `cats-app/src/cats/cats_controller.rs:50`, `chat.rs`, `common/filters.rs`, `common/interceptors.rs` | SSE data, WS payloads, filter body, `map_data` | WS / filter / interceptor APIs are `Value` | keep (out of scope) |
| `tertulia/src/auth/auth_service.rs:15-20`, `auth_controller.rs:18` | `{ "accessToken", "user" }` | no | `LoginResponse` |
| `tertulia/src/rooms/rooms_controller.rs:21,51-54` | room list, `{ "room", "delivered" }` | no | structs |
| `tertulia/src/rooms/rooms_service.rs:24,28,36,49,56,61,66,77` | JSON query arguments | no (typed helpers exist), except `create` (gap 3) | typed helpers, `Data` |
| `tertulia/src/stats/mod.rs:17-30` | stats snapshot | no | struct |
| `tertulia/src/chat/*`, `firehose.rs` | WS payloads and guard data | WS APIs are `Value` | keep (out of scope) |

### Inventory: docs (English pages; counts of lines with `Value` or `json!(`)

Highest: `controllers.md` (9), `openapi/operations.md` (8), `http/file-upload.md` (8), `pipes.md`,
`exception-filters.md`, `application/configuration.md`, `openapi/security.md`, `reliability/terminus.md` (5 each),
`data/corvis-orm.md` (4), 31 more pages with 1-4. In `controllers.md` the handlers at lines 212, 257-259, 297 and
311-314 return `HttpResult<Value>` for fixed shapes; `data/corvis-orm.md:166-176` leads the ORM chapter with JSON
query arguments and shows the typed helpers second. Many other uses are legitimate (`Value` bodies in filters,
`HttpException::new(json!(...))`, config values, OpenAPI examples).

## Goals / Non-Goals

**Goals:**
- Close the four API gaps so a fixed-shape endpoint never needs `Value`.
- Make the typed path the first thing the examples and the three core pages show.
- Additive API: no existing code changes behavior.

**Non-Goals:**
- Removing `Value` support anywhere, or rewriting every docs page.
- Changing how `OrmError` maps to HTTP (see Open questions).
- Typed WebSocket, interceptor or filter payloads; OpenAPI inference from return types.

## Decisions

1. **A trait for typed errors, not a change to the blanket `From`.** `HttpError` declares `status()` and an optional
   `body()`. Removing the blanket `From<E: Error>` would let users write `From<AppError>` but would break every `?`
   on an arbitrary error in existing handlers (documented behavior, `docs/exception-filters.md:89-99`) and the port
   apps. Alternatives considered: a registry of error-to-status mappings at startup (runtime lookup by `TypeId`,
   invisible at the call site); "use an exception filter with `downcast_cause`" (works today, but every error type
   needs a filter and a global registration). The trait is local, explicit and needs no registration.
2. **Two entry points: direct return and `.http()`.** A handler declared as `Result<T, AppError>` gets the status
   with no extra code: the route macro's conversion gains a level for `E: HttpError` above the existing
   `Into<HttpException>` / `Into<anyhow::Error>` levels. Because `AppError: HttpError` implies `AppError: Error`
   (so it also matches the anyhow level), the new level must win by receiver depth (for example, call the
   conversion on `&&__e` and implement the new kind for `&&E`); the implementer must keep the order of the existing
   levels and prove it with the tests of task 2.3. Inside an `HttpResult<T>` handler, `?` still goes through the
   blanket `From`, so `ResultHttpExt::http()` is provided, mirroring `OrmResultExt::http()`.
3. **Default body = built-in exception format.** `from_http_error` uses `HttpException::with_error(body, reason,
   status)` (`exception.rs:110-129`), so `AppError` answers like `NotFoundException::new(msg)`. The error is attached
   with `with_cause` (filters can downcast it) and `is_internal()` stays `false` because the status was chosen on
   purpose, even for 5xx; the docs warn that `Display` text reaches the client.
4. **`OrmError` does not implement `HttpError` in this change.** Doing so would make a handler returning
   `Result<T, OrmError>` answer 404/409 without `OrmExceptionFilter`, a behavior change for code (and possibly port
   apps) that returns ORM results directly. Recorded as an open question.
5. **`Data<T>` wrapper instead of a blanket `From<T: Serialize>`.** A blanket impl would overlap the existing
   `From<Value>`, `From<Vec<SetParam<M>>>` and `From<()>` impls. `Data(&dto)` is explicit and reads well at the call
   site. Serialization errors cannot be returned from `From`, so `Args`/`DataArg` carry a pending error that the
   query reports as a validation `OrmError` before touching the database (instead of `to_json`'s silent `Null`,
   `query.rs:24-27`).
6. **Relation counts as generated helpers plus a generic `WithCount<M, C>`.** Alternatives: a `_count` field on every
   model (changes every generated model and its `Serialize` output, and the docs/ports that compare JSON); returning
   `(M, Count)` tuples (does not serialize to the original shape). `WithCount` uses `#[serde(flatten)]` for the model
   and `#[serde(rename = "_count")]` for the counts, so it round-trips the engine's JSON. `include()` merges
   `_count.select` objects instead of overwriting them.
7. **Typed `create_many_and_return` / `update_many_and_return`.** Two `actions!` entries returning `Vec<M>` and two
   `delegate_ops!` entries; the engine already implements them.
8. **Examples convert only fixed-shape endpoints, with identical output.** Each new DTO lists its fields in the
   same order and with the same names (serde renames) as the `json!` it replaces. `blog-orm` models switch from
   `chrono::DateTime<Utc>` to `corvis_orm::DateTime` (as `tertulia` and the docs `Room` model already do), so
   endpoints moving from `.exec_json()` to typed results keep the same timestamp format. Endpoints whose shape comes
   from `aggregate`/`group_by` use example-local structs with `.exec_as::<T>()`.
9. **Docs: three pages lead with the typed path.** `controllers.md` (return types and the `Value` handlers listed in
   Context), `exception-filters.md` (new "Domain errors" section with `HttpError`, direct return and `.http()`),
   `data/corvis-orm.md` (typed helpers first, `Data`, counts, the two new operations, `exec_as` for aggregates). Each
   change mirrored in `docs/es/`.

## Risks / Trade-offs

- **Autoref ordering is fragile.** A mistake silently changes which conversion runs (for example, sending plain
  errors through `from_http_error`). Mitigation: one test per kind (`HttpException`, a built-in unit exception,
  `HttpError` type, plain `std::error::Error`, `anyhow::Error`) asserting status and body.
- **Two `.http()` traits.** `ResultHttpExt` and `OrmResultExt` both define `http()`; they apply to disjoint types
  while `OrmError` does not implement `HttpError`, so importing both is not ambiguous. If decision 4 is revisited,
  `OrmResultExt` must be deprecated at the same time.
- **`serde(flatten)` costs.** Flattened deserialization buffers the map and is slower; it only applies to queries
  that ask for counts. Field types that rely on `deserialize_any` quirks (integers inside flatten) need tests:
  `i64`, `Decimal`, `DateTime` and `Option` relations.
- **Leaking messages.** `HttpError`'s default body sends `Display`; a 5xx `AppError` that embeds internal details
  would expose them. Documented, and `body()` can be overridden.
- **Example output drift.** Changing the `blog-orm` timestamp type changes the `createdAt` format of endpoints that
  were already typed (`find_one`, `create`) to the JavaScript-compatible one; this is intended, and noted in the
  pull request.

## Open questions

- Should `OrmError` implement `HttpError` (deprecating `OrmResultExt`)? It would remove the need for the filter in
  simple apps, but changes what `Result<T, OrmError>` handlers answer without it. A maintainer should decide, ideally
  after checking whether the port apps return ORM errors directly.
- Names: `HttpError`, `ResultHttpExt`, `Data`, `WithCount`, `<model>::Count`. `Data` is short and may collide with
  user types in glob imports; alternative `OrmData`. This change uses `Data`, exported from `corvis_orm` but not from
  `corvis_orm::prelude`.
- Should `HttpError` also apply to `#[handlers]` (WebSocket messages, events)? Left out of scope.
- Is `HttpResult<Option<T>>` answering a JSON `null` (instead of an empty body) intended? Found while reading the
  conversion; left to a new issue.

## Review notes (2026-11-08)

- `OrmError` does not implement `HttpError` in this change and `OrmResultExt` stays: changing the response of handlers
  that return `Result<T, OrmError>` would need a parity review of the port apps first.
- The write wrapper is named `Data` and lives at `corvis_orm::Data`; it is not re-exported from any prelude, so glob
  imports cannot clash with user types.
- `HttpError` for WebSocket and event handlers is out of scope.
- The `HttpResult<Option<T>>` (`null`) vs `Option<T>` (empty body) inconsistency is filed as a separate issue.

## Security review

- `HttpError` 5xx without a custom `body()` answers the generic internal-error body, is marked internal (so it is
  logged) and never sends its `Display` text; a custom `body()` is sent as written. To tell "custom" from "default",
  `body()` returns `Option<ExceptionBody>` (default `None`) instead of an `ExceptionBody` defaulting to `Display`.
  A status below 400 is treated as 500.
- `update_many_and_return` without a `where` updates and loads the whole table, as in the reference client. Parity is
  kept; the method docs and the ORM docs carry a warning. `From<Data<T>>` stays available for every delegate action:
  `Args` is not typed by action, so restricting it to the create actions would need a separate argument type for
  `create`/`create_many`. Passing `Data` where a `where` is required (`update`, `delete`) fails the engine validation; `update_many(Data(..))` is the same unfiltered write as `update_many(json!({ "data": .. }))`.
