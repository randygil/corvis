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

    /// One line of the text format. The message and the context are escaped (see
    /// [`escape_control`]) so logged user data cannot start fake entries or drive the terminal.
    fn text_line(&self, level: LogLevel, message: &str, context: Option<&str>, pid: u32, date: &str, elapsed: Option<u128>) -> String {
        let message = escape_control(message);
        let ctx = context.map(|c| format!("[{}] ", escape_control(c))).unwrap_or_default();
        let diff = elapsed.map(|ms| format!(" +{ms}ms")).unwrap_or_default();
        if self.colors {
            let c = level.color();
            format!(
                "{c}[{}] {pid}  - \x1b[39m{date} {c}{:>7}\x1b[39m \x1b[33m{ctx}\x1b[39m{c}{message}\x1b[39m\x1b[33m{diff}\x1b[39m",
                self.app_name,
                level.label()
            )
        } else {
            format!("[{}] {pid}  - {date} {:>7} {ctx}{message}{diff}", self.app_name, level.label())
        }
    }
}

/// Escapes what a terminal or a line-oriented log reader would interpret: `\r` and `\n` as
/// `\r` / `\n`, the other C0 controls (except tab) and DEL as `\xNN`, and the C1 controls
/// (`U+0080`–`U+009F`, e.g. the 8-bit CSI `U+009B`) as `\u{NN}`.
fn escape_control(s: &str) -> std::borrow::Cow<'_, str> {
    let special = |c: char| c.is_control() && c != '\t';
    if !s.chars().any(special) {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if special(c) && (c as u32) < 0x80 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if special(c) => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    std::borrow::Cow::Owned(out)
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
        let date = now.format("%m/%d/%Y, %-I:%M:%S %p").to_string();
        let line = self.text_line(level, message, context, pid, &date, elapsed);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// static-response #9: user data cannot forge log lines or send terminal escapes.
    #[test]
    fn text_mode_escapes_control_characters() {
        let plain = ConsoleLogger::new().colors(false);
        let line = plain.text_line(
            LogLevel::Warn,
            "login failed for x\r\n[Corvis] 1  - 01/01/2026, 1:00:00 AM     LOG [Auth] admin logged in\x1b[2J\u{9b}31m",
            Some("Users\nX"),
            7,
            "D",
            None,
        );
        assert_eq!(
            line,
            r"[Corvis] 7  - D    WARN [Users\nX] login failed for x\r\n[Corvis] 1  - 01/01/2026, 1:00:00 AM     LOG [Auth] admin logged in\x1b[2J\u{9b}31m"
        );
        assert!(!line.contains('\n') && !line.contains('\r') && !line.contains('\x1b') && !line.contains('\u{9b}'));
        // Tabs and printable Unicode stay as they are; the colored format keeps its own escapes.
        assert_eq!(plain.text_line(LogLevel::Log, "a\tb ñ €", None, 7, "D", Some(3)), "[Corvis] 7  - D     LOG a\tb ñ € +3ms");
        let colored = ConsoleLogger::new().colors(true).text_line(LogLevel::Log, "x\x1b[2Jy", None, 7, "D", None);
        assert!(colored.contains(r"x\x1b[2Jy"), "{colored}");
    }
}
