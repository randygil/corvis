//! "Sign in with GitHub" against a mock provider on 127.0.0.1: the login redirects with a `state` and a PKCE challenge,
//! the callback exchanges the code, the response carries the application's token, and a forged callback is refused
//! before anything is sent to the provider. The endpoints come from `GITHUB_ENDPOINTS_BASE`, which only tests set.

use std::sync::{Arc, Mutex};

use corvis::testing::request;
use corvis::tokio::io::{AsyncReadExt, AsyncWriteExt};
use corvis::tokio::net::TcpListener;

/// A provider that answers the token request and the two profile requests, and records the paths it was asked for.
async fn provider(paths: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    corvis::tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let paths = paths.clone();
            corvis::tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut read = 0;
                // The request is small: read until the end of the header block (the token request's body follows it).
                loop {
                    let n = stream.read(&mut buf[read..]).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    read += n;
                    if buf[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&buf[..read]).into_owned();
                let path = head.split(' ').nth(1).unwrap_or("").to_string();
                paths.lock().unwrap().push(path.clone());
                let body = match path.as_str() {
                    "/token" => r#"{"access_token":"gho_FAKE","token_type":"bearer","scope":"user:email"}"#,
                    "/user" => r#"{"id":4242,"login":"octocat","name":"The Octocat","email":null}"#,
                    "/user/emails" => r#"[{"email":"octocat@example.com","primary":true,"verified":true}]"#,
                    _ => "{}",
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    port
}

fn cookie_pair(response: &corvis::testing::TestResponse) -> String {
    response.headers.get("set-cookie").expect("the state cookie").to_str().unwrap().split(';').next().unwrap().to_string()
}

#[corvis::test]
async fn the_github_login_ends_with_the_applications_token() {
    let paths = Arc::new(Mutex::new(Vec::new()));
    let port = provider(paths.clone()).await;
    // SAFETY: set before the application reads the environment; this binary has a single test.
    unsafe {
        std::env::set_var("JWT_SECRET", "cats-test-secret-0123456789abcdef");
        std::env::set_var("SESSION_SECRET", "cats-session-secret-0123456789abcdef");
        std::env::set_var("GITHUB_CLIENT_ID", "cats-client-id");
        std::env::set_var("GITHUB_CLIENT_SECRET", "cats-client-secret");
        std::env::set_var("OAUTH_STATE_SECRET", "cats-oauth-state-secret-0123456789abcdef");
        std::env::set_var("GITHUB_ENDPOINTS_BASE", format!("http://127.0.0.1:{port}"));
    }
    let mut app = cats_app::create_app().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let c = request(&app);

    let login = c.get("/api/v1/auth/github/login?return_to=/cats").send().await;
    assert_eq!(login.status, 302, "{}", login.text());
    let location = login.header("location").unwrap().to_string();
    let (target, query) = location.split_once('?').expect("a query");
    assert!(target.ends_with("/authorize"), "{target}");
    // The values stay percent-encoded.
    let param = |name: &str| query.split('&').find_map(|pair| pair.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v.to_string()));
    assert_eq!(param("scope").as_deref(), Some("user%3Aemail"));
    assert_eq!(param("code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(param("client_id").as_deref(), Some("cats-client-id"));
    assert_eq!(param("redirect_uri").as_deref(), Some("http%3A%2F%2F127.0.0.1%3A3000%2Fapi%2Fv1%2Fauth%2Fgithub%2Fcallback"));
    let state = param("state").expect("a state");
    let cookie = cookie_pair(&login);
    assert!(cookie.starts_with("__Host-oauth2.github="), "{cookie}");
    assert!(paths.lock().unwrap().is_empty(), "the login start contacts nobody");

    // A callback that does not carry this browser's state never reaches the provider.
    let forged = c.get("/api/v1/auth/github/callback?code=abc&state=forged").header("cookie", &cookie).send().await;
    assert_eq!(forged.status, 401);
    let no_cookie = c.get(&format!("/api/v1/auth/github/callback?code=abc&state={state}")).send().await;
    assert_eq!(no_cookie.status, 401);
    assert!(paths.lock().unwrap().is_empty());

    let callback = c.get(&format!("/api/v1/auth/github/callback?code=abc&state={state}")).header("cookie", &cookie).send().await;
    assert_eq!(callback.status, 200, "{}", callback.text());
    assert_eq!(callback.header("cache-control"), Some("no-store"));
    let body = callback.json_value();
    assert_eq!(body["return_to"], "/cats");
    let token = body["access_token"].as_str().expect("the application's token").to_string();
    assert!(!callback.text().contains("gho_FAKE"), "the provider's token never reaches the response");
    assert_eq!(*paths.lock().unwrap(), ["/token", "/user", "/user/emails"]);

    // The token opens the protected routes as the account created for GitHub user 4242.
    let profile = c.get("/api/v1/auth/profile").bearer(&token).send().await;
    assert_eq!(profile.status, 200, "{}", profile.text());
    assert_eq!(profile.json_value()["username"], "github:octocat");
}
