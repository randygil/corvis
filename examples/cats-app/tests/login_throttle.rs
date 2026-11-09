//! The login route has its own limit: 5 attempts per minute, while the global throttler allows 100.

use corvis::prelude::*;
use corvis::testing::request;

#[corvis::test]
async fn the_sixth_login_attempt_in_a_row_is_rejected() {
    // SAFETY: set before the application reads the environment; this binary has a single test.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
    }
    let mut app = cats_app::create_app().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);
    let attempt = || c.post("/api/v1/auth/login").json(&json!({ "username": "john", "password": "wrong" }));
    for i in 1..=5 {
        let r = attempt().send().await;
        assert_eq!(r.status, 401, "attempt {i}");
        r.expect_header("x-ratelimit-limit", "5");
    }
    let r = attempt().send().await;
    assert_eq!(r.status, 429);
    assert!(r.header("retry-after").is_some());
    // Other routes keep the global limit.
    let r = c.get("/api/v1/auth/profile").send().await;
    assert_eq!(r.status, 401);
    r.expect_header("x-ratelimit-limit", "100");
}
