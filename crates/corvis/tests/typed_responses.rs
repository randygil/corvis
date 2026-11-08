//! Typed handler return values and typed domain errors.

use std::fmt;

use corvis::prelude::*;
use corvis::testing::{Test, request};
use corvis::{ExceptionBody, StatusCode};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
struct CatDto {
    id: u32,
    name: String,
}

#[derive(Debug, Serialize)]
struct JobDto {
    id: u32,
}

#[derive(Debug)]
enum AppError {
    CatNotFound(u32),
}
impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::CatNotFound(id) => write!(f, "Cat #{id} not found"),
        }
    }
}
impl std::error::Error for AppError {}
impl HttpError for AppError {
    fn status(&self) -> StatusCode {
        StatusCode::NOT_FOUND
    }
}

#[derive(Debug)]
struct Teapot;
impl fmt::Display for Teapot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "short and stout")
    }
}
impl std::error::Error for Teapot {}
impl HttpError for Teapot {
    fn status(&self) -> StatusCode {
        StatusCode::IM_A_TEAPOT
    }
    fn body(&self) -> ExceptionBody {
        ExceptionBody::Object(json!({ "code": "TEAPOT", "statusCode": 418 }))
    }
}

fn find(id: u32) -> Result<CatDto, AppError> {
    Err(AppError::CatNotFound(id))
}

#[controller("t")]
pub struct TypedController;

#[routes]
impl TypedController {
    #[get("struct")]
    async fn as_struct(&self) -> HttpResult<CatDto> {
        Ok(CatDto { id: 1, name: "Tom".into() })
    }

    #[get("value")]
    async fn as_value(&self) -> HttpResult<Value> {
        Ok(json!({ "id": 1, "name": "Tom" }))
    }

    #[get("accepted")]
    async fn accepted(&self) -> (StatusCode, Json<JobDto>) {
        (StatusCode::ACCEPTED, Json(JobDto { id: 7 }))
    }

    #[get("accepted-u16")]
    async fn accepted_u16(&self) -> (u16, Json<JobDto>) {
        (202, Json(JobDto { id: 8 }))
    }

    #[post("created")]
    #[http_code(201)]
    async fn created(&self) -> CatDto {
        CatDto { id: 2, name: "Ann".into() }
    }

    #[get("direct")]
    async fn direct(&self) -> Result<CatDto, AppError> {
        find(42)
    }

    #[get("direct-body")]
    async fn direct_body(&self) -> Result<CatDto, Teapot> {
        Err(Teapot)
    }

    #[get("builtin")]
    async fn builtin(&self) -> HttpResult<CatDto> {
        Err(NotFoundException::new("Cat #42 not found"))
    }

    #[get("unit")]
    async fn unit(&self) -> Result<CatDto, NotFoundException> {
        Err(NotFoundException)
    }

    #[get("io")]
    async fn io(&self) -> Result<CatDto, std::io::Error> {
        Err(std::io::Error::other("disk on fire"))
    }

    #[get("anyhow")]
    async fn any(&self) -> anyhow::Result<CatDto> {
        Err(anyhow::anyhow!("anyhow failure"))
    }

    #[get("dot-http")]
    async fn dot_http(&self) -> HttpResult<CatDto> {
        let cat = find(42).http()?;
        Ok(cat)
    }

    #[get("plain-question")]
    async fn plain_question(&self) -> HttpResult<CatDto> {
        let cat = find(42)?;
        Ok(cat)
    }
}

#[module(controllers = [TypedController])]
pub struct TypedModule;

async fn app() -> CorvisApplication {
    let module = Test::create_testing_module().import::<TypedModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn struct_is_serialized_as_json() {
    let app = app().await;
    let client = request(&app);
    let res = client.get("/t/struct").send().await;
    res.expect_status(200);
    assert_eq!(res.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(res.text(), r#"{"id":1,"name":"Tom"}"#);
    let value = client.get("/t/value").send().await;
    assert_eq!(value.text(), res.text());
}

#[corvis::test]
async fn status_accompanies_a_typed_body() {
    let app = app().await;
    let client = request(&app);
    let res = client.get("/t/accepted").send().await;
    res.expect_status(202);
    assert_eq!(res.text(), r#"{"id":7}"#);
    let res = client.get("/t/accepted-u16").send().await;
    res.expect_status(202);
    assert_eq!(res.text(), r#"{"id":8}"#);
    let res = client.post("/t/created").send().await;
    res.expect_status(201);
    assert_eq!(res.text(), r#"{"id":2,"name":"Ann"}"#);
}

#[corvis::test]
async fn http_error_levels() {
    let app = app().await;
    let client = request(&app);
    let not_found = r#"{"message":"Cat #42 not found","error":"Not Found","statusCode":404}"#;

    let res = client.get("/t/direct").send().await;
    res.expect_status(404);
    assert_eq!(res.text(), not_found);

    let res = client.get("/t/direct-body").send().await;
    res.expect_status(418);
    assert_eq!(res.text(), r#"{"code":"TEAPOT","statusCode":418}"#);

    let res = client.get("/t/builtin").send().await;
    res.expect_status(404);
    assert_eq!(res.text(), not_found);

    client.get("/t/unit").send().await.expect_status(404).expect_json(json!({ "message": "Not Found", "statusCode": 404 }));

    for path in ["/t/io", "/t/anyhow", "/t/plain-question"] {
        let res = client.get(path).send().await;
        res.expect_status(500);
        assert_eq!(res.text(), r#"{"statusCode":500,"message":"Internal server error"}"#, "{path}");
    }

    let res = client.get("/t/dot-http").send().await;
    res.expect_status(404);
    assert_eq!(res.text(), not_found);
}
