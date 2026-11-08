# Spec Delta

## ADDED Requirements

### Requirement: Typed handler return values are serialized as JSON

A route handler SHALL be able to return any `T: Serialize`, or `HttpResult<T>` / `Result<T, E>` with
`T: Serialize`, and the framework MUST answer with `T` serialized as JSON (`Content-Type: application/json;
charset=utf-8`), keeping the field order of `T`'s `Serialize` implementation. A type that implements `IntoResponse`
MUST keep its own conversion instead (for example `String` answers text, `()` answers an empty body).

#### Scenario: A struct is returned
- **WHEN** a `#[get]` handler returns `HttpResult<CatDto>` with `CatDto { id: 1, name: "Tom" }` deriving `Serialize`
- **THEN** the response is 200 with the body `{"id":1,"name":"Tom"}` and a JSON content type

#### Scenario: Same bytes as the JSON version
- **WHEN** one handler returns a struct and another returns `json!` with the same keys in the same order
- **THEN** both response bodies are byte-identical

### Requirement: A status can accompany a typed body

A handler SHALL be able to choose the status of a typed response at run time by returning `(StatusCode, Json<T>)` or
`(u16, Json<T>)`, and `#[http_code(...)]` SHALL set a fixed status for a handler returning `T: Serialize`.

#### Scenario: Run-time status with a typed body
- **WHEN** a handler returns `(HttpStatus::ACCEPTED, Json(JobDto { id: 7 }))`
- **THEN** the response is 202 with the body `{"id":7}`
