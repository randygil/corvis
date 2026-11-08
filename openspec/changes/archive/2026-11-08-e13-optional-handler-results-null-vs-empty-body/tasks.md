# Tasks

## 1. Reference behavior and failing tests

- [x] 1.1 Add `crates/corvis/tests/fixtures/empty_results/` with a `package.json` pinning `@nestjs/common`,
  `@nestjs/core` and `@nestjs/platform-express` 11.2.1, `reflect-metadata` 0.2.2 and `rxjs` 7.8.2 (as
  `fixtures/terminus/package.json` does), and a `generate.mjs` that starts a controller with the rows of design.md
  ("Original framework" table) on `127.0.0.1:0`, records status, headers (without `date`, `connection`,
  `keep-alive`) and body with `http.request`, closes the app and writes `empty_results.json`; add
  `crates/corvis/tests/fixtures/empty_results/node_modules` to `.gitignore`. The scratch script used for design.md
  (`record.cjs`, plain JS decorators) can be adapted. If the recording differs from the design.md table, stop and
  record it under Open questions in design.md. Verify by running `npm install && node generate.mjs` once (no server
  left running afterwards) and committing the JSON, not `node_modules`
- [x] 1.2 Add `crates/corvis/tests/empty_results.rs` with one controller per return type (`Option<CatDto>` and
  `HttpResult<Option<CatDto>>` with `None` and `Some`, `Option<Value>` and `HttpResult<Option<String>>` with `None`
  and `Some`, `HttpResult<Value>` with `Value::Null`, `()`, an `async` handler, `#[post]`, `#[http_code(204)]`,
  `#[header("Content-Type", "application/json")]`, `Json(None::<CatDto>)`, and the non-empty controls of the fixture),
  listening on `127.0.0.1:0` with `set_x_powered_by("Express")`; one test replays `empty_results.json` with raw
  HTTP/1.1 requests and compares status, `content-type`, `content-length`, `etag`, `x-powered-by` and body; other
  tests cover the spec scenarios that the fixture does not (`Json(None)` literal, identity and envelope `map_data`
  interceptors, serializer with a strict and a nullable schema, cache hit on a stored `null`). Verify with
  `cargo test -p corvis --test empty_results` that they fail on the current code for the expected reason (`null`
  bodies, JSON type, `ETag`, `HEAD` length)

## 2. Conversion of handler results

- [x] 2.1 In `crates/corvis/src/__private.rs`, make `SerializeTag::convert` return `Ok(Response::new())` when the
  serialized value is `Value::Null`; in `crates/corvis/src/http/response.rs`, make `IntoResponse for Value` do the same
  for `Value::Null`, leave `IntoResponse for Option<T>` as is (it already answers `Response::new()`), and leave
  `Json<T>` and `Response::json` literal; update the doc comments of `IntoResponse`, `Json` and `Response::json` to say
  so. Verify the `Option`, `Value::Null`, `POST` and `204` rows of the replay pass and the `Json(None)` test still
  answers `null`
- [x] 2.2 In `Response::map_data` (`crates/corvis/src/http/response.rs`), store a `Value::Null` result of the closure
  as `Body::Empty`; update its doc comment. Verify the identity and envelope interceptor tests, and that
  `cargo test -p corvis --lib` and `cargo test -p corvis --test app` still pass

## 3. Neighbours and wire details

- [x] 3.1 In `ZodSerializerInterceptor` (`crates/corvis/src/z/pipe.rs`), when the serializer metadata is set and the
  body is `Body::Empty`, validate `Value::Null`; verify the strict and nullable serializer tests and
  `cargo test -p corvis --test zod_pipe`
- [x] 3.2 In `CacheInterceptor` (`crates/corvis/src/modules/cache.rs`), answer a cache hit whose value is
  `Value::Null` with `Response::new()` and status 200; verify the cache test and the existing cache tests
  (`cargo test -p corvis --test modules`)
- [x] 3.3 In `server::into_hyper` (`crates/corvis/src/server.rs`), skip the `HEAD` `Content-Length` when the body is
  `Body::Empty`; verify the `HEAD` row of the replay and that `crates/corvis/tests/express_parity.rs` still passes
  (its `HEAD` with a JSON body keeps `Content-Length: 16`). If hyper itself writes `Content-Length: 0`, keep the
  difference as a known difference in the replay test with a comment and note it in the docs bullet of 4.1
- [x] 3.4 Check the other callers that read `json_body()` or `Body::Json` (`crates/corvis/src/router/mod.rs`
  redirect handling, `crates/corvis/src/websockets.rs`, `crates/corvis-swagger`) still behave the same for empty
  bodies; verify with `cargo test -p corvis --test websockets` and `cargo test -p corvis-swagger`

## 4. Docs

- [x] 4.1 `docs/controllers.md`, section "Responses": replace the `Option<T>, Result<T, E>` "Depends on the content"
  row with the rule (`None`, `Value::Null` and any value whose JSON is `null` answer an empty body with the default
  status, no `Content-Type`, `Content-Length: 0` and no `ETag`, like returning `null` or `undefined` in the original
  framework; `Some(v)` answers `v`; `Result` forwards `Ok` and maps `Err`), add `Json(None::<T>)` /
  `Response::json(Value::Null)` as the way to send a literal `null`, and adjust the `HEAD` bullet; same in
  `docs/es/controllers.md`
- [x] 4.2 `docs/interceptors.md` (`map_data` paragraph): a `null` result of the closure is stored as an empty body;
  `docs/application/serialization.md` if it describes the serializer interceptor on empty results; same in `docs/es/`
- [x] 4.3 `docs/migration-guide.md`: one bullet on handlers that return `null` / `undefined` (now an empty body in
  Corvis too, `Json(None)` for a literal `null`), marked as a behavior change for apps on earlier Corvis versions;
  same in `docs/es/migration-guide.md`
- [x] 4.4 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [x] 5.1 `cargo fmt --all -- --check` passes
- [x] 5.2 `cargo build --workspace --all-targets` with no warnings
- [x] 5.3 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.4 `cargo test --workspace` passes, including `crates/corvis/tests/empty_results.rs` and the differential
  replay of 1.2
- [x] 5.5 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 5.6 Naming check: no "Nest", "NestJS" or "Prisma" in new public API names, doc comments, docs pages or error
  messages (`git diff main -- crates docs | grep -inE "nest|prisma"` shows only fixture package names and paths)

## 6. Parity (not run by the implementer: the maintainer runs it)

- [x] 6.1 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff (not run by the implementer: the maintainer runs it)
  differences; pay attention to houseplan `01-ledger.mjs` (`POST /api/transactions/3/undo`, "nothing left: null"),
  which must still answer an empty body through `PlainJson`, and to consi's envelope (`"data": null`) (maintainer batch run on batch/phase04-g d6f3ea0: consi 832/832, facten 727/727, houseplan 636/636; routes identical, databases equivalent)
- [x] 6.2 If #9 (`e0-typed-api-as-the-main-path`) is archived before this change, reconcile its `http/responses` (reconciled in the delta and design.md; #9 folder untouched)
  requirement "Typed handler return values are serialized as JSON" with the empty-body rule (a `T` whose JSON is
  `null` answers an empty body); if this change is archived first, leave the note for #9

## Workflow follow-up

- Archive the change with `/opsx:archive` after review and the parity run.
- Close issue #122, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).
- Optional port follow-up (not part of this change): remove houseplan's `PlainJson` null workaround once this lands.
