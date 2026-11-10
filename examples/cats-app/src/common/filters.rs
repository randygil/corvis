use corvis::prelude::*;

/// An `HttpExceptionFilter` that adds `timestamp` and `path`.
#[injectable]
pub struct HttpExceptionFilter;

#[async_trait]
impl ExceptionFilter for HttpExceptionFilter {
    fn catches(&self, exception: &HttpException) -> bool {
        // `@Catch(HttpException)`: only intentional exceptions, not unexpected errors.
        !exception.is_internal()
    }

    async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response {
        let request = host.switch_to_http().get_request();
        let status = exception.get_status();
        let message = exception.get_response().get("message").cloned().unwrap_or(json!(exception.message()));
        Response::json(json!({
            "statusCode": status,
            "message": message,
            "timestamp": unix_timestamp(),
            // The redacted form: a callback's `code` and `state` (and any marked credential) never come back in a body.
            "path": request.redacted_url(),
        }))
        .with_status(status)
    }
}

fn unix_timestamp() -> String {
    // RFC 3339 timestamp without pulling extra crates in the example.
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs}")
}
