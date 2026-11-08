# Proposal

Implements [randygil/corvis#122](https://github.com/randygil/corvis/issues/122) (epic [#13](https://github.com/randygil/corvis/issues/13)).
Found while drafting the typed API change for #9 (`openspec/changes/e0-typed-api-as-the-main-path`).

## Why

A handler that has no value to return answers in two different ways depending on the type inside the `Option`:
`Option<T>` with `T: IntoResponse` (for example `Option<Value>`, `HttpResult<Option<String>>`) answers `None` with an
empty body, while `Option<T>` with `T: Serialize` only (for example `HttpResult<Option<CatDto>>`, the usual result of
an ORM lookup) answers the JSON text `null` with `Content-Type: application/json`, an `ETag` and `Content-Length: 4`.
A handler returning `Value::Null` answers `null` too.

The original framework has one rule: when a controller handler returns `null` or `undefined`, the Express adapter
calls `res.send()` with no body, so the client gets the default status, no `Content-Type`, `Content-Length: 0` and no
`ETag`. The three original apps answer that way, and the houseplan port already carries an interceptor whose only job
is to turn a JSON `null` back into an empty body. Corvis should answer like the original without such workarounds.

## What Changes

- **Behavior change:** a handler result with no value answers an empty body, as in the original framework: `None`
  from any `Option<T>` (with or without `HttpResult`/`Result`), `Value::Null`, and any `T: Serialize` whose JSON is
  `null`. The status stays the default one (`200`, `201` for `POST`, or `#[http_code]`); there is no `Content-Type`
  unless the handler set one, `Content-Length` is `0` and there is no `ETag`.
- `Some(v)` keeps answering exactly what `v` answers today.
- `Json(v)` and `Response::json(v)` stay explicit: they answer the JSON text `null` when `v` is `null`, which is the
  way to send a literal `null` body on purpose.
- `Response::map_data`: an interceptor that maps the data to `null` produces an empty body, as an RxJS `map` that
  returns `null` does in the original framework.
- `ZodSerializerInterceptor` validates an empty handler result as `null` (today it validates `None` from the
  serialize path and skips the others).
- `CacheInterceptor` replays a cached `null` as an empty body.
- `HEAD` on an empty answer sends no `Content-Length`, as the original does.
- A differential fixture records the original framework's answers (null, undefined, `void`, async null, `POST`,
  `HEAD`, `@HttpCode(204)`, an explicit `Content-Type`, and non-empty controls) and a Rust test replays them.
- Docs (EN + ES): the response table in `controllers.md`, `map_data` in `interceptors.md`, and the migration guide.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `http/responses`: how handler return values with no value are answered. The capability is created by #9
  (`e0-typed-api-as-the-main-path`); this change only adds requirements to it with names of its own. If it lands
  before #9 is archived, the delta still applies as ADDED requirements.

## Impact

- `crates/corvis/src/__private.rs` (`SerializeTag::convert`), `crates/corvis/src/http/response.rs`
  (`IntoResponse for Value`, `IntoResponse for Option<T>`, `Response::map_data`), `crates/corvis/src/server.rs`
  (`into_hyper`, `HEAD` length), `crates/corvis/src/z/pipe.rs` (`ZodSerializerInterceptor`),
  `crates/corvis/src/modules/cache.rs` (cache hit replay).
- New fixture `crates/corvis/tests/fixtures/empty_results/` and replay test `crates/corvis/tests/empty_results.rs`.
- Docs: `controllers.md`, `interceptors.md`, `migration-guide.md` and their Spanish translations under `docs/es/`.
- Users: handlers returning `Option<SomeDto>` or `Value::Null` stop sending `null` with a JSON content type. Clients
  that call `response.json()` on such answers get a parse error on the empty body, exactly as with the original
  framework. The docs say how to keep a literal `null` (`Json(None::<T>)`).
- Port apps: consi wraps every result in an envelope through a global interceptor (unchanged), facten does not return
  `Option` or `null` from handlers, and houseplan's `PlainJson` interceptor already turns `null` into an empty body (it
  becomes redundant, still correct). apidiff should not move; the parity check confirms it.

## Out of scope

- Telling `null` from `undefined`. Rust has one "no value"; the cases where the original differs (a property set to
  `undefined` is dropped from JSON, `ZodSerializerDto` with `.optional()` vs `.nullable()` schemas) are not changed.
  Possible new issue if a port needs it.
- `map_data` turning a text body into a JSON string (an identity `map` over a string answers JSON in Corvis and text
  in the original). Possible new issue.
- WebSocket and microservice handlers: they already treat no value as "no reply".
- The OpenAPI document for `Option<T>` responses (#9 and the swagger crates decide that).
</content>
</invoke>
