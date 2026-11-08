//! HTTP-level parity with the Express platform (express 5 + `cors`): 404 bodies with
//! a global prefix, CORS preflights, ETag / 304, charset handling, `StreamableFile`,
//! raw bodies and `X-Powered-By`. Expected values were captured from a real Express 5 app.

use std::time::Duration;

use corvis::prelude::*;
use corvis::testing::{TestResponse, request};
use corvis::{CorsOrigin, EtagMode, StreamableFileOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[controller("health")]
pub struct HealthController;

#[routes]
impl HealthController {
    #[get]
    fn health(&self) -> Value {
        json!({ "status": "ok" })
    }
}

#[controller("invoices")]
pub struct InvoicesController;

#[routes]
impl InvoicesController {
    #[get]
    fn list(&self) -> Value {
        json!({ "data": [1, 2, 3] })
    }

    #[get(":id")]
    fn one(&self, #[param("id", ParseUUIDPipe)] id: String) -> Value {
        json!({ "id": id })
    }

    #[get("v4/:id")]
    fn one_v4(&self, #[param("id", ParseUUIDPipe::new().version(4))] id: String) -> Value {
        json!({ "id": id })
    }

    /// `res.setHeader(...)` x3 + `res.end(buffer)`.
    #[get(":id/pdf")]
    fn pdf(&self, #[param("id")] _id: String) -> Response {
        let bytes = b"%PDF-1.7 fake".to_vec();
        let len = bytes.len();
        Response::bytes(bytes, "application/pdf")
            .with_header("Content-Disposition", "inline; filename=\"factura-00-00000001.pdf\"")
            .with_header("Content-Length", len.to_string())
    }

    /// TXT in the layout's declared encoding: `text/plain; charset=windows-1252` + `res.end`.
    #[get("export/txt")]
    fn txt(&self) -> Response {
        Response::bytes(vec![0x41, 0xD1, 0x0D, 0x0A], "text/plain; charset=windows-1252")
            .with_header("Content-Disposition", "attachment; filename=\"retenciones.txt\"")
    }

    /// `@Header('Content-Type', 'text/plain')` + `return 'hello'`.
    #[get("plain/header")]
    #[header("Content-Type", "text/plain")]
    fn plain_header(&self) -> String {
        "hello".into()
    }

    /// `res.type('text/csv; charset=latin1'); res.send('a,b')` -> express forces utf-8.
    #[get("plain/csv")]
    fn csv(&self) -> Response {
        Response::text("a,b").with_header("content-type", "text/csv; charset=latin1")
    }

    #[get("file/buffer")]
    fn file_buffer(&self) -> StreamableFile {
        StreamableFile::with_options(
            b"col1,col2".to_vec(),
            StreamableFileOptions::new().r#type("text/csv").disposition("attachment; filename=\"a.csv\""),
        )
    }

    #[get("file/header")]
    #[header("Content-Type", "application/vnd.ms-excel")]
    #[header("Content-Disposition", "attachment; filename=\"override.xls\"")]
    fn file_header(&self) -> StreamableFile {
        StreamableFile::new(b"xls".to_vec()).content_type("text/csv").filename("ignored.csv")
    }

    #[get("file/stream")]
    fn file_stream(&self) -> StreamableFile {
        let chunks =
            futures::stream::iter(vec![Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"ab")), Ok(bytes::Bytes::from_static(b"cd"))]);
        StreamableFile::from_stream(Box::pin(chunks), StreamableFileOptions::new().length(4))
    }

    #[get("empty/text")]
    fn empty_text(&self) -> String {
        String::new()
    }
}

#[module(controllers = [HealthController, InvoicesController])]
pub struct AppModule;

fn facten_cors() -> CorsOptions {
    CorsOptions::new()
        .origins(["http://localhost:3000"])
        .methods(["GET", "HEAD", "POST", "PATCH", "PUT", "DELETE", "OPTIONS"])
        .allowed_headers(["Content-Type", "Authorization", "Idempotency-Key"])
        .exposed_headers(["Content-Disposition"])
        .credentials(true)
        .max_age(86_400)
}

async fn app_with(f: impl FnOnce(&mut CorvisApplication)) -> CorvisApplication {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.set_global_prefix_with_exclude("api/v1", &["health"]);
    app.enable_cors(facten_cors());
    app.silence_route_logs();
    f(&mut app);
    app.init().await.unwrap();
    app
}

fn h<'a>(r: &'a TestResponse, name: &str) -> Option<&'a str> {
    r.header(name)
}

#[corvis::test]
async fn not_found_bodies_and_etags_match_express() {
    let app = app_with(|_| {}).await;
    let c = request(&app);
    let cases = [
        ("GET", "/x", r#"{"message":"Cannot GET /x","error":"Not Found","statusCode":404}"#, r#"W/"40-Lts3F9bcD4+FSck1x7cIWF0Qf44""#),
        (
            "GET",
            "/api/v1/x",
            r#"{"message":"Cannot GET /api/v1/x","error":"Not Found","statusCode":404}"#,
            r#"W/"47-ycl2rmo1sCY1CWn0d//w5zHif+8""#,
        ),
        (
            "POST",
            "/health",
            r#"{"message":"Cannot POST /health","error":"Not Found","statusCode":404}"#,
            r#"W/"46-PNJ2WZUyGZyD7vY7oJLlfd1w6B4""#,
        ),
        (
            "GET",
            "/api/v1/x?a=1",
            r#"{"message":"Cannot GET /api/v1/x?a=1","error":"Not Found","statusCode":404}"#,
            r#"W/"4b-ldwy9WRYUkzaIPoNFsNPN+hKdEM""#,
        ),
    ];
    for (method, path, body, etag) in cases {
        let r = c.method(method.parse().unwrap(), path).send().await;
        assert_eq!(r.status(), 404, "{path}");
        assert_eq!(r.text(), body);
        assert_eq!(h(&r, "content-type"), Some("application/json; charset=utf-8"));
        assert_eq!(h(&r, "content-length"), Some(body.len().to_string().as_str()));
        assert_eq!(h(&r, "etag"), Some(etag));
        // No Origin header: express' cors still sends Vary/credentials/expose, no ACAO.
        assert_eq!(h(&r, "vary"), Some("Origin"));
        assert_eq!(h(&r, "access-control-allow-credentials"), Some("true"));
        assert_eq!(h(&r, "access-control-expose-headers"), Some("Content-Disposition"));
        assert_eq!(h(&r, "access-control-allow-origin"), None);
        assert_eq!(h(&r, "x-powered-by"), Some("corvis"));
    }
    c.get("/health").send().await.expect_status(200);
    c.get("/api/v1/health").send().await.expect_status(404);
}

#[corvis::test]
async fn cors_preflight_and_simple_requests_match_the_cors_package() {
    let app = app_with(|_| {}).await;
    let c = request(&app);
    let check_preflight = |r: &TestResponse, acao: Option<&str>| {
        assert_eq!(r.status(), 204);
        assert_eq!(h(r, "access-control-allow-origin"), acao);
        assert_eq!(h(r, "vary"), Some("Origin"));
        assert_eq!(h(r, "access-control-allow-credentials"), Some("true"));
        assert_eq!(h(r, "access-control-allow-methods"), Some("GET,HEAD,POST,PATCH,PUT,DELETE,OPTIONS"));
        assert_eq!(h(r, "access-control-allow-headers"), Some("Content-Type,Authorization,Idempotency-Key"));
        assert_eq!(h(r, "access-control-max-age"), Some("86400"));
        assert_eq!(h(r, "access-control-expose-headers"), Some("Content-Disposition"));
        assert_eq!(h(r, "content-length"), Some("0"));
        assert_eq!(h(r, "content-type"), None);
        assert_eq!(h(r, "etag"), None);
        assert!(r.body.is_empty());
    };
    let r = c
        .options("/api/v1/invoices")
        .header("origin", "http://localhost:3000")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type,authorization")
        .send()
        .await;
    check_preflight(&r, Some("http://localhost:3000"));
    let r = c.options("/api/v1/invoices").header("origin", "http://evil.com").header("access-control-request-method", "POST").send().await;
    check_preflight(&r, None);
    // Every OPTIONS is a preflight for `cors`, even without Access-Control-Request-Method,
    // and even on unknown routes.
    let r = c.options("/api/v1/invoices").header("origin", "http://localhost:3000").send().await;
    check_preflight(&r, Some("http://localhost:3000"));
    let r = c.options("/nope").send().await;
    check_preflight(&r, None);

    // Simple requests.
    let r = c.get("/api/v1/invoices").header("origin", "http://localhost:3000").send().await;
    assert_eq!(r.status(), 200);
    assert_eq!(h(&r, "access-control-allow-origin"), Some("http://localhost:3000"));
    assert_eq!(h(&r, "vary"), Some("Origin"));
    assert_eq!(h(&r, "access-control-allow-credentials"), Some("true"));
    assert_eq!(h(&r, "access-control-expose-headers"), Some("Content-Disposition"));
    assert_eq!(h(&r, "access-control-allow-methods"), None);
    let r = c.get("/api/v1/invoices").header("origin", "http://evil.com").send().await;
    assert_eq!(h(&r, "access-control-allow-origin"), None);
    assert_eq!(h(&r, "vary"), Some("Origin"));
}

#[corvis::test]
async fn cors_origin_variants() {
    // Default `enableCors()`: `*`, no Vary, requested headers reflected.
    let app = app_with(|a| {
        a.enable_cors(CorsOptions::default());
    })
    .await;
    let c = request(&app);
    let r = c.options("/x").header("origin", "http://a.com").header("access-control-request-headers", "x-a").send().await;
    assert_eq!(h(&r, "access-control-allow-origin"), Some("*"));
    assert_eq!(h(&r, "vary"), Some("Access-Control-Request-Headers"));
    assert_eq!(h(&r, "access-control-allow-headers"), Some("x-a"));
    assert_eq!(h(&r, "access-control-allow-methods"), Some("GET,HEAD,PUT,PATCH,POST,DELETE"));
    let r = c.get("/health").header("origin", "http://a.com").send().await;
    assert_eq!(h(&r, "access-control-allow-origin"), Some("*"));
    assert_eq!(h(&r, "vary"), None);

    // Fixed string: always sent, with Vary.
    let app = app_with(|a| {
        a.enable_cors(CorsOptions::new().origin("http://localhost:3000").credentials(true));
    })
    .await;
    let r = request(&app).get("/health").header("origin", "http://other").send().await;
    assert_eq!(h(&r, "access-control-allow-origin"), Some("http://localhost:3000"));
    assert_eq!(h(&r, "vary"), Some("Origin"));

    // `origin: true` reflects; regexes; functions; `false` disables CORS.
    let app = app_with(|a| {
        a.enable_cors(CorsOptions::new().reflect_origin());
    })
    .await;
    let r = request(&app).get("/health").header("origin", "http://z.io").send().await;
    assert_eq!(h(&r, "access-control-allow-origin"), Some("http://z.io"));
    let app = app_with(|a| {
        a.enable_cors(CorsOptions::new().origin_regex(regex::Regex::new(r"\.example\.com$").unwrap()));
    })
    .await;
    let c = request(&app);
    assert_eq!(
        h(&c.get("/health").header("origin", "https://a.example.com").send().await, "access-control-allow-origin"),
        Some("https://a.example.com")
    );
    assert_eq!(h(&c.get("/health").header("origin", "https://example.org").send().await, "access-control-allow-origin"), None);
    let app = app_with(|a| {
        a.enable_cors(CorsOptions::new().origin_fn(|o| if o == Some("http://ok") { CorsOrigin::Reflect } else { CorsOrigin::Disabled }));
    })
    .await;
    let c = request(&app);
    assert_eq!(h(&c.get("/health").header("origin", "http://ok").send().await, "access-control-allow-origin"), Some("http://ok"));
    let r = c.options("/health").header("origin", "http://ko").send().await;
    assert_eq!(r.status(), 404, "CORS skipped: OPTIONS reaches the router");
    assert_eq!(h(&r, "vary"), None);
}

#[corvis::test]
async fn parse_uuid_pipe_messages() {
    let app = app_with(|_| {}).await;
    let c = request(&app);
    let r = c.get("/api/v1/invoices/not-a-uuid").send().await;
    assert_eq!(r.status(), 400);
    assert_eq!(r.text(), r#"{"message":"Validation failed (uuid is expected)","error":"Bad Request","statusCode":400}"#);
    assert_eq!(h(&r, "etag"), Some(r#"W/"59-kArN5xCcYXjUJs5yv8pQBq+GJsU""#));
    c.get("/api/v1/invoices/315e0bac-eef7-4870-9843-2ea05d802808").send().await.expect_status(200);
    c.get("/api/v1/invoices/v4/315e0bac-eef7-4870-9843-2ea05d802808").send().await.expect_status(200);
    c.get("/api/v1/invoices/v4/315e0bac-eef7-1870-9843-2ea05d802808")
        .send()
        .await
        .expect_status(400)
        .expect_json(json!({ "message": "Validation failed (uuid v 4 is expected)", "error": "Bad Request", "statusCode": 400 }));
}

#[corvis::test]
async fn etag_freshness_head_and_charset() {
    let app = app_with(|_| {}).await;
    let c = request(&app);
    let r = c.get("/api/v1/invoices").send().await;
    let etag = h(&r, "etag").unwrap().to_string();
    assert!(etag.starts_with("W/\""));
    let r304 = c.get("/api/v1/invoices").header("if-none-match", &etag).send().await;
    assert_eq!(r304.status(), 304);
    assert_eq!(h(&r304, "etag"), Some(etag.as_str()));
    assert_eq!(h(&r304, "content-type"), None);
    assert_eq!(h(&r304, "content-length"), None);
    assert!(r304.body.is_empty());
    // `Cache-Control: no-cache` forces a full response; other methods are never fresh.
    let r = c.get("/api/v1/invoices").header("if-none-match", &etag).header("cache-control", "no-cache").send().await;
    assert_eq!(r.status(), 200);
    // HEAD keeps the headers (and length) of the GET without a body.
    let head = c.head("/api/v1/invoices").send().await;
    assert_eq!(head.status(), 200);
    assert_eq!(h(&head, "etag"), Some(etag.as_str()));
    assert_eq!(h(&head, "content-length"), Some("16"));
    assert!(head.body.is_empty());

    let r = c.get("/api/v1/invoices/plain/header").send().await;
    assert_eq!(h(&r, "content-type"), Some("text/plain; charset=utf-8"));
    assert_eq!(r.text(), "hello");
    let r = c.get("/api/v1/invoices/plain/csv").send().await;
    assert_eq!(h(&r, "content-type"), Some("text/csv; charset=utf-8"));
    let r = c.get("/api/v1/invoices/empty/text").send().await;
    assert_eq!(h(&r, "etag"), Some(r#"W/"0-2jmj7l5rSw0yVb/vlWAYkK/YBwk""#));
    assert_eq!(h(&r, "content-type"), Some("text/html; charset=utf-8"));

    // `app.set('etag', ...)`, `app.disable('x-powered-by')`.
    let app = app_with(|a| {
        a.set_etag(EtagMode::Strong).set_x_powered_by("Express");
    })
    .await;
    let r = request(&app).get("/x").send().await;
    assert_eq!(h(&r, "etag"), Some(r#""40-Lts3F9bcD4+FSck1x7cIWF0Qf44""#));
    assert_eq!(h(&r, "x-powered-by"), Some("Express"));
    let app = app_with(|a| {
        a.disable("etag").disable("x-powered-by");
    })
    .await;
    let r = request(&app).get("/x").send().await;
    assert_eq!(h(&r, "etag"), None);
    assert_eq!(h(&r, "x-powered-by"), None);
}

#[corvis::test]
async fn raw_bodies_and_streamable_file() {
    let app = app_with(|_| {}).await;
    let c = request(&app);
    let r = c.get("/api/v1/invoices/315e0bac-eef7-4870-9843-2ea05d802808/pdf").header("origin", "http://localhost:3000").send().await;
    assert_eq!(r.status(), 200);
    assert_eq!(h(&r, "content-type"), Some("application/pdf"));
    assert_eq!(h(&r, "content-disposition"), Some("inline; filename=\"factura-00-00000001.pdf\""));
    assert_eq!(h(&r, "content-length"), Some("13"));
    assert_eq!(h(&r, "etag"), None, "res.end() does not add an ETag");
    assert_eq!(h(&r, "access-control-allow-origin"), Some("http://localhost:3000"));

    let r = c.get("/api/v1/invoices/export/txt").send().await;
    assert_eq!(h(&r, "content-type"), Some("text/plain; charset=windows-1252"));
    assert_eq!(r.body.as_ref(), &[0x41, 0xD1, 0x0D, 0x0A]);
    assert_eq!(h(&r, "etag"), None);

    let r = c.get("/api/v1/invoices/file/buffer").send().await;
    assert_eq!(h(&r, "content-type"), Some("text/csv"), "StreamableFile uses setHeader: no charset added");
    assert_eq!(h(&r, "content-disposition"), Some("attachment; filename=\"a.csv\""));
    assert_eq!(h(&r, "content-length"), Some("9"));
    assert_eq!(h(&r, "etag"), None);

    // `@Header()` wins over the StreamableFile options.
    let r = c.get("/api/v1/invoices/file/header").send().await;
    assert_eq!(h(&r, "content-type"), Some("application/vnd.ms-excel"));
    assert_eq!(h(&r, "content-disposition"), Some("attachment; filename=\"override.xls\""));

    let r = c.get("/api/v1/invoices/file/stream").send().await;
    assert_eq!(h(&r, "content-type"), Some("application/octet-stream"));
    assert_eq!(h(&r, "content-length"), Some("4"));
    assert_eq!(r.text(), "abcd");
}

async fn raw(addr: std::net::SocketAddr, req: &str) -> String {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await.unwrap().unwrap();
    String::from_utf8_lossy(&out).to_ascii_lowercase()
}

#[corvis::test]
async fn wire_level_headers() {
    let mut app = app_with(|a| {
        a.set_x_powered_by("Express");
    })
    .await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let r = raw(addr, "OPTIONS /api/v1/invoices HTTP/1.1\r\nHost: x\r\nOrigin: http://localhost:3000\r\nConnection: close\r\n\r\n").await;
    assert!(r.starts_with("http/1.1 204 no content\r\n"), "{r}");
    // Express also writes `Content-Length: 0` on the 204; hyper never sends Content-Length
    // on a 204 (RFC 9110 §8.6), so it is only visible in the in-memory test client.
    assert!(!r.contains("\r\ncontent-type:"), "{r}");
    assert!(r.contains("\r\nx-powered-by: express\r\n"), "{r}");
    let r = raw(addr, "GET /api/v1/invoices/a/pdf HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(r.contains("\r\ncontent-length: 13\r\n") && r.contains("\r\ncontent-type: application/pdf\r\n"), "{r}");
    assert!(!r.contains("etag"), "{r}");
    let r = raw(addr, "HEAD /api/v1/invoices HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(r.contains("\r\ncontent-length: 16\r\n") && r.ends_with("\r\n\r\n"), "{r}");
    let r = raw(addr, "GET /api/v1/invoices/file/stream HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await;
    assert!(r.contains("\r\ncontent-length: 4\r\n") && r.ends_with("abcd"), "{r}");
    app.close().await.unwrap();
}

#[controller("echo")]
pub struct EchoController;

#[routes]
impl EchoController {
    #[post]
    fn echo(&self, #[body] body: Value) -> Value {
        json!({ "body": body })
    }

    #[get("Item/:code")]
    fn item(&self, #[param("code")] code: String) -> Value {
        json!({ "code": code })
    }
}

#[module(controllers = [EchoController])]
pub struct EchoModule;

/// body-parser 2 (`express.json()` + `express.urlencoded({ extended: true })`) and the router of
/// express 5 (case-insensitive, one optional trailing slash, `decodeURIComponent` params).
#[corvis::test]
async fn body_parser_and_router_match_express() {
    let mut app = CorvisFactory::create::<EchoModule>().await.unwrap();
    app.enable_cors(facten_cors());
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);
    let origin = "http://localhost:3000";

    let r = c.post("/echo").set("content-type", "application/json").set("origin", origin).body("{\"a\":1,}").send().await;
    assert_eq!(r.status, 400);
    assert_eq!(
        r.json_value(),
        json!({ "message": "Expected double-quoted property name in JSON at position 7 (line 1 column 8)", "error": "Bad Request", "statusCode": 400 })
    );
    // The error passes through `cors` first, like the Express middleware stack.
    assert_eq!(h(&r, "access-control-allow-origin"), Some(origin));

    let r = c.post("/echo").set("content-type", "application/json").body("\"x\"").send().await;
    assert_eq!(r.json_value()["message"], "Unexpected token '\"', \"\"x\"\" is not valid JSON");

    let r = c.post("/echo").set("content-type", "application/json").set("content-length", "0").send().await;
    assert_eq!(r.json_value(), json!({ "body": {} }));

    let r = c.post("/echo").set("content-type", "text/plain").body("hola").send().await;
    assert_eq!(r.json_value(), json!({ "body": null }));

    let r = c.post("/echo").set("content-type", "application/x-www-form-urlencoded").body("a[b]=1&a[c][]=2&n=x+y").send().await;
    assert_eq!(r.json_value(), json!({ "body": { "a": { "b": "1", "c": ["2"] }, "n": "x y" } }));

    let big = format!("{{\"a\":\"{}\"}}", "x".repeat(110 * 1024));
    let r = c.post("/echo").set("content-type", "application/json").body(big.clone()).send().await;
    assert_eq!(r.status, 413);
    assert_eq!(r.json_value(), json!({ "statusCode": 413, "message": "request entity too large" }));

    let r = c.post("/echo").set("content-type", "application/json; charset=latin1").body("{}").send().await;
    assert_eq!(r.json_value(), json!({ "statusCode": 415, "message": "unsupported charset \"LATIN1\"" }));

    // Routing.
    let r = c.get("/ECHO/item/AbC").send().await;
    assert_eq!(r.json_value(), json!({ "code": "AbC" }));
    let r = c.get("/echo/item/a%20b/").send().await;
    assert_eq!(r.json_value(), json!({ "code": "a b" }));
    let r = c.get("/echo/item/x//").send().await;
    assert_eq!(r.status, 404);
    let r = c.get("/echo/item/%E0%A4%A").send().await;
    assert_eq!(r.json_value(), json!({ "message": "Failed to decode param '%E0%A4%A'", "error": "Bad Request", "statusCode": 400 }));

    // `app.useBodyParser('json', { limit: '1mb' })`.
    let mut app = CorvisFactory::create::<EchoModule>().await.unwrap();
    app.use_body_parser("json", 1024 * 1024);
    app.silence_route_logs();
    app.init().await.unwrap();
    let r = request(&app).post("/echo").set("content-type", "application/json").body(big).send().await;
    assert_eq!(r.status, 201);
}

#[corvis::test]
async fn query_parsers_match_express() {
    #[controller("q")]
    pub struct QController;
    #[routes]
    impl QController {
        #[get]
        fn q(&self, #[query] q: Value) -> Value {
            q
        }
    }
    #[module(controllers = [QController])]
    pub struct QModule;

    let mut app = CorvisFactory::create::<QModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let r = request(&app).get("/q?b=1&a[]=x&a[]=y&2=z&b=2&c%5Bd%5D=e").send().await;
    assert_eq!(r.text(), r#"{"2":"z","b":["1","2"],"a[]":["x","y"],"c[d]":"e"}"#);

    let mut app = CorvisFactory::create::<QModule>().await.unwrap();
    app.set_query_parser(corvis::http::body_parser::QueryParser::Extended);
    app.silence_route_logs();
    app.init().await.unwrap();
    let r = request(&app).get("/q?a[]=x&a[]=y&c[d][e]=f&g[h][i][j][k][l][m]=n").send().await;
    assert_eq!(
        r.json_value(),
        json!({ "a": ["x", "y"], "c": { "d": { "e": "f" } }, "g": { "h": { "i": { "j": { "k": { "l": { "[m]": "n" } } } } } } })
    );
}

/// Parameters are resolved like `Promise.all`: the earliest failing pipe wins, and on a
/// tie the parameter declared last.
#[corvis::test]
async fn parameter_pipe_errors_follow_promise_all_order() {
    #[injectable]
    pub struct RejectBody;
    #[async_trait]
    impl PipeTransform for RejectBody {
        async fn transform(&self, value: Value, meta: &ArgumentMetadata) -> Result<Value, HttpException> {
            if meta.kind == corvis::ParamKind::Body {
                return Err(BadRequestException::new("body rejected by the global pipe"));
            }
            Ok(value)
        }
    }

    #[controller("pipes")]
    pub struct PipesController;
    #[routes]
    impl PipesController {
        #[put(":a")]
        fn put(&self, #[param("a", ParseIntPipe)] a: i64, #[body] _b: Value) -> Value {
            json!({ "a": a })
        }
        #[get(":a")]
        fn get(&self, #[param("a", ParseIntPipe)] a: i64, #[query("b", ParseBoolPipe)] b: bool) -> Value {
            json!({ "a": a, "b": b })
        }
    }
    #[module(controllers = [PipesController])]
    pub struct PipesModule;

    let mut app = CorvisFactory::create::<PipesModule>().await.unwrap();
    app.use_global_pipes(RejectBody);
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);
    // The body fails in the global pipe (step 0) before the param's ParseIntPipe (step 1).
    let r = c.put("/pipes/x").json(&json!({})).send().await;
    assert_eq!(r.json_value()["message"], "body rejected by the global pipe");
    // Both fail at step 1: the last parameter wins.
    let r = c.get("/pipes/x?b=y").send().await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json_value()["message"], "Validation failed (boolean string is expected)");
    let r = c.get("/pipes/1?b=true").send().await;
    assert_eq!(r.json_value(), json!({ "a": 1, "b": true }));
}
