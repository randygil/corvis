//! The test client reads a request the way a listening server does: the body limits, the refusal of a declared length
//! over the limit, and the pending multipart body. (The comparison of whole answers with a listening server is
//! `the_test_client_answers_like_a_listening_server` in `express_parity.rs`.)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use corvis::prelude::*;
use corvis::testing::request;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static ECHOED: AtomicUsize = AtomicUsize::new(0);

#[controller("in")]
pub struct InputController;

#[routes]
impl InputController {
    #[post("json")]
    fn json(&self, #[body] body: Value) -> Value {
        ECHOED.fetch_add(1, Ordering::SeqCst);
        json!({ "keys": body.as_object().map_or(0, |o| o.len()) })
    }

    /// What the request holds of a multipart body, and what reading it gives.
    #[post("upload")]
    async fn upload(&self, #[req] req: Request) -> Result<Value, HttpException> {
        let buffered = req.raw_body().len();
        let read = corvis::multer::read_body(&req, 1024 * 1024).await?;
        Ok(json!({ "buffered": buffered, "read": read.len() }))
    }
}

#[module(controllers = [InputController])]
pub struct InputModule;

async fn app() -> CorvisApplication {
    let mut app = CorvisFactory::create::<InputModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

/// A JSON body of about `bytes` bytes.
fn big_json(bytes: usize) -> String {
    format!(r#"{{"pad":"{}"}}"#, "x".repeat(bytes))
}

#[corvis::test]
async fn a_body_over_the_parser_limit_is_refused_by_the_test_client_as_by_the_server() {
    let mut app = app().await;
    // The JSON parser's default limit is 100 kB.
    let body = big_json(200 * 1024);
    let memory = request(&app).post("/in/json").header("content-type", "application/json").body(body.clone()).send().await;
    assert_eq!(memory.status(), 413, "{}", memory.text());
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "POST /in/json HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tcp.write_all(head.as_bytes()).await.unwrap();
    let _ = tcp.write_all(body.as_bytes()).await;
    let mut wire = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), tcp.read_to_end(&mut wire)).await.unwrap().unwrap();
    let wire = String::from_utf8_lossy(&wire);
    assert!(wire.starts_with("HTTP/1.1 413 "), "{wire}");
    assert!(wire.ends_with(&memory.text()), "{wire}");
    // Under the limit both read it.
    let r = request(&app).post("/in/json").json(&json!({ "a": 1 })).send().await;
    assert_eq!(r.json_value(), json!({ "keys": 1 }));
    app.close().await.unwrap();
}

#[corvis::test]
async fn a_declared_length_over_the_limit_is_refused_without_reading_the_body() {
    let app = app().await;
    let before = ECHOED.load(Ordering::SeqCst);
    let r = request(&app)
        .post("/in/json")
        .header("content-type", "application/json")
        .header("content-length", "10000000")
        .body(r#"{"a":1}"#)
        .send()
        .await;
    assert_eq!(r.status(), 413, "{}", r.text());
    assert_eq!(ECHOED.load(Ordering::SeqCst), before, "the handler did not run");
}

#[corvis::test]
async fn a_multipart_body_is_left_pending_for_the_reader_as_over_a_socket() {
    let app = app().await;
    let r = request(&app).post("/in/upload").field("name", "a").attach("file", vec![7u8; 5000], "a.bin").send().await;
    let v = r.json_value();
    // Nothing of it was buffered into the request: it was read through the pending body.
    assert_eq!(v["buffered"], 0, "{v}");
    assert!(v["read"].as_u64().unwrap() > 5000, "{v}");
}
