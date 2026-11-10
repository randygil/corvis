//! `HttpException` and the built-in HTTP exceptions (`NotFoundException`, ...).
//!
//! Response bodies have this format:
//!
//! ```json
//! { "message": "Cat not found", "error": "Not Found", "statusCode": 404 }
//! ```

use std::fmt;
use std::sync::Arc;

use http::StatusCode;
use serde_json::{Map, Value, json};

/// The base HTTP exception.
///
/// Any `std::error::Error` converts into an `HttpException` (500 Internal Server Error)
/// so the `?` operator works inside handlers.
#[derive(Clone)]
pub struct HttpException {
    status: StatusCode,
    response: Value,
    message: String,
    cause: Option<Arc<dyn std::error::Error + Send + Sync>>,
    anyhow_cause: Option<Arc<anyhow::Error>>,
    /// `true` when the exception was not intentionally thrown (unknown error).
    internal: bool,
    /// `true` when whoever raised it already wrote its own (rate-limited) log line, so the default filter does not
    /// write one more for every request.
    logged: bool,
}

/// Something that can be used as the response of an exception: a message or a JSON body.
pub enum ExceptionBody {
    Empty,
    Message(String),
    Messages(Vec<String>),
    Object(Value),
}

impl From<()> for ExceptionBody {
    fn from(_: ()) -> Self {
        ExceptionBody::Empty
    }
}
impl From<&str> for ExceptionBody {
    fn from(s: &str) -> Self {
        ExceptionBody::Message(s.to_string())
    }
}
impl From<String> for ExceptionBody {
    fn from(s: String) -> Self {
        ExceptionBody::Message(s)
    }
}
impl From<&String> for ExceptionBody {
    fn from(s: &String) -> Self {
        ExceptionBody::Message(s.clone())
    }
}
impl From<Vec<String>> for ExceptionBody {
    fn from(v: Vec<String>) -> Self {
        ExceptionBody::Messages(v)
    }
}
impl From<Vec<&str>> for ExceptionBody {
    fn from(v: Vec<&str>) -> Self {
        ExceptionBody::Messages(v.into_iter().map(String::from).collect())
    }
}
impl From<Value> for ExceptionBody {
    fn from(v: Value) -> Self {
        match v {
            Value::String(s) => ExceptionBody::Message(s),
            Value::Null => ExceptionBody::Empty,
            other => ExceptionBody::Object(other),
        }
    }
}

fn reason(status: StatusCode) -> &'static str {
    match status.as_u16() {
        418 => "I'm a teapot",
        421 => "Misdirected",
        _ => status.canonical_reason().unwrap_or("Unknown"),
    }
}

impl HttpException {
    /// `new HttpException(response, status)`.
    pub fn new(response: impl Into<ExceptionBody>, status: impl IntoStatus) -> Self {
        let status = status.into_status();
        let (response, message) = match response.into() {
            ExceptionBody::Empty => {
                let m = reason(status).to_string();
                (json!({ "statusCode": status.as_u16(), "message": m }), m)
            }
            ExceptionBody::Message(m) => (json!({ "statusCode": status.as_u16(), "message": m }), m),
            ExceptionBody::Messages(v) => {
                let m = v.join(", ");
                (json!({ "statusCode": status.as_u16(), "message": v }), m)
            }
            ExceptionBody::Object(o) => {
                let m = o.get("message").map(value_to_message).unwrap_or_else(|| reason(status).to_string());
                (o, m)
            }
        };
        HttpException { status, response, message, cause: None, anyhow_cause: None, internal: false, logged: false }
    }

    /// Body built by the built-in exceptions:
    /// `{ message, error, statusCode }`.
    pub fn with_error(body: impl Into<ExceptionBody>, error: &str, status: StatusCode) -> Self {
        let (response, message) = match body.into() {
            ExceptionBody::Empty => {
                let m = error.to_string();
                (json!({ "message": m, "statusCode": status.as_u16() }), m)
            }
            ExceptionBody::Message(m) => (json!({ "message": m, "error": error, "statusCode": status.as_u16() }), m),
            ExceptionBody::Messages(v) => {
                let m = v.join(", ");
                (json!({ "message": v, "error": error, "statusCode": status.as_u16() }), m)
            }
            ExceptionBody::Object(o) => {
                let m = o.get("message").map(value_to_message).unwrap_or_else(|| error.to_string());
                (o, m)
            }
        };
        HttpException { status, response, message, cause: None, anyhow_cause: None, internal: false, logged: false }
    }

    /// An `http-errors` error (`err.statusCode` + `err.message`) that is NOT an
    /// `HttpException`, e.g. the 413/415 of `body-parser`. `BaseExceptionFilter` answers
    /// `{ statusCode, message }` with that status (and logs it as an unknown error);
    /// `is_internal()` is `true`, so a `@Catch()` filter sees it as an unknown error.
    pub fn http_error(status: u16, message: impl Into<String>) -> Self {
        let message = message.into();
        let mut e = Self::new(json!({ "statusCode": status, "message": message }), status);
        e.internal = true;
        e
    }

    /// Exception of a domain error that declares its own status (see [`HttpError`]).
    ///
    /// - Below 400 the status becomes 500: an error cannot answer 1xx, 2xx or 3xx.
    /// - A 4xx answers `err.body()` (the `Display` text by default) in the format of the built-in
    ///   exceptions, and is not internal.
    /// - A 5xx without a custom `body()` answers the generic body of an internal error and is internal,
    ///   so the error is logged and its text never reaches the client. A custom `body()` is sent as written.
    ///
    /// `err` is always kept as the cause.
    pub fn from_http_error<E: HttpError>(err: E) -> Self {
        let mut status = err.status();
        if status.as_u16() < 400 {
            status = StatusCode::INTERNAL_SERVER_ERROR;
        }
        let custom = err.body();
        let mut e = match custom {
            None if status.is_server_error() => {
                let generic = if status == StatusCode::INTERNAL_SERVER_ERROR { "Internal server error" } else { reason(status) };
                let mut e = Self::new(json!({ "statusCode": status.as_u16(), "message": generic }), status);
                e.message = err.to_string();
                e.internal = true;
                e
            }
            None => Self::with_error(ExceptionBody::Message(err.to_string()), reason(status), status),
            Some(body) => Self::with_error(body, reason(status), status),
        };
        e.cause = Some(Arc::new(err));
        e
    }

    /// 500 for unexpected errors: `{ statusCode: 500, message: "Internal server error" }`.
    pub fn internal(cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        let mut e = Self::new(json!({ "statusCode": 500, "message": "Internal server error" }), StatusCode::INTERNAL_SERVER_ERROR);
        e.message = cause.to_string();
        e.cause = Some(Arc::new(cause));
        e.internal = true;
        e
    }

    /// Converts an `anyhow::Error` into a 500 (or keeps the inner `HttpException`).
    pub fn from_anyhow(err: anyhow::Error) -> Self {
        match err.downcast::<HttpExceptionError>() {
            Ok(HttpExceptionError(e)) => e,
            Err(err) => {
                let mut e = Self::new(json!({ "statusCode": 500, "message": "Internal server error" }), StatusCode::INTERNAL_SERVER_ERROR);
                e.message = err.to_string();
                e.anyhow_cause = Some(Arc::new(err));
                e.internal = true;
                e
            }
        }
    }

    /// Replaces the `error` description: `new BadRequestException('msg', { description })`.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        if let Value::Object(map) = &mut self.response {
            map.insert("error".into(), Value::String(description.into()));
            reorder_status_last(map);
        }
        self
    }

    /// Attaches the underlying cause: `new BadRequestException('msg', { cause })`.
    pub fn with_cause(mut self, cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Arc::new(cause));
        self
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }
    pub fn get_status(&self) -> u16 {
        self.status.as_u16()
    }
    /// The response body (`exception.getResponse()`).
    pub fn get_response(&self) -> &Value {
        &self.response
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn is_internal(&self) -> bool {
        self.internal
    }
    /// Marks the exception as logged by its raiser: [`BaseExceptionFilter`](crate::BaseExceptionFilter) answers it
    /// without writing the `ExceptionsHandler` line.
    pub(crate) fn already_logged(mut self) -> Self {
        self.logged = true;
        self
    }
    pub(crate) fn is_already_logged(&self) -> bool {
        self.logged
    }
    /// The underlying error, if any.
    pub fn cause(&self) -> Option<&(dyn std::error::Error + Send + Sync + 'static)> {
        self.cause.as_deref()
    }
    /// Downcasts the underlying error (`exception instanceof MyError`).
    pub fn cause_is<E: std::error::Error + Send + Sync + 'static>(&self) -> bool {
        self.downcast_cause::<E>().is_some()
    }
    pub fn downcast_cause<E: std::error::Error + Send + Sync + 'static>(&self) -> Option<&E> {
        if let Some(c) = &self.cause {
            let c: &(dyn std::error::Error + 'static) = c.as_ref();
            if let Some(e) = c.downcast_ref::<E>() {
                return Some(e);
            }
        }
        if let Some(a) = &self.anyhow_cause {
            return a.downcast_ref::<E>();
        }
        None
    }

    /// Wraps the exception into an `anyhow::Error` (keeps status when converted back).
    pub fn into_anyhow(self) -> anyhow::Error {
        anyhow::Error::new(HttpExceptionError(self))
    }
}

fn reorder_status_last(map: &mut Map<String, Value>) {
    if let Some(sc) = map.shift_remove("statusCode") {
        map.insert("statusCode".into(), sc);
    }
}

fn value_to_message(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(value_to_message).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

impl fmt::Debug for HttpException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpException").field("status", &self.status.as_u16()).field("response", &self.response).finish()
    }
}

impl fmt::Display for HttpException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Any error converts into a 500 `HttpException` (like any unhandled exception).
impl<E: std::error::Error + Send + Sync + 'static> From<E> for HttpException {
    fn from(e: E) -> Self {
        HttpException::internal(e)
    }
}

/// A domain error that knows its HTTP status.
///
/// A handler declared as `Result<T, AppError>` answers with that status when it returns `Err`; inside a
/// handler returning [`HttpResult`](crate::HttpResult), `.http()?` ([`ResultHttpExt`]) does the same.
/// For a 4xx the default body is the `Display` text in the format of the built-in exceptions.
///
/// For a 5xx without a custom [`HttpError::body`] the client gets the generic internal-error body (the
/// `Display` text is not sent) and the error is logged like any unexpected error, with the exception
/// marked internal. A custom `body()` is sent as written, so keep internal details out of it. A status
/// below 400 is treated as 500.
pub trait HttpError: std::error::Error + Send + Sync + 'static {
    /// The response status.
    fn status(&self) -> StatusCode;

    /// A custom response body, or `None` (the default) for the standard one: the `Display` text for a
    /// 4xx, the generic internal-error body for a 5xx.
    fn body(&self) -> Option<ExceptionBody> {
        None
    }
}

/// `result.http()?` for errors that implement [`HttpError`].
pub trait ResultHttpExt<T> {
    fn http(self) -> Result<T, HttpException>;
}
impl<T, E: HttpError> ResultHttpExt<T> for Result<T, E> {
    fn http(self) -> Result<T, HttpException> {
        self.map_err(HttpException::from_http_error)
    }
}

/// Wrapper so an `HttpException` can travel inside `anyhow::Error`.
#[derive(Debug)]
pub struct HttpExceptionError(pub HttpException);
impl fmt::Display for HttpExceptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for HttpExceptionError {}

/// Anything that can be used as an HTTP status (`u16`, `StatusCode`).
pub trait IntoStatus {
    fn into_status(self) -> StatusCode;
}
impl IntoStatus for StatusCode {
    fn into_status(self) -> StatusCode {
        self
    }
}
impl IntoStatus for u16 {
    fn into_status(self) -> StatusCode {
        StatusCode::from_u16(self).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}
impl IntoStatus for i32 {
    fn into_status(self) -> StatusCode {
        (self as u16).into_status()
    }
}

macro_rules! builtin_exceptions {
    ($($(#[$doc:meta])* $name:ident => $status:ident, $error:literal;)*) => {
        $(
            $(#[$doc])*
            ///
            /// Use it as a value (`Err(NotFoundException)?`) or with a message
            /// (`NotFoundException::new("Cat not found")`).
            #[derive(Debug, Clone, Copy, Default)]
            pub struct $name;

            impl $name {
                #[allow(clippy::new_ret_no_self)]
                pub fn new(body: impl Into<ExceptionBody>) -> HttpException {
                    HttpException::with_error(body, $error, StatusCode::$status)
                }
                /// Exception with the default message.
                pub fn empty() -> HttpException {
                    HttpException::with_error(ExceptionBody::Empty, $error, StatusCode::$status)
                }
            }

            impl From<$name> for HttpException {
                fn from(_: $name) -> Self {
                    $name::empty()
                }
            }
        )*
    };
}

builtin_exceptions! {
    /// 400 Bad Request.
    BadRequestException => BAD_REQUEST, "Bad Request";
    /// 401 Unauthorized.
    UnauthorizedException => UNAUTHORIZED, "Unauthorized";
    /// 403 Forbidden.
    ForbiddenException => FORBIDDEN, "Forbidden";
    /// 404 Not Found.
    NotFoundException => NOT_FOUND, "Not Found";
    /// 405 Method Not Allowed.
    MethodNotAllowedException => METHOD_NOT_ALLOWED, "Method Not Allowed";
    /// 406 Not Acceptable.
    NotAcceptableException => NOT_ACCEPTABLE, "Not Acceptable";
    /// 408 Request Timeout.
    RequestTimeoutException => REQUEST_TIMEOUT, "Request Timeout";
    /// 409 Conflict.
    ConflictException => CONFLICT, "Conflict";
    /// 410 Gone.
    GoneException => GONE, "Gone";
    /// 412 Precondition Failed.
    PreconditionFailedException => PRECONDITION_FAILED, "Precondition Failed";
    /// 413 Payload Too Large.
    PayloadTooLargeException => PAYLOAD_TOO_LARGE, "Payload Too Large";
    /// 415 Unsupported Media Type.
    UnsupportedMediaTypeException => UNSUPPORTED_MEDIA_TYPE, "Unsupported Media Type";
    /// 418 I'm a teapot.
    ImATeapotException => IM_A_TEAPOT, "I'm a teapot";
    /// 421 Misdirected.
    MisdirectedException => MISDIRECTED_REQUEST, "Misdirected";
    /// 422 Unprocessable Entity.
    UnprocessableEntityException => UNPROCESSABLE_ENTITY, "Unprocessable Entity";
    /// 429 Too Many Requests.
    TooManyRequestsException => TOO_MANY_REQUESTS, "Too Many Requests";
    /// 500 Internal Server Error.
    InternalServerErrorException => INTERNAL_SERVER_ERROR, "Internal Server Error";
    /// 501 Not Implemented.
    NotImplementedException => NOT_IMPLEMENTED, "Not Implemented";
    /// 502 Bad Gateway.
    BadGatewayException => BAD_GATEWAY, "Bad Gateway";
    /// 503 Service Unavailable.
    ServiceUnavailableException => SERVICE_UNAVAILABLE, "Service Unavailable";
    /// 504 Gateway Timeout.
    GatewayTimeoutException => GATEWAY_TIMEOUT, "Gateway Timeout";
    /// 505 HTTP Version Not Supported.
    HttpVersionNotSupportedException => HTTP_VERSION_NOT_SUPPORTED, "HTTP Version Not Supported";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_exception_bodies() {
        assert_eq!(
            serde_json::to_string(NotFoundException::empty().get_response()).unwrap(),
            r#"{"message":"Not Found","statusCode":404}"#
        );
        assert_eq!(
            serde_json::to_string(NotFoundException::new("Cat not found").get_response()).unwrap(),
            r#"{"message":"Cat not found","error":"Not Found","statusCode":404}"#
        );
        assert_eq!(
            serde_json::to_string(HttpException::new("Forbidden", 403).get_response()).unwrap(),
            r#"{"statusCode":403,"message":"Forbidden"}"#
        );
        assert_eq!(
            serde_json::to_string(BadRequestException::new("x").with_description("desc").get_response()).unwrap(),
            r#"{"message":"x","error":"desc","statusCode":400}"#
        );
    }

    #[test]
    fn errors_convert() {
        let io = std::io::Error::other("boom");
        let e: HttpException = io.into();
        assert_eq!(e.get_status(), 500);
        assert!(e.cause_is::<std::io::Error>());
        let e: HttpException = NotFoundException.into();
        assert_eq!(e.get_status(), 404);
    }

    #[derive(Debug)]
    enum AppError {
        CatNotFound(u32),
        Gone,
        Boom,
        Internal,
        Teapot503,
        Ok200,
    }
    impl fmt::Display for AppError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                AppError::CatNotFound(id) => write!(f, "Cat #{id} not found"),
                AppError::Gone => write!(f, "gone"),
                AppError::Boom => write!(f, "boom"),
                AppError::Internal => write!(f, "db: password authentication failed"),
                AppError::Teapot503 => write!(f, "later"),
                AppError::Ok200 => write!(f, "fine"),
            }
        }
    }
    impl std::error::Error for AppError {}
    impl HttpError for AppError {
        fn status(&self) -> StatusCode {
            match self {
                AppError::CatNotFound(_) => StatusCode::NOT_FOUND,
                AppError::Gone => StatusCode::GONE,
                AppError::Boom => StatusCode::BAD_GATEWAY,
                AppError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
                AppError::Teapot503 => StatusCode::SERVICE_UNAVAILABLE,
                AppError::Ok200 => StatusCode::OK,
            }
        }
        fn body(&self) -> Option<ExceptionBody> {
            match self {
                AppError::Gone => Some(ExceptionBody::Object(json!({ "code": "CAT_GONE", "statusCode": 410 }))),
                AppError::Teapot503 => Some(ExceptionBody::Object(json!({ "code": "LATER", "statusCode": 503 }))),
                _ => None,
            }
        }
    }

    #[test]
    fn http_error_default_body() {
        let e = HttpException::from_http_error(AppError::CatNotFound(42));
        assert_eq!(e.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            serde_json::to_string(e.get_response()).unwrap(),
            r#"{"message":"Cat #42 not found","error":"Not Found","statusCode":404}"#
        );
        assert_eq!(e.message(), "Cat #42 not found");
    }

    #[test]
    fn http_error_custom_body() {
        let e = HttpException::from_http_error(AppError::Gone);
        assert_eq!(e.status(), StatusCode::GONE);
        assert_eq!(serde_json::to_string(e.get_response()).unwrap(), r#"{"code":"CAT_GONE","statusCode":410}"#);
    }

    #[test]
    fn http_error_4xx_is_not_internal_and_keeps_the_cause() {
        let e = HttpException::from_http_error(AppError::CatNotFound(1));
        assert!(!e.is_internal());
        assert!(matches!(e.downcast_cause::<AppError>(), Some(AppError::CatNotFound(1))));
    }

    #[test]
    fn http_error_5xx_hides_the_display_text_and_is_internal() {
        let e = HttpException::from_http_error(AppError::Boom);
        assert_eq!(e.status(), StatusCode::BAD_GATEWAY);
        assert!(e.is_internal());
        assert_eq!(serde_json::to_string(e.get_response()).unwrap(), r#"{"statusCode":502,"message":"Bad Gateway"}"#);
        assert_eq!(e.message(), "boom", "the operator still sees the error text");
        assert!(matches!(e.downcast_cause::<AppError>(), Some(AppError::Boom)));
        let e = HttpException::from_http_error(AppError::Internal);
        assert_eq!(serde_json::to_string(e.get_response()).unwrap(), r#"{"statusCode":500,"message":"Internal server error"}"#);
    }

    #[test]
    fn http_error_5xx_with_a_custom_body_sends_it() {
        let e = HttpException::from_http_error(AppError::Teapot503);
        assert_eq!(e.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!e.is_internal());
        assert_eq!(serde_json::to_string(e.get_response()).unwrap(), r#"{"code":"LATER","statusCode":503}"#);
    }

    #[test]
    fn http_error_below_400_answers_500() {
        let e = HttpException::from_http_error(AppError::Ok200);
        assert_eq!(e.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(e.is_internal());
        assert_eq!(serde_json::to_string(e.get_response()).unwrap(), r#"{"statusCode":500,"message":"Internal server error"}"#);
    }

    #[test]
    fn result_http_ext_converts_the_error() {
        let ok: Result<u8, AppError> = Ok(1);
        assert_eq!(ok.http().unwrap(), 1);
        let err: Result<u8, AppError> = Err(AppError::CatNotFound(1));
        assert_eq!(err.http().unwrap_err().get_status(), 404);
    }
}
