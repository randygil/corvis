use std::time::{Duration, Instant};

use corvis::{HealthCheckService, HealthIndicator, HttpException, HttpHealthIndicator, MemoryHealthIndicator, PingOptions};
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
async fn server(respond: impl Fn(&str) -> String + Send + Sync + 'static) -> u16 {
    let respond = std::sync::Arc::new(respond);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let respond = respond.clone();
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

// 6. security of the HTTP ping

#[tokio::test]
async fn invalid_url_message_never_echoes_the_configured_url() {
    let url = "redis://:s3cr3t-pass@redis.internal:6379";
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("redis", url)).await, "redis");
    assert!(!up);
    let text = e.to_string();
    assert!(!text.contains("s3cr3t-pass") && !text.contains("redis.internal"), "{text}");
    assert!(text.contains("tcp_check"));
    let (_, e) = entry(run(HttpHealthIndicator::ping_check("redis", "http://user:s3cr3t-pass@")).await, "redis");
    assert!(!e.to_string().contains("s3cr3t-pass"));
}

#[tokio::test]
async fn redirect_location_is_never_copied_into_the_response() {
    let long = "a".repeat(2000);
    let big = long.clone();
    let port = server(move |p| match p {
        "/" => resp("302 Found", &format!("location: /{big}\r\n")),
        _ => resp("404 Not Found", ""),
    })
    .await;
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", format!("http://127.0.0.1:{port}/"))).await, "api");
    assert!(!up);
    assert!(e.to_string().len() < 1024, "{}", e.to_string().len());
    assert!(!e.to_string().contains(&long[..64]));

    let port = server(|_| resp("302 Found", "location: http://user:s3cr3t-pass@elsewhere.example/\r\n")).await;
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", format!("http://127.0.0.1:{port}/"))).await, "api");
    assert!(!up);
    assert!(!e.to_string().contains("s3cr3t-pass") && !e.to_string().contains("elsewhere"));
    assert_eq!(e["statusCode"], 302);
}

#[tokio::test]
async fn redirect_to_another_origin_is_not_followed_by_default() {
    let target = server(|_| resp("200 OK", "")).await;
    let port = server(move |_| resp("302 Found", &format!("location: http://127.0.0.1:{target}/\r\n"))).await;
    let url = format!("http://127.0.0.1:{port}/");
    let (up, e) = entry(run(HttpHealthIndicator::ping_check("api", url.clone())).await, "api");
    assert!(!up);
    assert_eq!(e["statusCode"], 302);
    assert!(e["message"].as_str().unwrap().contains("another origin"));

    // opted in: followed (the configured URL is itself on loopback, so the internal-address rule does not apply)
    let ind = HttpHealthIndicator::ping_check_with("api", url, PingOptions::new().follow_cross_origin(true));
    let (up, _) = entry(run(ind).await, "api");
    assert!(up);
}

#[tokio::test]
async fn max_redirects_limits_and_zero_disables_following() {
    let port = server(|p| match p {
        "/" => resp("302 Found", "location: /a\r\n"),
        "/a" => resp("302 Found", "location: /b\r\n"),
        "/b" => resp("200 OK", ""),
        _ => resp("404 Not Found", ""),
    })
    .await;
    let url = format!("http://127.0.0.1:{port}/");
    let with = |n| HttpHealthIndicator::ping_check_with("api", url.clone(), PingOptions::new().max_redirects(n));
    let (up, _) = entry(run(with(2)).await, "api");
    assert!(up);
    let (up, e) = entry(run(with(1)).await, "api");
    assert!(!up);
    assert!(e["message"].as_str().unwrap().contains("redirects"));
    let (up, e) = entry(run(with(0)).await, "api");
    assert!(!up);
    assert_eq!(e["statusCode"], 302);
    assert_eq!(e["statusText"], "Found");
}

#[cfg(feature = "health-tls")]
#[tokio::test]
async fn https_check_never_panics_whatever_the_crypto_provider_setup() {
    // No process-wide provider is installed here; the check builds its own config instead of panicking
    // like `ClientConfig::builder()` does when several providers are compiled in.
    let port = server(|_| resp("200 OK", "")).await;
    let res = run(HttpHealthIndicator::ping_check("api", format!("https://127.0.0.1:{port}/"))).await;
    let (up, e) = entry(res, "api");
    assert!(!up);
    assert!(e["message"].as_str().unwrap().contains("TLS"), "{e}");
}

// 7. differential fixture recorded from the original framework (tests/fixtures/terminus)

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/terminus/terminus.json")).unwrap()
}

/// Compares `actual` with `expected` exactly (keys and values), except that a string expected under `message` that
/// starts with one of `volatile` (OS-dependent text) only has to be present as a string.
fn assert_same(actual: &Value, expected: &Value, volatile: &[&str], at: &str) {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            let mut ak: Vec<_> = a.keys().collect();
            let mut ek: Vec<_> = e.keys().collect();
            ak.sort();
            ek.sort();
            assert_eq!(ak, ek, "keys at {at}");
            for (k, ev) in e {
                if k == "message" && ev.as_str().is_some_and(|m| volatile.iter().any(|v| m.starts_with(v))) {
                    assert!(a[k].is_string(), "message at {at}");
                } else {
                    assert_same(&a[k], ev, volatile, &format!("{at}.{k}"));
                }
            }
        }
        _ => assert_eq!(actual, expected, "value at {at}"),
    }
}

/// Runs `inds` and returns (status code, body) as an HTTP client would see them.
async fn replay(inds: Vec<HealthIndicator>) -> (u64, Value) {
    match HealthCheckService.check(inds).await {
        Ok(body) => (200, body),
        Err(e) => (e.get_status() as u64, e.get_response().clone()),
    }
}

async fn assert_scenario(name: &str, inds: Vec<HealthIndicator>) {
    let fx = fixture();
    let expected = &fx["scenarios"][name];
    assert!(expected.is_object(), "scenario {name} missing from terminus.json");
    let (status, body) = replay(inds).await;
    assert_eq!(status, expected["status"].as_u64().unwrap(), "{name}: status");
    assert_same(&body, &expected["body"], &["connect ECONNREFUSED"], name);
}

#[tokio::test]
async fn fixture_http_scenarios() {
    let ok = server(|p| match p {
        "/ok" => resp("200 OK", ""),
        "/error" => resp("500 Internal Server Error", ""),
        "/start" => resp("302 Found", "location: /ok\r\n"),
        _ => resp("404 Not Found", ""),
    })
    .await;
    let u = |p: &str| format!("http://127.0.0.1:{ok}{p}");
    assert_scenario("ping_200", vec![HttpHealthIndicator::ping_check("api", u("/ok"))]).await;
    assert_scenario("ping_500", vec![HttpHealthIndicator::ping_check("api", u("/error"))]).await;
    assert_scenario("ping_302_then_200", vec![HttpHealthIndicator::ping_check("api", u("/start"))]).await;
    let closed = format!("http://127.0.0.1:{}/", closed_port());
    assert_scenario("ping_closed_port", vec![HttpHealthIndicator::ping_check("api", closed)]).await;
}

#[tokio::test]
async fn fixture_rss_scenarios() {
    assert_scenario("rss_huge_threshold", vec![MemoryHealthIndicator::check_rss("memory_rss", u64::MAX)]).await;
    assert_scenario("rss_tiny_threshold", vec![MemoryHealthIndicator::check_rss("memory_rss", 1)]).await;
}

#[tokio::test]
async fn fixture_mixed_scenario() {
    assert_scenario(
        "mixed",
        vec![
            HealthIndicator::new("db", || async { Ok(json!({})) }),
            HealthIndicator::new("redis", || async { Err(json!({ "message": "down" })) }),
        ],
    )
    .await;
}

/// Intentional divergence: the original answers a URL without a scheme with a bare 500 (axios throws a non-HTTP
/// error); corvis reports the indicator `down` (503) and points to `tcp_check`.
#[tokio::test]
async fn fixture_no_scheme_diverges_on_purpose() {
    let fx = fixture();
    assert_eq!(fx["scenarios"]["ping_no_scheme"]["status"], 500);
    let (status, body) = replay(vec![HttpHealthIndicator::ping_check("api", "127.0.0.1:6379")]).await;
    assert_eq!(status, 503);
    assert_eq!(body["error"]["api"]["status"], "down");
}

/// The original does not catch an indicator that throws: it answers 500 `{ statusCode, message }` without a
/// health body. corvis diverges on purpose: a panicking indicator is reported `down` (503, see the panic tests below).
#[test]
fn fixture_records_the_throwing_indicator() {
    let fx = fixture();
    let s = &fx["scenarios"]["throws_plain_error"];
    assert_eq!(s["status"], 500);
    assert_eq!(s["body"], json!({ "statusCode": 500, "message": "Internal server error" }));
}

// 4. panic isolation

fn panicking_future(key: &str) -> HealthIndicator {
    HealthIndicator::new(key.to_string(), || async {
        if true {
            panic!("connection pool poisoned");
        }
        Ok(json!({}))
    })
}

#[tokio::test]
async fn panicking_indicator_is_reported_down_next_to_the_others() {
    let e = HealthCheckService
        .check(vec![HealthIndicator::new("db", || async { Ok(json!({})) }), panicking_future("cache")])
        .await
        .unwrap_err();
    assert_eq!(e.status().as_u16(), 503);
    let body = e.get_response();
    assert_eq!(body["status"], "error");
    assert_eq!(body["info"]["db"], json!({ "status": "up" }));
    assert_eq!(body["error"]["cache"], json!({ "status": "down", "message": "health indicator panicked" }));
    assert_eq!(body["details"].as_object().unwrap().len(), 2);
}

#[tokio::test]
async fn indicator_that_panics_before_returning_its_future_is_reported_down() {
    let e = HealthCheckService
        .check(vec![
            HealthIndicator::new("cfg", || -> futures::future::Ready<Result<Value, Value>> { panic!("bad config") }),
            HealthIndicator::new("db", || async { Ok(json!({})) }),
        ])
        .await
        .unwrap_err();
    let body = e.get_response();
    assert_eq!(body["error"]["cfg"], json!({ "status": "down", "message": "health indicator panicked" }));
    assert_eq!(body["info"]["db"], json!({ "status": "up" }));
}

#[tokio::test]
async fn a_hang_is_still_a_timeout_and_a_panic_a_panic() {
    let e = HealthCheckService
        .check(vec![
            HealthIndicator::new("stuck", || async {
                std::future::pending::<()>().await;
                Ok(json!({}))
            })
            .timeout(Duration::from_millis(50)),
            panicking_future("boom"),
        ])
        .await
        .unwrap_err();
    let body = e.get_response();
    assert_eq!(body["error"]["stuck"]["message"], "timeout of 50ms exceeded");
    assert_eq!(body["error"]["boom"]["message"], "health indicator panicked");
}

#[tokio::test]
async fn panic_message_stays_out_of_the_response() {
    let e = HealthCheckService
        .check(vec![HealthIndicator::new("db", || async {
            if true {
                panic!("postgres://user:secret@db/app");
            }
            Ok(json!({}))
        })])
        .await
        .unwrap_err();
    let text = e.get_response().to_string();
    assert!(!text.contains("secret"), "{text}");
    assert!(text.contains("health indicator panicked"));
}

mod http_route {
    use corvis::prelude::*;
    use corvis::testing::{Test, request};

    #[controller("health")]
    pub struct HealthController {
        health: Arc<HealthCheckService>,
    }

    #[routes]
    impl HealthController {
        #[get]
        async fn check(&self) -> HttpResult<Value> {
            self.health
                .check(vec![
                    HealthIndicator::new("db", || async { Ok(json!({})) }),
                    HealthIndicator::new("cache", || async {
                        if true {
                            panic!("pool poisoned");
                        }
                        Ok(json!({}))
                    }),
                ])
                .await
        }
    }

    #[module(imports = [TerminusModule], controllers = [HealthController])]
    pub struct AppModule;

    #[corvis::test]
    async fn route_answers_503_with_the_health_body_not_500() {
        let module = Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
        let mut app = module.create_corvis_application();
        app.init().await.unwrap();
        let res = request(&app).get("/health").send().await;
        res.expect_status(503);
        let body = res.json_value();
        assert_eq!(body["status"], "error");
        assert_eq!(body["info"]["db"]["status"], "up");
        assert_eq!(body["error"]["cache"], json!({ "status": "down", "message": "health indicator panicked" }));
        app.close().await.unwrap();
    }
}
