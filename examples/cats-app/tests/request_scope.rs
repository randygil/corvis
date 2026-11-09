//! The request-scoped `RequestContext`: the interceptor and a handler see the same per-request instance.

use corvis::prelude::*;
use corvis::testing::request;

async fn app() -> CorvisApplication {
    // SAFETY: set before the application reads the environment.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
    }
    let mut app = cats_app::create_app().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn the_request_id_header_is_echoed() {
    let app = app().await;
    let response = request(&app).get("/api/v1/cats").set("X-Request-Id", "abc-123").send().await;
    response.expect_status(200).expect_header("x-request-id", "abc-123");
}

#[corvis::test]
async fn a_request_id_is_generated_per_request() {
    let app = app().await;
    let client = request(&app);
    let a = client.get("/api/v1/cats").send().await;
    let b = client.get("/api/v1/cats").send().await;
    let (a, b) = (a.header("x-request-id").unwrap().to_string(), b.header("x-request-id").unwrap().to_string());
    assert!(!a.is_empty() && !b.is_empty());
    assert_ne!(a, b);
}

#[corvis::test]
async fn the_handler_and_the_interceptor_share_the_request_context() {
    let app = app().await;
    let response = request(&app).get("/api/v1/cats/request-id").set("X-Request-Id", "same-one").send().await;
    response.expect_status(200).expect_header("x-request-id", "same-one");
    assert_eq!(response.json_value()["requestId"], "same-one");
    let generated = request(&app).get("/api/v1/cats/request-id").send().await;
    assert_eq!(generated.json_value()["requestId"], generated.header("x-request-id").unwrap());
}
