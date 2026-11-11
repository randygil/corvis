#![allow(dead_code)]

use auth_app::Settings;
use corvis::prelude::*;
use corvis::testing::{TestClient, TestResponse, request};
use corvis::tokio::io::{AsyncReadExt, AsyncWriteExt};
use corvis::tokio::net::TcpStream;

pub const UNAUTHORIZED: &str = r#"{"message":"Unauthorized","statusCode":401}"#;

/// An application with development settings: generated secrets and its own mock provider on a free loopback port.
pub struct Harness {
    pub app: CorvisApplication,
    pub settings: Settings,
}

impl Harness {
    pub async fn start() -> Harness {
        let settings = Settings::dev(3000).await.unwrap();
        Harness::with(settings).await
    }

    pub async fn with(settings: Settings) -> Harness {
        let mut app = auth_app::create_app(settings.clone()).await.unwrap();
        app.silence_route_logs();
        app.init().await.unwrap();
        Harness { app, settings }
    }

    pub fn client(&self) -> TestClient {
        request(&self.app)
    }

    pub fn users(&self) -> std::sync::Arc<auth_app::users::UsersService> {
        self.app.get::<auth_app::users::UsersService>().unwrap()
    }
}

/// `name=value` of the first `Set-Cookie` of a response.
pub fn cookie_of(response: &TestResponse) -> String {
    let header = response.headers.get("set-cookie").expect("a Set-Cookie header").to_str().unwrap();
    header.split(';').next().unwrap().to_string()
}

pub fn set_cookie(response: &TestResponse) -> String {
    response.headers.get("set-cookie").expect("a Set-Cookie header").to_str().unwrap().to_string()
}

pub async fn token_of(c: &TestClient, username: &str, password: &str) -> String {
    let login = c.post("/auth/login").json(&json!({ "username": username, "password": password })).send().await;
    assert_eq!(login.status, 200, "{}", login.text());
    login.json_value()["access_token"].as_str().expect("an access token").to_string()
}

pub async fn session_of(c: &TestClient, username: &str, password: &str) -> String {
    let login = c.post("/session/login").json(&json!({ "username": username, "password": password })).send().await;
    assert_eq!(login.status, 200, "{}", login.text());
    cookie_of(&login)
}

/// A plain HTTP/1.1 GET to the mock provider (or any loopback server): status, `Location` and body.
pub async fn http_get(addr: std::net::SocketAddr, target: &str) -> (u16, Option<String>, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap();
    let location = head.lines().find_map(|l| l.strip_prefix("Location: ")).map(str::to_string);
    (status, location, body.to_string())
}

/// What the browser does after `GET /auth/provider`: follows the redirect to the provider, which approves, and returns
/// the path and query of the callback on the application.
pub async fn approve_at_provider(settings: &Settings, authorize_location: &str) -> String {
    let url = authorize_location.strip_prefix(&format!("http://{}", settings.mock().unwrap().addr())).expect("the provider's own address");
    let (status, location, _) = http_get(settings.mock().unwrap().addr(), url).await;
    assert_eq!(status, 302);
    let location = location.expect("a Location header");
    // `http://127.0.0.1:3000/auth/provider/callback?code=..&state=..` -> `/auth/provider/callback?code=..&state=..`
    location.strip_prefix("http://127.0.0.1:3000").expect("the registered redirect URI").to_string()
}
