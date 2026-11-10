//! The machine route `GET /api/v1/cats/export`: a service with the configured `SERVICE_API_KEY` gets in, a wrong or
//! missing key gets the same 401, and a signed-in user still works.

use corvis::prelude::*;
use corvis::testing::request;

const KEY: &str = "cats-service-key-0123456789abcdef012345";

async fn app() -> CorvisApplication {
    // SAFETY: set before the application reads the environment; this binary has a single test.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
        std::env::set_var("SERVICE_API_KEY", KEY);
    }
    let mut app = cats_app::create_app().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn a_service_key_opens_the_machine_route_and_the_rest_get_the_same_401() {
    let app = app().await;
    let c = request(&app);

    let ok = c.get("/api/v1/cats/export").header("x-api-key", KEY).send().await;
    assert_eq!(ok.status, 200, "{}", ok.text());
    assert_eq!(ok.json_value()["caller"], json!({ "service": "reports" }));

    let wrong = c.get("/api/v1/cats/export").header("x-api-key", "cats-service-key-wrong-0123456789abcdef0").send().await;
    let missing = c.get("/api/v1/cats/export").send().await;
    let short = c.get("/api/v1/cats/export").header("x-api-key", "x").send().await;
    assert_eq!(wrong.status, 401);
    for other in [&missing, &short] {
        assert_eq!((other.status, other.json_value()["message"].clone()), (401, wrong.json_value()["message"].clone()));
    }
    assert!(!wrong.text().contains("wrong"), "the key never reaches a response");

    // The key in the query string is not a source.
    let query = c.get(&format!("/api/v1/cats/export?api_key={KEY}")).send().await;
    assert_eq!(query.status, 401);

    // A signed-in user is accepted by the first strategy of the chain.
    let login = c.post("/api/v1/auth/login").json(&json!({ "username": "john", "password": "changeme" })).send().await;
    let token = login.json_value()["access_token"].as_str().unwrap().to_string();
    let user = c.get("/api/v1/cats/export").header("authorization", &format!("Bearer {token}")).send().await;
    assert_eq!(user.status, 200);
    assert_eq!(user.json_value()["caller"]["username"], "john");

    // The other routes keep their guard: the service key is not a user.
    let create = c.post("/api/v1/cats").header("x-api-key", KEY).json(&json!({ "name": "Tom", "age": 3 })).send().await;
    assert_eq!(create.status, 401);
}
