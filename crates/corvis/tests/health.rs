use std::time::{Duration, Instant};

use corvis::{HealthCheckService, HealthIndicator, HttpException, HttpHealthIndicator, MemoryHealthIndicator};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn run(ind: HealthIndicator) -> Result<Value, HttpException> {
    HealthCheckService.check(vec![ind]).await
}

/// The `error` or `details` entry of the single indicator `k`.
fn entry(res: Result<Value, HttpException>, k: &str) -> (bool, Value) {
    match res {
        Ok(body) => (true, body["details"][k].clone()),
        Err(e) => {
            assert_eq!(e.status().as_u16(), 503);
            (false, e.get_response()["details"][k].clone())
        }
    }
}

/// Serves `respond(request_path)` as a raw HTTP/1.1 response (status line + headers) to every connection.
async fn server(respond: fn(&str) -> String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let _ = sock.write_all(respond(&path).as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    port
}

fn closed_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn resp(status: &str, extra: &str) -> String {
    format!("HTTP/1.1 {status}\r\n{extra}content-length: 0\r\nconnection: close\r\n\r\n")
}

// 1. time limits

#[tokio::test]
async fn hung_indicator_is_reported_down_without_hiding_the_others() {
    let start = Instant::now();
    let res = HealthCheckService
        .check(vec![
            HealthIndicator::new("stuck", || async {
                std::future::pending::<()>().await;
                Ok(json!({}))
            })
            .timeout(Duration::from_millis(100)),
            HealthIndicator::new("fine", || async { Ok(json!({})) }),
        ])
        .await;
    assert!(start.elapsed() < Duration::from_secs(1));
    let e = res.unwrap_err();
    assert_eq!(e.status().as_u16(), 503);
    let body = e.get_response();
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"]["stuck"], json!({ "status": "down", "message": "timeout of 100ms exceeded" }));
    assert_eq!(body["info"]["fine"], json!({ "status": "up" }));
    assert_eq!(body["details"].as_object().unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn default_limit_is_five_seconds() {
    let started = tokio::time::Instant::now();
    let ind = HealthIndicator::new("stuck", || async {
        std::future::pending::<()>().await;
        Ok(json!({}))
    });
    let (up, e) = entry(run(ind).await, "stuck");
    assert!(!up);
    assert_eq!(e["message"], "timeout of 5000ms exceeded");
    assert_eq!(started.elapsed(), Duration::from_secs(5));
}

#[tokio::test(start_paused = true)]
async fn indicators_run_concurrently() {
    let started = tokio::time::Instant::now();
    let slow = |k: &'static str| {
        HealthIndicator::new(k, || async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(json!({}))
        })
    };
    HealthCheckService.check(vec![slow("a"), slow("b"), slow("c")]).await.unwrap();
    assert_eq!(started.elapsed(), Duration::from_secs(2));
}

// 2. tcp_check

#[tokio::test]
async fn tcp_check_up_when_port_is_open() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let (up, e) = entry(run(HttpHealthIndicator::tcp_check("redis", addr)).await, "redis");
    assert!(up);
    assert_eq!(e, json!({ "status": "up" }));
}

#[tokio::test]
async fn tcp_check_down_when_port_is_closed() {
    let addr = format!("127.0.0.1:{}", closed_port());
    let (up, e) = entry(run(HttpHealthIndicator::tcp_check("redis", addr)).await, "redis");
    assert!(!up);
    assert_eq!(e["status"], "down");
    assert!(e["message"].is_string());
}

// 3. ping_check

#[tokio::test]
async fn ping_check_up_on_200() {
    let port = server(|_| resp("200 OK", "")).await;
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", format!("http://127.0.0.1:{port}/"))).await, "api");
    assert!(up);
    assert_eq!(e, json!({ "status": "up" }));
}

#[tokio::test]
async fn ping_check_down_on_500_with_status_details() {
    let port = server(|_| resp("500 Internal Server Error", "")).await;
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", format!("http://127.0.0.1:{port}/"))).await, "api");
    assert!(!up);
    assert_eq!(e["status"], "down");
    assert_eq!(e["statusCode"], 500);
    assert_eq!(e["statusText"], "Internal Server Error");
    assert_eq!(e["message"], "Request failed with status code 500");
}

#[tokio::test]
async fn ping_check_down_when_nothing_listens() {
    let url = format!("http://127.0.0.1:{}/", closed_port());
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", url)).await, "api");
    assert!(!up);
    assert!(e["message"].is_string());
    assert!(e.get("statusCode").is_none());
}

#[tokio::test]
async fn ping_check_rejects_host_port_and_names_tcp_check() {
    for addr in ["127.0.0.1:6379", "localhost:6379", "just-a-host", "ftp://127.0.0.1/"] {
        let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", addr)).await, "api");
        assert!(!up, "{addr}");
        assert!(e["message"].as_str().unwrap().contains("tcp_check"), "{addr}: {e}");
        assert!(e.get("statusCode").is_none());
    }
}

#[tokio::test]
async fn ping_check_follows_redirects() {
    let port = server(|path| match path {
        "/start" => resp("302 Found", "location: /moved\r\n"),
        "/moved" => resp("301 Moved Permanently", "location: /ok\r\n"),
        "/ok" => resp("200 OK", ""),
        _ => resp("404 Not Found", ""),
    })
    .await;
    let (up, _) = entry(run(HttpHealthIndicator::ping_check("api", format!("http://127.0.0.1:{port}/start"))).await, "api");
    assert!(up);
}

#[tokio::test]
async fn ping_check_stops_after_five_redirects() {
    let port = server(|_| resp("302 Found", "location: /again\r\n")).await;
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", format!("http://127.0.0.1:{port}/"))).await, "api");
    assert!(!up);
    assert!(e["message"].as_str().unwrap().contains("redirects"));
}

#[tokio::test]
async fn ping_check_reports_redirect_to_failing_target() {
    let port = server(|path| match path {
        "/" => resp("302 Found", "location: /gone\r\n"),
        _ => resp("404 Not Found", ""),
    })
    .await;
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", format!("http://127.0.0.1:{port}/"))).await, "api");
    assert!(!up);
    assert_eq!(e["statusCode"], 404);
    assert_eq!(e["statusText"], "Not Found");
}

#[cfg(not(feature = "health-tls"))]
#[tokio::test]
async fn ping_check_https_without_tls_feature_is_down_and_sends_nothing() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = [0u8; 16];
        // the check closes the socket without writing anything
        s.read(&mut buf).await.unwrap()
    });
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", format!("https://127.0.0.1:{port}/"))).await, "api");
    assert!(!up);
    assert!(e["message"].as_str().unwrap().contains("health-tls"));
    assert_eq!(seen.await.unwrap(), 0);
}

#[cfg(feature = "health-tls")]
#[tokio::test]
async fn ping_check_https_with_tls_feature_fails_the_handshake_against_plain_server() {
    let port = server(|_| resp("200 OK", "")).await;
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", format!("https://127.0.0.1:{port}/"))).await, "api");
    assert!(!up);
    assert!(e["message"].is_string());
}

// 4. check_rss

#[tokio::test]
async fn check_rss_up_under_threshold() {
    let (up, e) = entry(run(MemoryHealthIndicator::check_rss("memory_rss", u64::MAX)).await, "memory_rss");
    assert!(up);
    assert_eq!(e, json!({ "status": "up" }));
}

#[tokio::test]
async fn check_rss_down_over_threshold() {
    let (up, e) = entry(run(MemoryHealthIndicator::check_rss("memory_rss", 1)).await, "memory_rss");
    assert!(!up);
    assert_eq!(e, json!({ "status": "down", "message": "Used rss exceeded the set threshold" }));
}

// 5. response format

#[tokio::test]
async fn mixed_results_use_the_documented_format() {
    let res = HealthCheckService
        .check(vec![
            HealthIndicator::new("a", || async { Ok(json!({})) }),
            HealthIndicator::new("b", || async { Err(json!({ "message": "down" })) }),
        ])
        .await;
    let body = res.unwrap_err().get_response().clone();
    assert_eq!(body["info"], json!({ "a": { "status": "up" } }));
    assert_eq!(body["error"], json!({ "b": { "status": "down", "message": "down" } }));
    assert_eq!(body["details"].as_object().unwrap().len(), 2);
}
