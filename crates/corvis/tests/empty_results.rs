//! A handler result with no value answers an empty body, whatever the return type: the rule of the
//! original framework's Express adapter (`null` and `undefined` answer `res.send()`).
//! `fixtures/empty_results/empty_results.json` holds the answers recorded from the original
//! (`fixtures/empty_results/generate.mjs`); the first test replays them over a real socket.

use std::time::Duration;

use corvis::prelude::*;
use corvis::testing::{Test, request};
use corvis::zod_dto;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize)]
pub struct CatDto {
    name: String,
}

fn tom() -> CatDto {
    CatDto { name: "Tom".into() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrictDto {
    name: String,
}
zod_dto!(StrictDto, z::object! { name: z::string() });

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NullableDto {
    name: String,
}
zod_dto!(NullableDto, z::object! { name: z::string() }.nullable());

#[controller("r")]
pub struct EmptyController;

#[routes]
impl EmptyController {
    // Rows of the recording.
    #[get("null")]
    fn null(&self) -> Value {
        Value::Null
    }

    #[get("undefined")]
    fn undefined(&self) -> Option<CatDto> {
        None
    }

    #[get("void")]
    fn void(&self) {}

    #[get("async-null")]
    async fn async_null(&self) -> HttpResult<Option<CatDto>> {
        Ok(None)
    }

    #[post("null")]
    fn post_null(&self) -> Option<CatDto> {
        None
    }

    #[get("null-204")]
    #[http_code(204)]
    fn null_204(&self) -> Option<CatDto> {
        None
    }

    #[get("null-json-header")]
    #[header("Content-Type", "application/json")]
    fn null_json_header(&self) -> Option<CatDto> {
        None
    }

    #[get("obj")]
    fn obj(&self) -> Value {
        json!({ "a": 1 })
    }

    #[get("str")]
    fn str(&self) -> String {
        "hi".into()
    }

    #[get("empty-str")]
    fn empty_str(&self) -> String {
        String::new()
    }

    #[get("array-null")]
    fn array_null(&self) -> Value {
        json!([null])
    }

    // Other return types.
    #[get("opt-dto-none")]
    fn opt_dto_none(&self) -> Option<CatDto> {
        None
    }

    #[get("opt-dto-some")]
    fn opt_dto_some(&self) -> Option<CatDto> {
        Some(tom())
    }

    #[get("res-opt-dto-none")]
    fn res_opt_dto_none(&self) -> HttpResult<Option<CatDto>> {
        Ok(None)
    }

    #[get("res-opt-dto-some")]
    fn res_opt_dto_some(&self) -> HttpResult<Option<CatDto>> {
        Ok(Some(tom()))
    }

    #[get("opt-value-none")]
    fn opt_value_none(&self) -> Option<Value> {
        None
    }

    #[get("opt-value-some")]
    fn opt_value_some(&self) -> Option<Value> {
        Some(json!({ "name": "Tom" }))
    }

    #[get("res-opt-text-none")]
    fn res_opt_text_none(&self) -> HttpResult<Option<String>> {
        Ok(None)
    }

    #[get("res-opt-text-some")]
    fn res_opt_text_some(&self) -> HttpResult<Option<String>> {
        Ok(Some("hi".into()))
    }

    #[get("res-null")]
    fn res_null(&self) -> HttpResult<Value> {
        Ok(Value::Null)
    }

    #[get("unit-null")]
    fn unit_null(&self) -> HttpResult<()> {
        Ok(())
    }

    #[get("json-none")]
    fn json_none(&self) -> Json<Option<CatDto>> {
        Json(None)
    }

    #[get("response-json-null")]
    fn response_json_null(&self) -> Response {
        Response::json(Value::Null)
    }

    // Interceptors.
    #[get("identity")]
    #[use_interceptors(IdentityInterceptor)]
    fn identity(&self) -> Option<CatDto> {
        None
    }

    #[get("envelope")]
    #[use_interceptors(EnvelopeInterceptor)]
    fn envelope(&self) -> Option<CatDto> {
        None
    }

    #[get("to-null")]
    #[use_interceptors(ToNullInterceptor)]
    fn to_null(&self) -> Value {
        json!({ "a": 1 })
    }

    // Serializer.
    #[get("strict")]
    #[use_interceptors(z::ZodSerializerInterceptor)]
    #[set_metadata(z::ZOD_SERIALIZER_DTO_OPTIONS, z::ZodSerializerDto::dto::<StrictDto>())]
    fn strict(&self) -> Option<CatDto> {
        None
    }

    #[get("strict-unit")]
    #[use_interceptors(z::ZodSerializerInterceptor)]
    #[set_metadata(z::ZOD_SERIALIZER_DTO_OPTIONS, z::ZodSerializerDto::dto::<StrictDto>())]
    fn strict_unit(&self) -> Option<Value> {
        None
    }

    #[get("nullable")]
    #[use_interceptors(z::ZodSerializerInterceptor)]
    #[set_metadata(z::ZOD_SERIALIZER_DTO_OPTIONS, z::ZodSerializerDto::dto::<NullableDto>())]
    fn nullable(&self) -> Option<CatDto> {
        None
    }

    // Cache.
    #[get("cached")]
    #[use_interceptors(CacheInterceptor)]
    fn cached(&self) -> Option<CatDto> {
        None
    }
}

#[injectable]
pub struct IdentityInterceptor;

#[async_trait]
impl CorvisInterceptor for IdentityInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        Ok(next.handle().await?.map_data(|data| data))
    }
}

#[injectable]
pub struct EnvelopeInterceptor;

#[async_trait]
impl CorvisInterceptor for EnvelopeInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        Ok(next.handle().await?.map_data(|data| json!({ "data": data })))
    }
}

#[injectable]
pub struct ToNullInterceptor;

#[async_trait]
impl CorvisInterceptor for ToNullInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        Ok(next.handle().await?.map_data(|_| Value::Null))
    }
}

#[module(imports = [CacheModule::register(CacheOptions::default())], controllers = [EmptyController])]
pub struct EmptyModule;

async fn app() -> CorvisApplication {
    let module = Test::create_testing_module().import::<EmptyModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.set_x_powered_by("Express");
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

/// A parsed HTTP/1.1 answer read from a socket: status, headers (lower-cased names) and body.
struct Wire {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Wire {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

async fn raw(addr: std::net::SocketAddr, method: &str, path: &str) -> Wire {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await.unwrap().unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("a complete answer");
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .filter(|(k, _)| !matches!(k.as_str(), "date" | "connection" | "keep-alive"))
        .collect();
    Wire { status, headers, body: body.to_string() }
}

fn recorded() -> Value {
    serde_json::from_str(include_str!("fixtures/empty_results/empty_results.json")).unwrap()
}

#[corvis::test]
async fn recorded_answers_of_the_original_are_replayed() {
    let fx = recorded();
    let cases = fx["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 12, "the fixture rows changed: update the controller");
    let mut app = app().await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    for case in cases {
        let method = case["method"].as_str().unwrap();
        let path = case["path"].as_str().unwrap();
        let got = raw(addr, method, path).await;
        let at = format!("{method} {path}");
        assert_eq!(u64::from(got.status), case["status"].as_u64().unwrap(), "status of {at}");
        assert_eq!(got.body, case["body"].as_str().unwrap(), "body of {at}");
        let want = case["headers"].as_object().unwrap();
        for name in ["content-type", "content-length", "etag", "x-powered-by"] {
            assert_eq!(got.header(name), want.get(name).and_then(Value::as_str), "{name} of {at}");
        }
    }
    app.close().await.unwrap();
}

#[corvis::test]
async fn every_valueless_return_type_answers_the_same_empty_body() {
    let app = app().await;
    let client = request(&app);
    for path in [
        "/r/opt-dto-none",
        "/r/res-opt-dto-none",
        "/r/opt-value-none",
        "/r/res-opt-text-none",
        "/r/res-null",
        "/r/unit-null",
        "/r/null",
        "/r/void",
    ] {
        let res = client.get(path).send().await;
        res.expect_status(200);
        assert_eq!(res.text(), "", "{path}");
        assert_eq!(res.header("content-type"), None, "{path}");
        assert_eq!(res.header("etag"), None, "{path}");
    }
}

#[corvis::test]
async fn a_value_is_answered_as_that_value() {
    let app = app().await;
    let client = request(&app);
    for path in ["/r/opt-dto-some", "/r/res-opt-dto-some", "/r/opt-value-some"] {
        let res = client.get(path).send().await;
        res.expect_status(200);
        assert_eq!(res.text(), r#"{"name":"Tom"}"#, "{path}");
        assert_eq!(res.header("content-type"), Some("application/json; charset=utf-8"), "{path}");
        assert!(res.header("etag").is_some_and(|e| e.starts_with("W/")), "{path}");
    }
    let res = client.get("/r/res-opt-text-some").send().await;
    assert_eq!(res.text(), "hi");
    assert_eq!(res.header("content-type"), Some("text/html; charset=utf-8"));
}

#[corvis::test]
async fn explicit_json_keeps_a_literal_null() {
    let app = app().await;
    let client = request(&app);
    for path in ["/r/json-none", "/r/response-json-null"] {
        let res = client.get(path).send().await;
        res.expect_status(200);
        assert_eq!(res.text(), "null", "{path}");
        assert_eq!(res.header("content-type"), Some("application/json; charset=utf-8"), "{path}");
        assert_eq!(res.header("content-length"), Some("4"), "{path}");
    }
}

#[corvis::test]
async fn head_on_an_empty_answer_has_no_length_and_no_type() {
    let mut app = app().await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    for path in ["/r/opt-dto-none", "/r/opt-value-none", "/r/void"] {
        let got = raw(addr, "HEAD", path).await;
        assert_eq!(got.status, 200, "{path}");
        assert_eq!(got.body, "", "{path}");
        for name in ["content-type", "content-length", "etag"] {
            assert_eq!(got.header(name), None, "{name} of {path}");
        }
    }
    // A text body that is the empty string keeps its length, as in Express.
    let got = raw(addr, "HEAD", "/r/empty-str").await;
    assert_eq!(got.header("content-length"), Some("0"));
    app.close().await.unwrap();
}

#[corvis::test]
async fn identity_interceptor_keeps_an_empty_answer_empty() {
    let app = app().await;
    let res = request(&app).get("/r/identity").send().await;
    res.expect_status(200);
    assert_eq!(res.text(), "");
    assert_eq!(res.header("content-type"), None);
}

#[corvis::test]
async fn envelope_interceptor_sees_null_for_an_empty_answer() {
    let app = app().await;
    let res = request(&app).get("/r/envelope").send().await;
    assert_eq!(res.text(), r#"{"data":null}"#);
    assert_eq!(res.header("content-type"), Some("application/json; charset=utf-8"));
}

#[corvis::test]
async fn interceptor_that_maps_to_null_answers_an_empty_body() {
    let app = app().await;
    let res = request(&app).get("/r/to-null").send().await;
    res.expect_status(200);
    assert_eq!(res.text(), "");
    assert_eq!(res.header("content-type"), None);
}

#[test]
fn map_data_stores_null_as_an_empty_body() {
    let r = Response::json(json!({ "a": 1 })).map_data(|_| Value::Null);
    assert!(matches!(r.body(), Body::Empty));
    let r = Response::new().map_data(|d| d);
    assert!(matches!(r.body(), Body::Empty));
    let r = Response::json(Value::Null).map_data(|d| d);
    assert!(matches!(r.body(), Body::Empty));
}

#[corvis::test]
async fn serializer_validates_an_empty_result_as_null() {
    let app = app().await;
    let client = request(&app);
    // The strict schema rejects `null`, for `Option<Dto>` and for `Option<Value>` alike.
    client.get("/r/strict").send().await.expect_status(500);
    client.get("/r/strict-unit").send().await.expect_status(500);
    let res = client.get("/r/nullable").send().await;
    res.expect_status(200);
    assert_eq!(res.text(), "");
    assert_eq!(res.header("content-type"), None);
}

#[corvis::test]
async fn cache_replays_a_stored_null_as_an_empty_body() {
    let app = app().await;
    let cache = app.get::<CacheManager>().unwrap();
    cache.set_value("/r/cached", Value::Null, None);
    let res = request(&app).get("/r/cached").send().await;
    res.expect_status(200);
    assert_eq!(res.text(), "");
    assert_eq!(res.header("content-type"), None);
}

#[corvis::test]
async fn an_empty_answer_is_not_cached() {
    let app = app().await;
    let cache = app.get::<CacheManager>().unwrap();
    request(&app).get("/r/cached").send().await.expect_status(200);
    assert!(cache.get_value("/r/cached").is_none());
}
