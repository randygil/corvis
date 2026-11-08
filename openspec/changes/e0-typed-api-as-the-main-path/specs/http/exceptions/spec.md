# Spec Delta

## ADDED Requirements

### Requirement: Domain errors declare their HTTP status

`corvis` SHALL provide a public trait `HttpError: std::error::Error + Send + Sync + 'static`, exported from the
prelude, with a required `fn status(&self) -> StatusCode` and a provided `fn body(&self) -> ExceptionBody` whose
default is the error's `Display` text. `HttpException::from_http_error(err)` SHALL build an exception with that
status and a body in the format of the built-in exceptions (`{ "message", "error", "statusCode" }`, where `error` is
the status reason phrase), keep `err` as the cause, and report `is_internal() == false`.

#### Scenario: Default body
- **WHEN** `AppError::CatNotFound(42)` implements `HttpError` with status 404 and `Display` "Cat #42 not found", and is
  converted with `HttpException::from_http_error`
- **THEN** the exception has status 404 and the body `{"message":"Cat #42 not found","error":"Not Found","statusCode":404}`

#### Scenario: Custom body
- **WHEN** an error's `body()` returns `ExceptionBody::Object(json!({ "code": "CAT_GONE", "statusCode": 410 }))` and its
  status is 410
- **THEN** the response body is exactly `{"code":"CAT_GONE","statusCode":410}`

#### Scenario: Filters can still see the typed error
- **WHEN** an exception built from an `AppError` reaches a custom `ExceptionFilter`
- **THEN** `exception.downcast_cause::<AppError>()` returns the original error

### Requirement: Handlers can return typed errors directly

A route handler declared as returning `Result<T, E>` with `E: HttpError` SHALL answer with the exception built by
`HttpException::from_http_error` when it returns `Err`. Handlers returning `HttpResult<T>`, a built-in exception type,
another `std::error::Error`, or `anyhow::Error` MUST keep their current conversions.

#### Scenario: Typed error returned from a handler
- **WHEN** `#[get(":id")] async fn find(&self, ...) -> Result<CatDto, AppError>` returns `Err(AppError::CatNotFound(42))`
- **THEN** the client receives 404 with the body of the default-body scenario

#### Scenario: Other errors are unchanged
- **WHEN** a handler returning `Result<T, std::io::Error>` returns an error
- **THEN** the client receives `{"statusCode":500,"message":"Internal server error"}` as before

### Requirement: Typed errors convert with `?` through an extension

`corvis` SHALL provide an extension trait `ResultHttpExt<T>` with `fn http(self) -> HttpResult<T>` implemented for
`Result<T, E>` where `E: HttpError`, exported from the prelude, so `service.call().http()?` works inside a handler
returning `HttpResult<T>`. The blanket `From<E: std::error::Error> for HttpException` MUST stay as it is.

#### Scenario: `?` inside an `HttpResult` handler
- **WHEN** a handler returning `HttpResult<CatDto>` calls `self.cats.find(42).http()?` and the service returns
  `Err(AppError::CatNotFound(42))`
- **THEN** the client receives 404, not 500

#### Scenario: Plain `?` keeps today's behavior
- **WHEN** the same handler uses `self.cats.find(42)?` without `.http()`
- **THEN** the client receives 500 with the error as the exception's cause, as documented today
