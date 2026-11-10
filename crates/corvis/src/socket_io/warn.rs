//! Rate-limited log lines of the Socket.IO layer.
//!
//! A line that a client can provoke (a handler that panics on its input, a connect middleware that is slow for it)
//! would let that client write to the log at the rate it sends packets. Each kind of line is written at most once per
//! interval and per namespace, and the next one that gets through says how many were left out. The keys are the
//! static description of the line and the name of a namespace the application registered: never anything from the
//! client. The lines never carry a panic message or any data of a packet.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use crate::logger::Logger;

/// At most one line per kind and namespace in this period.
pub(crate) const WARN_INTERVAL: Duration = Duration::from_secs(60);
/// The most (kind, namespace) pairs remembered: namespaces are registered by the application, so this is never
/// reached unless it registers thousands; beyond it new pairs are dropped silently instead of growing the table.
const MAX_KEYS: usize = 4_096;

/// (kind, namespace) -> (when the last line was written, how many were left out since).
type Seen = HashMap<(&'static str, String), (Instant, u64)>;

pub(crate) struct WarnLimiter {
    interval: Duration,
    state: Mutex<Seen>,
}

impl WarnLimiter {
    pub(crate) fn new(interval: Duration) -> Self {
        WarnLimiter { interval, state: Mutex::new(HashMap::new()) }
    }

    /// `Some(suppressed)` when a line of this kind for this namespace may be written now (`suppressed` counts the
    /// ones left out since the last), `None` when it must be dropped.
    pub(crate) fn allow(&self, kind: &'static str, namespace: &str, now: Instant) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let key = (kind, namespace.to_string());
        match state.get_mut(&key) {
            None => {
                if state.len() >= MAX_KEYS {
                    return None;
                }
                state.insert(key, (now, 0));
                Some(0)
            }
            Some((last, suppressed)) if now.saturating_duration_since(*last) >= self.interval => {
                let count = *suppressed;
                (*last, *suppressed) = (now, 0);
                Some(count)
            }
            Some((_, suppressed)) => {
                *suppressed += 1;
                None
            }
        }
    }

    /// The text to log for `message` now, or `None` when it is rate limited.
    pub(crate) fn line(&self, kind: &'static str, namespace: &str, message: &str, now: Instant) -> Option<String> {
        let suppressed = self.allow(kind, namespace, now)?;
        Some(if suppressed > 0 {
            format!("{message} ({suppressed} more of this kind were not logged in the last minute)")
        } else {
            message.to_string()
        })
    }

    /// An error line for a handler that panicked: the namespace only, never the panic message.
    pub(crate) fn panicked(&self, what: &'static str, namespace: &str) {
        if let Some(line) = self.line(what, namespace, &format!("{what} of the namespace {namespace} panicked"), Instant::now()) {
            Logger::new("SocketIO").error(line);
        }
    }

    /// A warning, rate limited like the panics.
    pub(crate) fn warn(&self, kind: &'static str, namespace: &str, message: &str) {
        if let Some(line) = self.line(kind, namespace, message, Instant::now()) {
            Logger::new("SocketIO").warn(line);
        }
    }

    /// Runs a synchronous handler; a panic is logged through [`panicked`](Self::panicked) and `false` returned.
    pub(crate) fn guard(&self, what: &'static str, namespace: &str, f: impl FnOnce()) -> bool {
        if catch_unwind(AssertUnwindSafe(f)).is_err() {
            self.panicked(what, namespace);
            return false;
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn keys(&self) -> usize {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_written_once_per_interval_with_the_count_of_those_left_out() {
        let limiter = WarnLimiter::new(Duration::from_secs(60));
        let t0 = Instant::now();
        assert_eq!(limiter.line("a handler", "/chat", "boom", t0).as_deref(), Some("boom"));
        for i in 1..=5 {
            assert_eq!(limiter.line("a handler", "/chat", "boom", t0 + Duration::from_secs(i)), None);
        }
        let later = limiter.line("a handler", "/chat", "boom", t0 + Duration::from_secs(60)).unwrap();
        assert_eq!(later, "boom (5 more of this kind were not logged in the last minute)");
        assert_eq!(limiter.line("a handler", "/chat", "boom", t0 + Duration::from_secs(121)).as_deref(), Some("boom"));
    }

    #[test]
    fn each_kind_and_namespace_has_its_own_budget() {
        let limiter = WarnLimiter::new(Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(limiter.allow("a handler", "/a", t0).is_some());
        assert!(limiter.allow("a handler", "/b", t0).is_some());
        assert!(limiter.allow("middleware", "/a", t0).is_some());
        assert!(limiter.allow("a handler", "/a", t0).is_none());
        assert_eq!(limiter.keys(), 3);
    }

    #[test]
    fn the_table_cannot_grow_past_its_cap() {
        let limiter = WarnLimiter::new(Duration::from_secs(60));
        let t0 = Instant::now();
        for i in 0..MAX_KEYS + 10 {
            let _ = limiter.allow("a handler", &format!("/n{i}"), t0);
        }
        assert_eq!(limiter.keys(), MAX_KEYS);
        assert!(limiter.allow("a handler", "/new", t0).is_none());
    }
}
