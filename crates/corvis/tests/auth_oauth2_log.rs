//! What the OAuth2 strategy logs: startup warnings for unsafe choices, one rate-limited line for what a client can
//! trigger, a fixed line for a failed exchange, the callback's `code` and `state` hidden in the logged URL, and nothing
//! that holds a token, a secret, a code, a state or personal data. Its own test binary: the logger is process-wide, so
//! the tests share one capturing sink and are serialized.
#![cfg(feature = "auth-oauth2")]

mod oauth2_common;

use std::sync::{Arc, Mutex};

use corvis::testing::request;
use corvis::{LogLevel, LoggerService};
use oauth2_common::*;

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn capture() -> Captured {
    let captured: Captured = Arc::default();
    corvis::logger::set_logger(Capture(captured.clone()));
    corvis::logger::set_log_levels(&LogLevel::all());
    captured
}

fn all_lines(captured: &Captured) -> Vec<String> {
    captured.lock().unwrap().iter().map(|(_, m, _)| m.clone()).collect()
}

fn lines(captured: &Captured, context: &str) -> Vec<String> {
    captured.lock().unwrap().iter().filter(|(_, _, c)| c.as_deref() == Some(context)).map(|(_, m, _)| m.clone()).collect()
}

#[tokio::test]
async fn unsafe_choices_are_logged_once_at_startup_naming_the_strategy() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(google_options(&mock).pkce(false).state_cookie_secure(false));
    let _app = AppBuilder::default().google(&cfg).logger().build().await;
    let notes = lines(&captured, "OAuth2Strategy");
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("\"google\"") && n.contains("PKCE is turned off")), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("\"google\"") && n.contains("not Secure")), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("\"google\"") && n.contains("\"oauth2.google\"") && n.contains("__Host-")), "{notes:?}");

    captured.lock().unwrap().clear();
    let cfg = Cfg::new(google_options(&mock));
    let _app = AppBuilder::default().google(&cfg).logger().build().await;
    assert!(lines(&captured, "OAuth2Strategy").is_empty(), "the safe defaults log nothing");
}

#[tokio::test]
async fn a_state_cookie_without_the_host_prefix_warns_naming_the_strategy_and_the_cookie() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(google_options(&mock).state_cookie_name("oauth_state"));
    let _app = AppBuilder::default().google(&cfg).logger().build().await;
    let notes = lines(&captured, "OAuth2Strategy");
    assert_eq!(notes.len(), 1, "{notes:?}");
    let note = &notes[0];
    assert!(note.contains("\"google\"") && note.contains("\"oauth_state\""), "{note}");
    assert!(note.contains("sibling subdomain") && note.contains("login CSRF") && note.contains("__Host-"), "{note}");

    // a prefixed custom name is the safe choice and says nothing; the cookie value is never printed
    captured.lock().unwrap().clear();
    let cfg = Cfg::new(google_options(&mock).state_cookie_name("__Host-my-state"));
    let _app = AppBuilder::default().google(&cfg).logger().build().await;
    assert!(lines(&captured, "OAuth2Strategy").is_empty());
}

#[tokio::test]
async fn state_in_the_session_warns_when_the_session_cookie_has_no_host_prefix() {
    use corvis::auth::session::{SessionCookie, SessionOptions};
    let _g = SERIAL.lock().await;
    let captured = capture();
    let mock = Mock::start(standard).await;
    let session = |name: &str| {
        SessionOptions::with_secrets(["oauth2-test-session-secret-0123456789abcdef"]).unwrap().name(name).cookie(SessionCookie::default())
    };
    let cfg = Cfg::new(google_options(&mock).state_in_session());
    let _app = AppBuilder::default().google(&cfg).session(session("connect.sid")).logger().build().await;
    let notes = lines(&captured, "OAuth2Strategy");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("\"google\"") && notes[0].contains("\"connect.sid\"") && notes[0].contains("sibling subdomain"), "{notes:?}");
    assert!(notes[0].contains("__Host-"), "{notes:?}");

    captured.lock().unwrap().clear();
    let cfg = Cfg::new(google_options(&mock).state_in_session());
    let _app = AppBuilder::default().google(&cfg).session(session("__Host-sid")).logger().build().await;
    assert!(lines(&captured, "OAuth2Strategy").is_empty());
}

#[tokio::test]
async fn a_failed_exchange_logs_a_fixed_line_without_the_provider_text() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let mock = Mock::start(|r| {
        if r.path() == "/token" {
            let echo = format!("code={} secret={}", r.field("code").unwrap_or_default(), r.field("client_secret").unwrap_or_default());
            Reply::json(500, &format!("{{\"echo\":\"{echo}\"}}"))
        } else {
            standard(r)
        }
    })
    .await;
    let cfg = Cfg::new(google_options(&mock));
    let app = AppBuilder::default().google(&cfg).logger().build().await;
    let l = login(&app, "/auth/google").await;
    let r = callback(&app, "/auth/google/callback", &l, "THE-CODE-4711").await;
    assert_eq!(r.status, 500);
    let lines = lines(&captured, "OAuth2Strategy");
    let failed: Vec<&String> = lines.iter().filter(|l| l.contains("token request failed")).collect();
    assert_eq!(failed.len(), 1, "{lines:?}");
    assert!(failed[0].contains("\"google\"") && failed[0].contains("127.0.0.1") && failed[0].contains("500"), "{}", failed[0]);
    for line in all_lines(&captured) {
        for secret in ["THE-CODE-4711", CLIENT_SECRET, STATE_SECRET, &l.state(), "ACCESS-TOKEN", "echo"] {
            assert!(!line.contains(secret), "{line}");
        }
    }
}

#[tokio::test]
async fn what_a_client_can_trigger_is_rate_limited_and_the_logged_url_hides_the_credentials() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(google_options(&mock));
    let app = AppBuilder::default().google(&cfg).logger().logging_interceptor().build().await;
    let l = login(&app, "/auth/google").await;
    let state = l.state();
    // a flood of forged callbacks
    for i in 0..40 {
        let r = request(&app).get(&format!("/auth/google/callback?code=FORGED-CODE-{i}&state=forged-{i}")).send().await;
        assert_eq!(r.status, 401);
    }
    let r = request(&app)
        .get(&format!("/auth/google/callback?code=REAL-CODE-9&state={state}"))
        .header("cookie", l.cookie.as_deref().unwrap())
        .send()
        .await;
    assert_eq!(r.status, 200);
    let strategy_lines = lines(&captured, "OAuth2Strategy");
    let mismatches = strategy_lines.iter().filter(|l| l.contains("state mismatch")).count();
    assert_eq!(mismatches, 1, "one line for 40 forged callbacks: {strategy_lines:?}");
    // the request log names the route and hides `code` and `state`
    let logged = lines(&captured, "LoggingInterceptor");
    assert!(logged.iter().any(|l| l.contains("GET /auth/google/callback?code=REDACTED&state=REDACTED 200")), "{logged:?}");
    for line in all_lines(&captured) {
        for secret in [
            "REAL-CODE-9",
            "FORGED-CODE",
            state.as_str(),
            "forged-",
            "ACCESS-TOKEN",
            "REFRESH-TOKEN",
            "ID-TOKEN",
            CLIENT_SECRET,
            STATE_SECRET,
            "ana@example.com",
        ] {
            assert!(!line.contains(secret), "{line}");
        }
    }
}
