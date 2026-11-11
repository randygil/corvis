//! The throttler is a global guard, so it runs before every `AuthGuard`: startup logs no guard-order warning. Its own test
//! binary, because the logger is process-wide.

use std::sync::{Arc, Mutex};

use auth_app::Settings;
use corvis::{LogLevel, LoggerService};

struct Capture(Arc<Mutex<Vec<(LogLevel, String)>>>);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, _context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string()));
    }
}

#[corvis::test]
async fn startup_logs_no_guard_order_warning() {
    let captured = Arc::default();
    let settings = Settings::dev(3000).await.unwrap();
    let mut app = auth_app::create_app(settings).await.unwrap();
    corvis::logger::set_logging_enabled(true);
    corvis::logger::set_log_levels(&[LogLevel::Log, LogLevel::Error, LogLevel::Warn, LogLevel::Debug, LogLevel::Verbose, LogLevel::Fatal]);
    app.use_logger(Capture(Arc::clone(&captured)));
    app.silence_route_logs();
    app.init().await.unwrap();

    let lines = captured.lock().unwrap();
    assert!(!lines.is_empty(), "the capture saw the startup logs");
    let order: Vec<_> = lines.iter().filter(|(_, m)| m.contains("before ThrottlerGuard")).collect();
    assert!(order.is_empty(), "{order:?}");
    // The `--dev` settings are announced as what they are: cookies without `Secure`.
    assert!(lines.iter().any(|(l, m)| *l == LogLevel::Warn && m.contains("not Secure")), "{lines:?}");
}
