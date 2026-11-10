//! The global guard authenticates with the `jwt` strategy: a token from `/auth/login` opens the protected routes,
//! anything else is a 401, and public routes need no token.

use corvis::prelude::*;
use corvis::testing::request;

#[corvis::test]
async fn the_jwt_strategy_guards_the_routes() {
    // SAFETY: set before the application reads the environment; this binary has a single test.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
    }
    let mut app = cats_app::create_app().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);

    let login = c.post("/api/v1/auth/login").json(&json!({ "username": "john", "password": "changeme" })).send().await;
    assert_eq!(login.status, 200, "{}", login.text());
    let token = login.json_value()["access_token"].as_str().unwrap().to_string();

    let profile = c.get("/api/v1/auth/profile").bearer(&token).send().await;
    assert_eq!(profile.status, 200, "{}", profile.text());
    assert_eq!(profile.json_value()["username"], "john");
    assert_eq!(profile.json_value()["sub"], 1);

    for bearer in ["abc", "a.b.c"] {
        assert_eq!(c.get("/api/v1/auth/profile").bearer(bearer).send().await.status, 401, "{bearer}");
    }
    assert_eq!(c.get("/api/v1/auth/profile").send().await.status, 401);
    // The token is read only from the Authorization header.
    assert_eq!(c.get("/api/v1/auth/profile").query("access_token", &token).send().await.status, 401);

    // A public route needs no token.
    assert_eq!(c.get("/api/v1/cats").send().await.status, 200);
}
