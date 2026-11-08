# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state, verified in the code and on the
wire on 2026-11-08 (scratch probe against the current `main`, `app.set_x_powered_by("Express")`, raw HTTP/1.1 on
`127.0.0.1`).

**How a handler result becomes a response**

- `#[routes]` (`crates/corvis-macros/src/controller.rs:408-421`) unwraps a return type whose last segment ends in
  `Result` and converts the `Ok` value with autoref dispatch: `(&v).__corvis_reply_kind().convert(v)`.
- `ReplyKind` (`crates/corvis/src/__private.rs:289-295`) is picked when the value implements `IntoResponse` and calls
  `into_response()` (`ReplyTag`, `__private.rs:277-282`). Otherwise `SerializeKind` (`__private.rs:296-301`) is picked
  for any `T: Serialize` and builds `Response::json(serde_json::to_value(&v))` (`SerializeTag`, `__private.rs:283-288`).
- `IntoResponse for Option<T>` exists only for `T: IntoResponse` (`crates/corvis/src/http/response.rs:371-378`):
  `Some(v)` → `v.into_response()`, `None` → `Response::new()` (`Body::Empty`). `Result<T, E>` forwards to `T`
  (`response.rs:363-370`).
- So the split is decided by `T`, not by `HttpResult`: `Option<CatDto>` and `HttpResult<Option<CatDto>>` are not
  `IntoResponse` (the DTO is only `Serialize`) and go through `SerializeTag`, which turns `None` into `Value::Null`;
  `Option<Value>` and `HttpResult<Option<String>>` go through `Option::into_response` and answer an empty body.
- `IntoResponse for Value` (`response.rs:343-347`) wraps any value, `Null` included, in `Body::Json`.
  `Json<T>` (`response.rs:396-401`) does the same explicitly. `()` answers `Response::new()` (`response.rs:328-332`).
- `Response::map_data` (`response.rs:189-204`) gives an empty body to the closure as `null` and always stores the
  result as `Body::Json`, so an identity `map_data` turns an empty answer into `null`.

**How the body is written**

- `http::express::finalize` (`crates/corvis/src/http/express.rs:233-295`) gives `Body::Json` / `Body::Text` /
  `Body::Html` the `res.send()` treatment: default `Content-Type` (`application/json; charset=utf-8` for JSON), weak
  `ETag` (line 273) and `Content-Length`. `Body::Empty` returns early (line 242-249): no type, no `ETag`.
- `server::into_hyper` (`crates/corvis/src/server.rs:120-160`) writes `Body::Empty` with no `Content-Type` (line
  130) and hyper adds `Content-Length: 0`. For `HEAD` it inserts the exact length of the body it would have sent
  (line 149-154), which is `0` for `Body::Empty`.

**Current answers (probe, `GET` unless stated)**

| Handler return | Status | Content-Type | Content-Length | ETag | Body |
|---|---|---|---|---|---|
| `Option<CatDto>` → `None` | 200 | `application/json; charset=utf-8` | 4 | `W/"4-K+iMpCQsduglOsYkdIUQZQMtaDM"` | `null` |
| `HttpResult<Option<CatDto>>` → `Ok(None)` | 200 | same | 4 | same | `null` |
| `Option<CatDto>` → `Some(..)` | 200 | `application/json; charset=utf-8` | 14 | yes | `{"name":"Tom"}` |
| `Option<Value>` → `None` | 200 | none | 0 | none | empty |
| `HttpResult<Option<String>>` → `Ok(None)` | 200 | none | 0 | none | empty |
| `Option<Value>` → `Some(json!(..))` | 200 | `application/json; charset=utf-8` | 14 | yes | `{"name":"Tom"}` |
| `HttpResult<Option<String>>` → `Ok(Some("hi"))` | 200 | `text/html; charset=utf-8` | 2 | yes | `hi` |
| `POST`, `Option<Value>` → `None` | 201 | none | 0 | none | empty |
| `POST`, `Option<CatDto>` → `None` | 201 | `application/json; charset=utf-8` | 4 | yes | `null` |
| `HEAD`, `Option<Value>` → `None` | 200 | none | 0 | none | empty |
| `HEAD`, `Option<CatDto>` → `None` | 200 | `application/json; charset=utf-8` | 4 | yes | empty |
| `HttpResult<Value>` → `Ok(Value::Null)` | 200 | `application/json; charset=utf-8` | 4 | yes | `null` |
| `()` | 200 | none | 0 | none | empty |
| `Json<Option<CatDto>>` → `Json(None)` | 200 | `application/json; charset=utf-8` | 4 | yes | `null` |
| `#[http_code(204)]`, `Option<CatDto>` → `None` | 204 | none | none | `W/"4-K+iMpCQsduglOsYkdIUQZQMtaDM"` | empty |

The last row also shows a stray `ETag` on a `204`, computed from the `null` body before `finalize` empties it.

**Original framework (source read in the originals' `node_modules`; answers recorded on 2026-11-08)**

- `@nestjs/core` `router/router-execution-context.js:189-193` (11.2.1, facten-api): the handler result is awaited
  (`transformToResult`, which also unwraps observables) and passed to `RouterResponseController.apply` unless the
  handler uses `@Res()` without passthrough.
- `router/router-response-controller.js:15-17`: `apply` calls `applicationRef.reply(response, result, statusCode)`
  with the default status (`getStatusByMethod`, lines 38-45: `201` for `POST`, `200` otherwise) or `@HttpCode`.
- `@nestjs/platform-express` `adapters/express-adapter.js:44-50` (identical in 11.1.28 for consi, 11.2.1 for facten
  and 12.0.4 for houseplan): `if (isNil(body)) return response.send();`. `isNil` is
  `@nestjs/common/utils/shared.utils.js:46`: `undefined` or `null`. Objects go to `response.json(body)`, everything
  else to `response.send(String(body))` (line 68).
- `express/lib/response.js:125-196` (express 5.2.1): `send()` with `undefined` sets no `Content-Type`, computes no
  length and no `ETag` (lines 172-196 only run for a defined chunk); Node then writes `Content-Length: 0` for a `GET`
  and nothing for a `HEAD`. A `null` passed to `send` directly would become `''` (line 144-145), but the adapter never
  passes it.

Recorded with a scratch app on `127.0.0.1` (plain JS controller, `NestFactory.create`, express adapter, Node 24.13.0)
against the `node_modules` of all three originals (core 11.1.28, 11.2.1 and 12.0.4, express 5.2.1); the three
recordings are identical. `x-powered-by: Express` is present in every row.

| Original handler | Status | Content-Type | Content-Length | ETag | Body |
|---|---|---|---|---|---|
| `@Get() f() { return null; }` | 200 | none | 0 | none | empty |
| `return undefined;` | 200 | none | 0 | none | empty |
| `f() {}` (no return) | 200 | none | 0 | none | empty |
| `async f() { return null; }` | 200 | none | 0 | none | empty |
| `@Post()`, `return null;` | 201 | none | 0 | none | empty |
| `HEAD`, `return null;` | 200 | none | none | none | empty |
| `@HttpCode(204)`, `return null;` | 204 | none | none | none | empty |
| `@Header('Content-Type', 'application/json')`, `return null;` | 200 | `application/json; charset=utf-8` | 0 | none | empty |
| `return { a: 1 };` | 200 | `application/json; charset=utf-8` | 7 | `W/"7-n4nHQM60bXQYySSnisV5QdXpZSA"` | `{"a":1}` |
| `return 'hi';` | 200 | `text/html; charset=utf-8` | 2 | `W/"2-witfkXg0JglCjW9RssWvTAveakI"` | `hi` |
| `return '';` | 200 | `text/html; charset=utf-8` | 0 | `W/"0-2jmj7l5rSw0yVb/vlWAYkK/YBwk"` | empty |
| `return [null];` | 200 | `application/json; charset=utf-8` | 6 | `W/"6-1E2iJRDeml63J1thpL7r1NDNa10"` | `[null]` |

The scratch script is the starting point for the committed fixture (task 1.1); it records the same rows.

**Neighbours that read the body**

- `ZodSerializerInterceptor` (`crates/corvis/src/z/pipe.rs:359-368`) runs the schema only when `json_body()` is
  `Some`: today it validates `None` from the serialize path (`Body::Json(Null)`) and skips `None` from the
  `IntoResponse` path and `()`. The original (`nestjs-zod` 5.5.0, `dist/index.cjs:148-168`) validates every result,
  `null` and `undefined` included.
- `CacheInterceptor` (`crates/corvis/src/modules/cache.rs:175-203`) caches `Body::Json` and text bodies and replays a
  cached value as `Body::Json(v)` (line 188), so a cached `null` would replay as the text `null`.
- WebSocket handlers convert results with their own `__corvis_ws_kind` (`crates/corvis-macros/src/handlers.rs:297-306`)
  and treat `null` as "no reply" (`crates/corvis/src/websockets.rs:731-739`); this change does not touch them.

**Port apps**

- consi (`corvis-tests/corvis/consi-backend`): 22 handlers return `HttpResult<Value>`; none returns `Option`. A
  global `TransformInterceptor` (`src/main.rs:66`, `src/common/transform.interceptor.rs`) wraps every result in
  `{ success, data, error }` with `map_data`, so a `null` result is never the whole body. No change expected.
- facten (`corvis-tests/corvis/facten-api`): handlers return typed DTOs, `Vec<Dto>`, `Value` or `Response`; none
  returns `Option` or `Value::Null`. `reports.controller.rs:33,55` builds `Response::json(..)` explicitly, which keeps
  its literal JSON. No change expected.
- houseplan (`corvis-tests/corvis/houseplan-api`): `POST transactions/:id/undo` returns `Value::Null` when there is
  nothing to undo (`src/http/api.controller.rs:331-335`), and `entry_json` returns `Value::Null` for a missing entry
  (`api.controller.rs:272-277`, used by three handlers). The controller is wrapped in `PlainJson`
  (`api.controller.rs:53-64`, applied at line 249), which turns a JSON `null` into `Body::Empty` with the comment
  "`null`/`undefined` → `res.send()`: empty body": the workaround for this issue. apidiff covers the case
  (`apidiff/01-ledger.mjs:81`, "nothing left: null") and passes today because of `PlainJson`. After the change
  `map_data(plain)` on an empty body gives `null` to `plain`, which returns `null`, which `map_data` stores as
  `Body::Empty`: the same answer. The workaround becomes redundant and can be removed in the port later.
- Parity risk: low. The only port that returns `null` from a handler already answers an empty body, the other two
  never return one. The parity task still runs all three suites.

## Goals / Non-Goals

**Goals:**
- One rule for "no value" in HTTP handlers, the original framework's: empty body, default status, no type, no
  `ETag`, whatever the return type.
- A recorded fixture that pins the original's answers and a replay test that compares them byte for byte on the
  wire.
- An explicit way to send a literal `null` body.

**Non-Goals:**
- Distinguishing `null` from `undefined` (proposal, Out of scope).
- WebSocket, microservice and event handler results.
- The OpenAPI description of optional responses.

## Decisions

1. **Normalize at conversion time, not when writing.** `SerializeTag::convert` returns `Ok(Response::new())` when
   the serialized value is `Value::Null`, and `IntoResponse for Value` does the same for `Value::Null`. `Option<T:
   IntoResponse>` already answers `Response::new()` for `None` and keeps it. `Body::Empty` then means "the handler had
   no value", the meaning `map_data` already gives it (`null` in, see Decision 3).
   Alternative considered: turn every `Body::Json(Null)` into an empty body in `finalize`. Rejected: it would also
   rewrite `Response::json(Value::Null)` and `Json(None)`, removing the only way to send a literal `null`, and
   responses built by filters.
2. **`Json<T>` and `Response::json` stay literal.** They are the explicit "serialize this as JSON" wrappers (docs:
   "Explicit JSON or HTML"); `Json(None::<CatDto>)` keeps answering `null` with a JSON type. This is the documented
   escape hatch for APIs whose clients expect `null`. In the original the same needs `@Res()` and `res.json(null)`.
   Alternative considered: make `Json(None)` empty too, for strict uniformity. Rejected: leaves no return-value way to
   send `null`. See Open questions.
3. **`map_data` stores `null` as an empty body.** After `f` runs, a `Value::Null` result sets `Body::Empty` instead of
   `Body::Json(Null)`. This makes an identity interceptor (`map_data(|d| d)`) keep an empty answer empty, and an
   interceptor that maps to `null` answer like the original's `map(() => null)`. Envelope interceptors (consi's) are
   unaffected: their closure returns an object.
4. **`ZodSerializerInterceptor` validates an empty result as `null`.** When the serializer metadata is set and the
   body is `Body::Empty`, the schema runs on `Value::Null`; if it passes, the output goes through `map_data` (so a
   `null` output stays empty). This keeps today's behavior for `Option<Dto>` returns (validated as `null`) and matches
   the original, which validates every result. It newly validates `()` handlers that declare a serializer DTO; in the
   original those are validated too (as `undefined`).
5. **`CacheInterceptor` replays `null` as empty.** A cache hit whose value is `Value::Null` answers `Response::new()`
   with status 200. Empty answers are still not stored (as today for `Body::Empty`). Entries written as `null` by an
   older version (a persistent store) then replay like a fresh answer.
6. **`HEAD` on an empty answer sends no `Content-Length`.** In `into_hyper`, the `HEAD` length is only inserted when
   the body is not `Body::Empty`. A text body that is the empty string still gets `Content-Length: 0` from `finalize`,
   as in Express. If hyper adds `Content-Length: 0` on its own for a `HEAD`, the replay test records it as a known
   difference with a comment and the docs say so (task 3.3).
7. **Differential fixture like `crates/corvis/tests/fixtures/terminus/`.** A `package.json` pinning
   `@nestjs/common`, `@nestjs/core`, `@nestjs/platform-express` (11.2.1, the version facten uses), `reflect-metadata`
   and `rxjs`, and a `generate.mjs` that starts a plain JS controller with the rows of the table above on
   `127.0.0.1:0` (ephemeral port), records status, headers (without `date`, `connection`, `keep-alive`) and body as
   text with a raw `http.request`, closes the app, and writes `empty_results.json`. The Rust test
   `crates/corvis/tests/empty_results.rs` declares the Corvis equivalents (`Option<Dto>` → `None` and
   `HttpResult<Option<Dto>>`, `Option<Value>`, `HttpResult<Option<String>>`, `Value::Null`, `()`, `async`, `POST`,
   `HEAD`, `#[http_code(204)]`, `#[header("Content-Type", "application/json")]`, and the non-empty controls), listens
   on `127.0.0.1:0` with `set_x_powered_by("Express")`, sends raw HTTP/1.1 requests and compares status, the
   `content-type`, `content-length`, `etag` and `x-powered-by` headers and the body with the recorded row.

## Risks / Trade-offs

- [Clients of a Corvis app that parse `null` as JSON get an empty body] → that is what the original framework
  answers, so migrated clients already handle it; documented as a behavior change in the docs and the change summary,
  with `Json(None::<T>)` as the way back.
- [`ZodSerializerInterceptor` starts validating `()` handlers that declare a serializer DTO] → matches the original;
  a contradictory setup (a DTO on a handler with no result) that the docs mention.
- [Interceptors that relied on `map_data` producing `Body::Json(Null)` (for example reading `json_body()` and checking
  `is_null()`) now see `json_body() == None`] → houseplan's `PlainJson` does that check and keeps working (the body is
  already empty); the `interceptors.md` docs state the rule.
- [#9 adds a requirement that any `T: Serialize` answers its JSON] → a `T` whose JSON is `null` now answers an empty
  body. The spec delta here states the exception; whichever change is archived second reconciles the wording of the
  `http/responses` spec (task 6.6).

## Open questions

1. **`Json(None)`.** This change keeps `Json(v)` and `Response::json(v)` literal, so they answer `null`. Confirm, or
   decide that `Json(None)` also answers empty and only `Response::json(Value::Null)` sends `null`.
2. **`HEAD` length.** Decision 6 fixes the `HEAD` `Content-Length` of every empty answer (`()` included), which is a
   small widening of the issue. Confirm it belongs here, or split it into its own issue.
3. **houseplan `PlainJson`.** It becomes redundant. Remove it from the port after this lands (a port change, not part
   of this change), or keep it as is.

## Review notes (2026-11-08)

1. Accept the recommended decision: every result without a value (`None` of any `Option`, `Value::Null`, a
   `Serialize` whose JSON is `null`) answers an empty body, like the original; decisions 1 to 7 stand.
2. Open question 1: `Json(v)` and `Response::json(v)` stay literal, including `Json(None)`. They are the explicit way
   to send a JSON `null`, and the docs say so.
3. Open question 2: the `HEAD` `Content-Length` fix (decision 6) and the stray `ETag` on a `204` belong in this change;
   both are part of "what an empty answer looks like" and the fixture covers them.
4. Open question 3: houseplan's `PlainJson` stays as it is. Removing it is an optional port follow-up, not part of
   this change; the parity run must show it still answers an empty body.
5. Order: start after #9 (`e0-typed-api-as-the-main-path`) is merged, on top of it, since both touch the route
   macro's result conversion. #9 is archived first, so task 6.2 applies: reconcile its `http/responses` requirement.
6. The change name differs from the one suggested in the issue; the issue is updated to point at this one.

## Reconciliation with #9 (task 6.2, 2026-11-08)

#9's delta (`e0-typed-api-as-the-main-path/specs/http/responses/spec.md`, not archived yet) says "Typed handler return
values are serialized as JSON": any `T: Serialize` answers its JSON with `Content-Type: application/json`, and a type
that implements `IntoResponse` keeps its own conversion. Its scenarios (a struct, same bytes as `json!`) never use a
value whose JSON is `null`, so they stay true. The two combine as follows: #9's requirement is the general rule and the
first requirement of this change is its single exception (a `T` whose JSON is `null`, `None` of an `Option<T>`, and
`Value::Null` answer an empty body). Since #9's requirement does not exist in the main spec yet, this delta cannot use
`MODIFIED`; it states the exception in its own wording (naming #9's requirement) and stays `ADDED`. #9's change folder
was not edited.

## Implementation notes (2026-11-08)

- The recording of the original (`fixtures/empty_results/empty_results.json`, core 11.2.1, express 5.2.1, Node 24.13.0)
  matches the "Original framework" table above row by row.
- The stray `ETag` on a `204` disappears with the normalization: an empty answer never reaches the `ETag` step of
  `finalize`, which is unchanged. `Json(None)` on a `204` would still compute one from the discarded body, as Express
  does for `res.json(null)`.
- The in-memory test client does not show `Content-Length` (hyper writes it on the wire), so `Content-Length: 0` is
  asserted in the socket replay only.
