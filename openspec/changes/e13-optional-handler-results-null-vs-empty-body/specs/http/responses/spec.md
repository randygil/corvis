# Spec Delta

## Purpose

How an HTTP route handler's result that carries no value is answered, so that every return type agrees with the
original framework's Express adapter (`null` and `undefined` answer `res.send()` with no body).

## ADDED Requirements

### Requirement: A handler result with no value answers an empty body
When a route handler's result carries no value (`None` from any `Option<T>`, with or without `HttpResult<_>` /
`Result<_, E>`; `Value::Null`; or any `T: Serialize` whose JSON is `null`), the framework SHALL answer with an empty
body and the status the handler would get with a body (`201` for `POST`, `200` otherwise, or `#[http_code]`).

#### Scenario: Optional DTO with no value
- **WHEN** a `#[get]` handler returns `HttpResult<Option<CatDto>>` with `Ok(None)`, where `CatDto` only derives
  `Serialize`
- **THEN** the response is 200 with an empty body, `Content-Length: 0`, and no `Content-Type` or `ETag` header

#### Scenario: Both optional return types agree
- **WHEN** one handler returns `Option<CatDto>` with `None` and another returns `Option<Value>` with `None`
- **THEN** both answers have the same status, the same headers and an empty body

#### Scenario: JSON null value
- **WHEN** a handler returns `HttpResult<Value>` with `Ok(Value::Null)`
- **THEN** the response is 200 with an empty body and no `Content-Type` header

#### Scenario: Default status of a POST
- **WHEN** a `#[post]` handler returns `Option<CatDto>` with `None`
- **THEN** the response is 201 with an empty body and `Content-Length: 0`

### Requirement: An empty answer carries the original framework's headers
An empty answer MUST NOT carry a `Content-Type` unless the handler or an interceptor set one, MUST NOT carry an
`ETag`, and MUST carry `Content-Length: 0` except for `HEAD`, `204` and `304`, as the original framework does.

#### Scenario: Fixed status
- **WHEN** a handler with `#[http_code(204)]` returns `Option<CatDto>` with `None`
- **THEN** the response is 204 with no `Content-Type`, `Content-Length` or `ETag` header

#### Scenario: Content type set by the handler
- **WHEN** a handler with `#[header("Content-Type", "application/json")]` returns `Option<CatDto>` with `None`
- **THEN** the response carries `Content-Type: application/json; charset=utf-8`, `Content-Length: 0` and an empty
  body

#### Scenario: HEAD request
- **WHEN** a `HEAD` request reaches a `#[get]` handler that returns `Option<CatDto>` with `None`
- **THEN** the response is 200 with no body and no `Content-Type`, `Content-Length` or `ETag` header

### Requirement: A handler result with a value is answered as that value
`Some(v)` SHALL be answered exactly as `v` would be answered on its own: JSON for a `T: Serialize`, text for a
`String`, and the value's own conversion for any other `T: IntoResponse`.

#### Scenario: Optional DTO with a value
- **WHEN** a handler returns `Option<CatDto>` with `Some(CatDto { name: "Tom" })`
- **THEN** the response is 200 with the body `{"name":"Tom"}`, `Content-Type: application/json; charset=utf-8` and
  a weak `ETag`

#### Scenario: Optional text with a value
- **WHEN** a handler returns `HttpResult<Option<String>>` with `Ok(Some("hi".into()))`
- **THEN** the response is 200 with the body `hi` and `Content-Type: text/html; charset=utf-8`

### Requirement: Explicit JSON wrappers keep a literal null
`Json(v)` and `Response::json(v)` SHALL serialize `v` as JSON even when it is `null`, so a handler can answer the
text `null` with `Content-Type: application/json; charset=utf-8` on purpose.

#### Scenario: Literal null on purpose
- **WHEN** a handler returns `Json(None::<CatDto>)`
- **THEN** the response is 200 with the body `null` and `Content-Type: application/json; charset=utf-8`

### Requirement: Interceptors that map the data to null produce an empty body
`Response::map_data` SHALL store a `null` result of its closure as an empty body, and SHALL keep giving an empty body
to the closure as `null`, so an interceptor that returns the data unchanged keeps an empty answer empty.

#### Scenario: Identity interceptor on an empty answer
- **WHEN** an interceptor calls `map_data(|data| data)` on the answer of a handler that returns `Option<CatDto>` with
  `None`
- **THEN** the response has an empty body and no `Content-Type` header

#### Scenario: Envelope interceptor on an empty answer
- **WHEN** an interceptor calls `map_data(|data| json!({ "data": data }))` on the answer of a handler that returns
  `Option<CatDto>` with `None`
- **THEN** the response body is `{"data":null}`

### Requirement: Response serializers and the cache treat an empty answer as null
`ZodSerializerInterceptor` SHALL validate an empty handler result as `null` when the handler declares a serializer
DTO, as the original framework validates every result. `CacheInterceptor` SHALL answer a cached `null` with an empty
body.

#### Scenario: Serializer on an optional result
- **WHEN** a handler with a serializer DTO whose schema does not accept `null` returns `Option<CatDto>` with `None`
- **THEN** the serializer reports the validation failure as it does today for that return type

#### Scenario: Serializer with a nullable schema
- **WHEN** the serializer DTO's schema accepts `null` and the handler returns `Option<CatDto>` with `None`
- **THEN** the response has an empty body

### Requirement: Empty answers match the original framework
A differential test SHALL replay the answers recorded from the original framework with the Express adapter for
handlers that return `null`, `undefined`, nothing, an async `null`, a `POST` `null`, a `HEAD` on `null`, `null` with a
fixed `204`, `null` with an explicit `Content-Type`, and non-empty controls, and the Corvis equivalents MUST give the
same status, `Content-Type`, `Content-Length`, `ETag`, `X-Powered-By` and body.

#### Scenario: Recorded answers replayed
- **WHEN** the fixture's Corvis app answers each recorded request on `127.0.0.1`
- **THEN** every compared field equals the recorded one, or the difference is listed in the test as a known
  difference with its reason
</content>
</invoke>
