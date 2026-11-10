//! Login with the `local` strategy: argon2id hashes in the users store, the same answer for a wrong password and an
//! unknown user, and a token that opens the profile.

use corvis::prelude::*;
use corvis::testing::request;

async fn app() -> CorvisApplication {
    // SAFETY: set before the application reads the environment; this binary has a single test.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
    }
    let mut app = cats_app::create_app().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn the_documented_credentials_log_in_and_the_rest_get_the_same_401() {
    let app = app().await;
    let c = request(&app);

    let ok = c.post("/api/v1/auth/login").json(&json!({ "username": "john", "password": "changeme" })).send().await;
    assert_eq!(ok.status, 200, "{}", ok.text());
    let token = ok.json_value()["access_token"].as_str().unwrap().to_string();
    let profile = c.get("/api/v1/auth/profile").header("authorization", &format!("Bearer {token}")).send().await;
    assert_eq!(profile.status, 200);
    assert_eq!(profile.json_value()["username"], "john");
    assert!(!profile.text().contains("argon2"), "the hash never reaches a response");

    let wrong = c.post("/api/v1/auth/login").json(&json!({ "username": "john", "password": "wrong-password" })).send().await;
    let unknown = c.post("/api/v1/auth/login").json(&json!({ "username": "nobody", "password": "wrong-password" })).send().await;
    assert_eq!(wrong.status, 401);
    // The body carries a timestamp, so compare what the client can tell apart.
    assert_eq!((unknown.status, unknown.json_value()["message"].clone()), (401, wrong.json_value()["message"].clone()));

    let missing = c.post("/api/v1/auth/login").json(&json!({ "username": "john" })).send().await;
    assert_eq!((missing.status, missing.json_value()["message"].clone()), (401, wrong.json_value()["message"].clone()));
}
