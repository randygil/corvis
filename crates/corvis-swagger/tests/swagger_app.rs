//! End-to-end: a Corvis app with `#[api_*]` attributes and zod DTOs produces the same OpenAPI
//! document the Node.js generators produce for the equivalent TypeScript app, and
//! `SwaggerModule::setup` / the Scalar reference serve it.

use corvis::prelude::*;
use corvis::testing::request;
use corvis::zod_dto;
use corvis_swagger::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateCatDto {
    pub name: String,
    pub lives: i64,
}
zod_dto!(CreateCatDto, z::object! { name: z::string().min(1), lives: z::int().min(1).default(9) });

#[derive(Debug, Deserialize, Serialize)]
pub struct CatDto {
    pub id: String,
    pub name: String,
}
zod_dto!(CatDto, z::object! { id: z::uuid(), name: z::string() });

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ListCatsQuery {
    #[serde(default)]
    pub q: Option<String>,
}
zod_dto!(ListCatsQuery, z::object! { q: z::string().max(20).optional() });

#[controller("cats")]
#[api_tags("cats")]
#[api_bearer_auth]
#[api_unauthorized_response(description = "Missing token")]
pub struct CatsController;

#[routes]
impl CatsController {
    #[post]
    #[api_operation(summary = "Create a cat")]
    #[api_created_response(type = CatDto)]
    #[api_conflict_response(description = "Duplicated")]
    fn create(&self, #[body] dto: CreateCatDto) -> Value {
        json!({ "id": "00000000-0000-4000-8000-000000000000", "name": dto.name })
    }

    #[get]
    #[api_ok_response(type = [CatDto])]
    fn list(&self, #[query] _q: ListCatsQuery) -> Value {
        json!([])
    }

    #[get(":id")]
    #[api_param(name = "id", example = "abc")]
    #[api_not_found_response(description = "Not found")]
    #[apply_decorators(corvis_swagger::decorators::zod_response::<CatDto>(Some(202), Some("The cat")))]
    fn one(&self, #[param("id")] id: String) -> Value {
        json!({ "id": id, "name": "Tom", "extra": true })
    }
}

#[module(controllers = [CatsController], providers = [Provider::app_interceptor::<z::ZodSerializerInterceptor>()])]
pub struct AppModule;

#[corvis::test]
async fn document_matches_reference_generator() {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.set_global_prefix("api");
    app.silence_route_logs();
    let config = DocumentBuilder::new().set_title("Cats").set_version("1.0").add_bearer_auth().build();
    let doc = cleanup_open_api_doc(SwaggerModule::create_document(&app, &config));
    let v = doc.as_value();
    // Expected values were produced by the reference Node.js generators for the same app.
    assert_eq!(
        v["paths"]["/api/cats"]["post"],
        json!({
            "operationId": "CatsController_create",
            "parameters": [],
            "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateCatDto" } } } },
            "responses": {
                "201": { "description": "", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CatDto" } } } },
                "401": { "description": "Missing token" },
                "409": { "description": "Duplicated" }
            },
            "security": [{ "bearer": [] }],
            "summary": "Create a cat",
            "tags": ["cats"]
        })
    );
    assert_eq!(
        serde_json::to_string(&v["paths"]["/api/cats"]["get"]["parameters"]).unwrap(),
        r#"[{"name":"q","required":false,"in":"query","schema":{"maxLength":20,"type":"string"}}]"#
    );
    assert_eq!(
        serde_json::to_string(&v["components"]["schemas"]["CreateCatDto"]).unwrap(),
        r#"{"type":"object","properties":{"name":{"type":"string","minLength":1},"lives":{"default":9,"type":"integer","minimum":1,"maximum":9007199254740991}},"required":["name"]}"#
    );
    let one = &v["paths"]["/api/cats/{id}"]["get"];
    assert_eq!(one["responses"]["202"]["description"], "The cat");
    assert_eq!(one["responses"]["202"]["content"]["application/json"]["schema"]["$ref"], "#/components/schemas/CatDto_Output");
    // As in the reference generator, `example` of an explicit @ApiParam moves into `schema`.
    assert_eq!(serde_json::to_string(&one["parameters"]).unwrap(), r#"[{"name":"id","required":true,"in":"path","schema":{"example":"abc","type":"string"}}]"#);

    // ZodResponse: status 202 and the response serialized through the DTO schema.
    SwaggerModule::setup("docs", &mut app, doc.clone());
    scalar::mount(&mut app, "/reference", ApiReferenceOptions::new().integration("express").document(&doc));
    app.init().await.unwrap();
    let c = request(&app);
    let r = c.get("/api/cats/00000000-0000-4000-8000-000000000007").send().await;
    assert_eq!(r.status, 202);
    assert_eq!(r.json_value(), json!({ "id": "00000000-0000-4000-8000-000000000007", "name": "Tom" }));
    let r = c.get("/docs-json").send().await;
    assert_eq!(r.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(r.text(), doc.to_json());
    let r = c.get("/docs-yaml").send().await;
    assert!(r.text().starts_with("openapi: 3.0.0\n"));
    let r = c.get("/docs").send().await;
    assert!(r.text().contains("<script src=\"./docs/swagger-ui-init.js\"> </script>"));
    let r = c.get("/docs/swagger-ui-init.js").send().await;
    assert!(r.text().contains("\"swaggerDoc\": {"));
    let r = c.get("/docs/swagger-ui.css").send().await;
    assert_eq!(r.status, 302);
    let r = c.post("/Reference/anything").send().await;
    assert_eq!(r.status, 200);
    assert!(r.text().contains("Scalar.createApiReference('#app', {\n        \"_integration\": \"express\",\n        \"content\": {"));
}
