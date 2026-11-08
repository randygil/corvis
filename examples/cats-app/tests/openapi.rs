//! The OpenAPI document published by `create_app()` (first repository test of `#[derive(ApiSchema)]`).

use corvis::prelude::*;
use corvis::testing::request;

async fn document() -> Value {
    // SAFETY: set before the application reads the environment; every test sets the same value.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
    }
    let mut app = cats_app::create_app().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);
    let r = c.get("/docs-json").send().await;
    assert_eq!(r.status, 200);
    let doc = r.json_value();
    let r = c.get("/docs").send().await;
    assert_eq!(r.status, 200);
    doc
}

#[corvis::test]
async fn document_describes_the_cats_api() {
    let v = document().await;

    // Versioned routes with their tags.
    assert_eq!(v["paths"]["/api/v1/cats"]["get"]["tags"], json!(["cats"]));
    assert_eq!(v["paths"]["/api/v1/cats"]["post"]["tags"], json!(["cats"]));
    assert!(v["paths"]["/api/v1/cats/{id}"]["get"].is_object());
    assert!(v["paths"]["/api/v1/cats/{id}"]["patch"].is_object());
    assert_eq!(v["paths"]["/api/v1/auth/login"]["post"]["tags"], json!(["auth"]));

    // Schemas derived with `#[derive(ApiSchema)]`.
    let create = &v["components"]["schemas"]["CreateCatDto"];
    for p in ["name", "age", "breed"] {
        assert!(create["properties"][p].is_object(), "CreateCatDto.{p}: {create}");
    }
    assert_eq!(create["properties"]["name"]["description"], "Name of the cat");
    assert_eq!(create["properties"]["age"]["example"], 3);
    assert!(v["components"]["schemas"]["Cat"]["properties"]["ownerId"].is_object(), "{}", v["components"]["schemas"]["Cat"]);
    assert!(v["components"]["schemas"]["Cat"]["properties"]["owner_id"].is_null());
    assert_eq!(
        v["paths"]["/api/v1/auth/login"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/SignInDto"
    );
    assert_eq!(
        v["paths"]["/api/v1/cats"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/CreateCatDto"
    );
}

#[corvis::test]
async fn document_declares_bearer_auth() {
    let v = document().await;
    assert_eq!(v["components"]["securitySchemes"]["bearer"]["type"], "http");
    assert_eq!(v["components"]["securitySchemes"]["bearer"]["scheme"], "bearer");
    assert_eq!(v["paths"]["/api/v1/cats"]["post"]["security"], json!([{ "bearer": [] }]));
    // Public routes are not marked as protected.
    assert!(v["paths"]["/api/v1/cats"]["get"]["security"].is_null());
}
