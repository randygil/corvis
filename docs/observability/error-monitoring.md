---
status: unavailable
---

# Error monitoring

There is no integration with error monitoring services.

## Alternative

Register a global [exception filter](/exception-filters) that reports unexpected errors and delegates the response to the default behavior:

```rust
pub struct ReportingFilter;

#[async_trait]
impl ExceptionFilter for ReportingFilter {
    async fn catch(&self, exception: HttpException, _host: &ArgumentsHost) -> Response {
        if exception.is_internal() {
            Logger::new("Errors").error(exception.message());
            // Send the error here to the service you use.
        }
        BaseExceptionFilter::handle(&exception)
    }
}

// main.rs
app.use_global_filters(ReportingFilter);
```
