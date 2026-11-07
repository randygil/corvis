//! The built-in logger. Its console format:
//!
//! ```text
//! [Corvis] 18072  - 11/07/2026, 1:20:03 AM     LOG [CorvisFactory] Starting Corvis application...
//! ```

use std::io::Write;
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::Instant;

/// Log levels (`'log' | 'error' | 'warn' | 'debug' | 'verbose' | 'fatal'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogLevel {
    Log,
    Error,
    Warn,
    Debug,
    Verbose,
    Fatal,
}

impl LogLevel {
    pub fn label(&self) -> &'static str {
        match self {
            LogLevel::Log => "LOG",
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Debug => "DEBUG",
            LogLevel::Verbose => "VERBOSE",
            LogLevel::Fatal => "FATAL",
        }
    }

    fn color(&self) -> &'static str {
        match self {
            LogLevel::Log => "\x1b[32m",
            LogLevel::Error | LogLevel::Fatal => "\x1b[31m",
            LogLevel::Warn => "\x1b[33m",
            LogLevel::Debug => "\x1b[95m",
            LogLevel::Verbose => "\x1b[96m",
        }
    }

    pub fn all() -> Vec<LogLevel> {
        vec![LogLevel::Log, LogLevel::Error, LogLevel::Warn, LogLevel::Debug, LogLevel::Verbose, LogLevel::Fatal]
    }
}

/// Custom logger implementations (`LoggerService`).
pub trait LoggerService: Send + Sync + 'static {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>);
}

/// The default console logger.
pub struct ConsoleLogger {
    app_name: String,
    colors: bool,
    json: bool,
}

impl Default for ConsoleLogger {
    fn default() -> Self {
        let colors = std::env::var("NO_COLOR").is_err();
        ConsoleLogger { app_name: "Corvis".into(), colors, json: false }
    }
}

impl ConsoleLogger {
    pub fn new() -> Self {
        Self::default()
    }
    /// Prefix shown before the PID (default `Corvis`).
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.app_name = prefix.into();
        self
    }
    pub fn colors(mut self, colors: bool) -> Self {
        self.colors = colors;
        self
    }
    /// Structured JSON output.
    pub fn json(mut self, json: bool) -> Self {
        self.json = json;
        self
    }
}

static LAST_LOG: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));

impl LoggerService for ConsoleLogger {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        let now = chrono::Local::now();
        let pid = std::process::id();
        let elapsed = {
            let mut last = LAST_LOG.lock().unwrap();
            let e = last.map(|l| l.elapsed().as_millis());
            *last = Some(Instant::now());
            e
        };
        if self.json {
            let v = serde_json::json!({
                "level": level.label().to_lowercase(),
                "pid": pid,
                "timestamp": now.timestamp_millis(),
                "message": message,
                "context": context,
            });
            let _ = writeln!(std::io::stdout(), "{v}");
            return;
        }
        let date = now.format("%m/%d/%Y, %-I:%M:%S %p");
        let ctx = context.map(|c| format!("[{c}] ")).unwrap_or_default();
        let diff = elapsed.map(|ms| format!(" +{ms}ms")).unwrap_or_default();
        let line = if self.colors {
            let c = level.color();
            format!(
                "{c}[{}] {pid}  - \x1b[39m{date} {c}{:>7}\x1b[39m \x1b[33m{ctx}\x1b[39m{c}{message}\x1b[39m\x1b[33m{diff}\x1b[39m",
                self.app_name,
                level.label()
            )
        } else {
            format!("[{}] {pid}  - {date} {:>7} {ctx}{message}{diff}", self.app_name, level.label())
        };
        if matches!(level, LogLevel::Error | LogLevel::Fatal) {
            let _ = writeln!(std::io::stderr(), "{line}");
        } else {
            let _ = writeln!(std::io::stdout(), "{line}");
        }
    }
}

struct LoggerState {
    service: Arc<dyn LoggerService>,
    levels: Vec<LogLevel>,
    enabled: bool,
}

static STATE: LazyLock<RwLock<LoggerState>> = LazyLock::new(|| {
    RwLock::new(LoggerState { service: Arc::new(ConsoleLogger::default()), levels: LogLevel::all(), enabled: true })
});

/// Replace the global logger (`app.useLogger(...)`).
pub fn set_logger(service: impl LoggerService) {
    STATE.write().unwrap().service = Arc::new(service);
}

/// Enable only the given levels (`logger: ['error', 'warn']`).
pub fn set_log_levels(levels: &[LogLevel]) {
    STATE.write().unwrap().levels = levels.to_vec();
}

/// Disable / enable all logging (`logger: false`).
pub fn set_logging_enabled(enabled: bool) {
    STATE.write().unwrap().enabled = enabled;
}

fn emit(level: LogLevel, message: &str, context: Option<&str>) {
    let (service, ok) = {
        let s = STATE.read().unwrap();
        (s.service.clone(), s.enabled && s.levels.contains(&level))
    };
    if ok {
        service.log(level, message, context);
    }
}

/// `new Logger(CatsService.name)`.
#[derive(Debug, Clone, Default)]
pub struct Logger {
    context: Option<String>,
}

impl Logger {
    pub fn new(context: impl Into<String>) -> Self {
        Logger { context: Some(context.into()) }
    }
    /// A logger named after a type (`new Logger(CatsService.name)`).
    pub fn for_type<T: ?Sized>() -> Self {
        Logger::new(crate::di::token::short_type_name(std::any::type_name::<T>()))
    }
    pub fn log(&self, message: impl AsRef<str>) {
        emit(LogLevel::Log, message.as_ref(), self.context.as_deref());
    }
    pub fn error(&self, message: impl AsRef<str>) {
        emit(LogLevel::Error, message.as_ref(), self.context.as_deref());
    }
    pub fn warn(&self, message: impl AsRef<str>) {
        emit(LogLevel::Warn, message.as_ref(), self.context.as_deref());
    }
    pub fn debug(&self, message: impl AsRef<str>) {
        emit(LogLevel::Debug, message.as_ref(), self.context.as_deref());
    }
    pub fn verbose(&self, message: impl AsRef<str>) {
        emit(LogLevel::Verbose, message.as_ref(), self.context.as_deref());
    }
    pub fn fatal(&self, message: impl AsRef<str>) {
        emit(LogLevel::Fatal, message.as_ref(), self.context.as_deref());
    }
    /// Static logging with an explicit context (`Logger.log('msg', 'Ctx')`).
    pub fn log_with(message: impl AsRef<str>, context: &str) {
        emit(LogLevel::Log, message.as_ref(), Some(context));
    }
    pub fn error_with(message: impl AsRef<str>, context: &str) {
        emit(LogLevel::Error, message.as_ref(), Some(context));
    }
    pub fn warn_with(message: impl AsRef<str>, context: &str) {
        emit(LogLevel::Warn, message.as_ref(), Some(context));
    }
}
