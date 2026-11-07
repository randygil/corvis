use corvis::prelude::*;
use corvis::testing::{Test, request};
use corvis::zod_dto;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCatDto {
    pub name: String,
    pub age: u32,
    pub breed: String,
    pub tags: Vec<String>,
}

zod_dto!(
    CreateCatDto,
    z::object! {
        name: z::string().trim().min(1),
        age: z::int().nonnegative().max(30),
        breed: z::string().default("mestizo"),
        tags: z::array(z::string()).max(3).default(json!([])),
    }
);

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub page: u32,
    pub active: bool,
}

fn list_query_schema() -> z::Schema {
    z::object! {
        page: z::coerce::number().int().min(1).default(1),
        active: z::coerce::boolean().default(true),
    }
    .into()
}

#[controller("cats")]
pub struct CatsController;

#[routes]
impl CatsController {
    /// Validated by the global `ZodValidationPipe` through `zod_dto!`.
    #[post]
    fn create(&self, #[body] dto: CreateCatDto) -> CreateCatDto {
        dto
    }

    /// Explicit schema on a single parameter.
    #[get]
    fn list(&self, #[query(ZodValidationPipe::new(list_query_schema()))] q: ListQuery) -> Value {
        json!({ "page": q.page, "active": q.active })
    }

    /// `@UsePipes(new ZodValidationPipe(schema))`.
    #[post("rename")]
    #[use_pipes(ZodValidationPipe::new(z::object! { name: z::string().min(3) }.strict()))]
    fn rename(&self, #[body] body: Value) -> Value {
        body
    }
}

#[module(controllers = [CatsController])]
pub struct AppModule;

#[corvis::test]
async fn zod_validation_pipe() {
    z::set_version(z::ZodVersion::V3);
    let module = Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.use_global_pipes(ZodValidationPipe::auto());
    app.init().await.unwrap();
    let client = request(&app);

    client
        .post("/cats")
        .json(&json!({ "name": "  Tom ", "age": 3, "unknown": true }))
        .send()
        .await
        .expect_status(201)
        .expect_json(json!({ "name": "Tom", "age": 3, "breed": "mestizo", "tags": [] }));

    client
        .post("/cats")
        .json(&json!({ "name": "", "age": 3.5, "tags": ["a", "b", "c", "d"] }))
        .send()
        .await
        .expect_status(400)
        .expect_json(json!({
            "statusCode": 400,
            "message": "Validation failed",
            "errors": [
                { "code": "too_small", "minimum": 1, "type": "string", "inclusive": true, "exact": false, "path": ["name"], "message": "String must contain at least 1 character(s)" },
                { "code": "invalid_type", "expected": "integer", "received": "float", "path": ["age"], "message": "Expected integer, received float" },
                { "code": "too_big", "maximum": 3, "type": "array", "inclusive": true, "exact": false, "path": ["tags"], "message": "Array must contain at most 3 element(s)" },
            ]
        }));

    client.get("/cats").send().await.expect_status(200).expect_json(json!({ "page": 1, "active": true }));
    client
        .get("/cats")
        .query("page", "3")
        .query("active", "")
        .send()
        .await
        .expect_status(200)
        .expect_json(json!({ "page": 3, "active": false }));
    let res = client.get("/cats").query("page", "0").send().await;
    res.expect_status(400);
    assert_eq!(res.json_value()["errors"][0]["path"], json!(["page"]));

    client.post("/cats/rename").json(&json!({ "name": "Garfield" })).send().await.expect_status(201);
    let res = client.post("/cats/rename").json(&json!({ "name": "Garfield", "x": 1 })).send().await;
    res.expect_status(400);
    assert_eq!(res.json_value()["errors"][0]["message"], json!("Unrecognized key(s) in object: 'x'"));
}
