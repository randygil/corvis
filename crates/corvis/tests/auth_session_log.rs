//! What the session module logs: startup notes for unsafe choices, one rate-limited line for what a client can
//! trigger, and nothing that holds a session identifier, a secret, a cookie value or session data. Its own test binary:
//! the logger is process-wide, so the tests share one capturing sink and are serialized.
#![cfg(feature = "auth-session")]

mod session_common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::auth::session::{MemorySessionStore, SessionCookie, SessionStore};
use corvis::testing::request;
use corvis::{LogLevel, LoggerService, LoggingInterceptor};
use session_common::*;

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

async fn logged_app(options: corvis::auth::session::SessionOptions) -> corvis::CorvisApplication {
    app_with(options, |b| b.with_logger(true)).await
}

#[tokio::test]
async fn unsafe_choices_log_one_note_each_at_startup() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let _app = logged_app(options().cookie(SessionCookie::default().secure(false).http_only(false)).no_absolute_timeout()).await;
    let notes = lines(&captured, "SessionModule");
    let warnings: Vec<&String> = notes
        .iter()
        .filter(|n| n.contains("not Secure") || n.contains("not HttpOnly") || n.contains("no maximum age") || n.contains("kept in memory"))
        .collect();
    assert_eq!(warnings.len(), 4, "{notes:?}");
    assert!(notes.iter().any(|n| n.starts_with("session cookie \"connect.sid\": Path=/")), "the attributes are logged: {notes:?}");

    captured.lock().unwrap().clear();
    let _app = logged_app(options()).await;
    let notes = lines(&captured, "SessionModule");
    assert_eq!(notes.iter().filter(|n| n.contains("not Secure")).count(), 0, "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("Secure; SameSite=Lax")), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("kept in memory")), "{notes:?}");
}

#[tokio::test]
async fn nothing_secret_is_logged_and_what_a_client_triggers_is_rate_limited() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let store = RecordingStore::new();
    let mut app = app_with(options().store_arc(store.clone()), |b| b.with_logger(true)).await;
    app.use_global_interceptors(LoggingInterceptor);

    let marker = "dataMarkerSECRETdataMarker";
    let cookie = pair(&set_cookies(&request(&app).post(&format!("/set?k=a&v={marker}")).send().await)[0]);
    let id = id_of(&cookie);
    let value = cookie.split_once('=').unwrap().1.to_string();
    // A flood of bad cookies, an oversized session and a failing store.
    for _ in 0..20 {
        request(&app).get("/sess").header("cookie", &format!("connect.sid={id}")).send().await;
        request(&app).get("/sess").header("cookie", &format!("connect.sid={}x", value)).send().await;
    }
    for _ in 0..30 {
        request(&app).post("/big").send().await;
    }
    *store.fail_load.lock().unwrap() = true;
    for _ in 0..30 {
        let r = request(&app).get("/sess").header("cookie", &cookie).send().await;
        assert_eq!(r.status, 500);
    }
    *store.fail_load.lock().unwrap() = false;

    let session_lines = lines(&captured, "SessionMiddleware");
    let loads = session_lines.iter().filter(|l| l.contains("while loading")).count();
    let big = session_lines.iter().filter(|l| l.contains("max_session_bytes")).count();
    assert_eq!(loads, 1, "one line for 30 failures: {session_lines:?}");
    assert_eq!(big, 1, "one line for 30 oversized sessions: {session_lines:?}");
    // The default exception filter does not write a line per failed request.
    assert!(!all_lines(&captured).iter().any(|l| l.contains("session failure") && l.contains("ExceptionsHandler")));

    // No line of the whole run holds the identifier, the cookie value, a secret or the session data.
    for line in all_lines(&captured) {
        assert!(!line.contains(&id), "{line}");
        assert!(!line.contains(&value), "{line}");
        assert!(!line.contains(NEW) && !line.contains(OLD), "{line}");
        assert!(!line.contains(marker), "{line}");
        assert!(!line.contains("SECRET-ID"), "{line}");
    }
}

#[tokio::test(start_paused = true)]
async fn eviction_warnings_hold_no_identifier_and_are_limited() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let store = MemorySessionStore::new().max_sessions(5);
    let ttl = Duration::from_secs(60);
    let record = serde_json::json!({ "cookie": { "originalMaxAge": null } });
    for i in 0..200 {
        store.create(&format!("identifier-{i}-IDENTIFIER"), &record, ttl).await.unwrap();
    }
    let warnings = lines(&captured, "MemorySessionStore");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("evicted") && warnings[0].contains("max_sessions 5"), "{warnings:?}");
    assert!(!warnings[0].contains("identifier-"), "{warnings:?}");
}
