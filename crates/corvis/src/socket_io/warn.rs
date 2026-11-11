//! Rate-limited log lines of the Socket.IO layer and of the Engine.IO server under it.
//!
//! A line that a client can provoke (a handler that panics on its input, a connect middleware that is slow for it, a
//! session closed for its send buffer, a handshake refused at a cap) would let that client write to the log at the
//! rate it sends packets. Each kind of line is written at most once per interval and per namespace (the engine has
//! none and uses the empty name), and the next one that gets through says how many were left out. The keys are the
//! static description of the line and the name of a namespace the application registered: never anything from the
//! client. The lines never carry a panic message or any data of a packet. A line that is left out costs a lookup and
//! nothing else: its text is built only when it is written.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use crate::logger::Logger;

/// At most one line per kind and namespace in this period.
pub(crate) const WARN_INTERVAL: Duration = Duration::from_secs(60);
/// The most (kind, namespace) pairs remembered, over every kind: namespaces are registered by the application, so this
/// is never reached unless it registers thousands; beyond it new pairs are dropped silently instead of growing the
/// table.
const MAX_KEYS: usize = 4_096;

/// When the last line of a (kind, namespace) pair was written, and how many were left out since.
struct Seen {
    last: Instant,
    left_out: u64,
}

#[derive(Default)]
struct Table {
    /// kind -> namespace -> what was seen. Looked up by `&str`: a namespace name is copied only on first insert.
    kinds: HashMap<&'static str, HashMap<String, Seen>>,
    /// The (kind, namespace) pairs held, at most `MAX_KEYS`.
    pairs: usize,
}

pub(crate) struct WarnLimiter {
    interval: Duration,
    /// The logger context the lines are written under (`SocketIO`, `EngineIO`).
    context: &'static str,
    state: Mutex<Table>,
    /// The messages built, for the tests that check a line left out builds none.
    #[cfg(test)]
    built: std::sync::atomic::AtomicUsize,
}

impl WarnLimiter {
    pub(crate) fn new(interval: Duration, context: &'static str) -> Self {
        WarnLimiter {
            interval,
            context,
            state: Mutex::new(Table::default()),
            #[cfg(test)]
            built: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// `Some(left_out)` when a line of this kind for this namespace may be written now (`left_out` counts the ones left
    /// out since the last), `None` when it must be dropped.
    pub(crate) fn allow(&self, kind: &'static str, namespace: &str, now: Instant) -> Option<u64> {
        let mut table = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Table { kinds, pairs } = &mut *table;
        if let Some(seen) = kinds.get_mut(kind).and_then(|by_namespace| by_namespace.get_mut(namespace)) {
            if now.saturating_duration_since(seen.last) < self.interval {
                seen.left_out += 1;
                return None;
            }
            seen.last = now;
            return Some(std::mem::take(&mut seen.left_out));
        }
        if *pairs >= MAX_KEYS {
            return None;
        }
        *pairs += 1;
        kinds.entry(kind).or_default().insert(namespace.to_string(), Seen { last: now, left_out: 0 });
        Some(0)
    }

    /// The text to log now, or `None` when the line is rate limited; `message` is called only in the first case.
    pub(crate) fn line(&self, kind: &'static str, namespace: &str, message: impl FnOnce() -> String, now: Instant) -> Option<String> {
        let left_out = self.allow(kind, namespace, now)?;
        #[cfg(test)]
        self.built.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let message = message();
        Some(if left_out > 0 { format!("{message} ({left_out} more of this kind were not logged since the last one)") } else { message })
    }

    /// An error line for a handler that panicked: the namespace only, never the panic message.
    pub(crate) fn panicked(&self, what: &'static str, namespace: &str) {
        if let Some(line) = self.line(what, namespace, || format!("{what} of the namespace {namespace} panicked"), Instant::now()) {
            Logger::new(self.context).error(line);
        }
    }

    /// A warning, rate limited like the panics.
    pub(crate) fn warn(&self, kind: &'static str, namespace: &str, message: impl FnOnce() -> String) {
        if let Some(line) = self.line(kind, namespace, message, Instant::now()) {
            Logger::new(self.context).warn(line);
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
        self.state.lock().unwrap_or_else(|e| e.into_inner()).pairs
    }

    #[cfg(test)]
    pub(crate) fn built(&self) -> usize {
        self.built.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter() -> WarnLimiter {
        WarnLimiter::new(Duration::from_secs(60), "SocketIO")
    }

    #[test]
    fn a_line_is_written_once_per_interval_with_the_count_of_those_left_out() {
        let limiter = limiter();
        let t0 = Instant::now();
        let boom = || "boom".to_string();
        assert_eq!(limiter.line("a handler", "/chat", boom, t0).as_deref(), Some("boom"));
        for i in 1..=5 {
            assert_eq!(limiter.line("a handler", "/chat", boom, t0 + Duration::from_secs(i)), None);
        }
        let later = limiter.line("a handler", "/chat", boom, t0 + Duration::from_secs(60)).unwrap();
        assert_eq!(later, "boom (5 more of this kind were not logged since the last one)");
        assert_eq!(limiter.line("a handler", "/chat", boom, t0 + Duration::from_secs(121)).as_deref(), Some("boom"));
    }

    #[test]
    fn a_line_left_out_builds_no_message() {
        let limiter = limiter();
        let t0 = Instant::now();
        let mut calls = 0;
        for i in 0..1_000 {
            let _ = limiter.line(
                "a handler",
                "/chat",
                || {
                    calls += 1;
                    format!("line {i}")
                },
                t0 + Duration::from_millis(i),
            );
        }
        assert_eq!(calls, 1, "only the line that was written built its text");
        assert_eq!(limiter.built(), 1);
    }

    #[test]
    fn each_kind_and_namespace_has_its_own_budget() {
        let limiter = limiter();
        let t0 = Instant::now();
        assert!(limiter.allow("a handler", "/a", t0).is_some());
        assert!(limiter.allow("a handler", "/b", t0).is_some());
        assert!(limiter.allow("middleware", "/a", t0).is_some());
        assert!(limiter.allow("a handler", "/a", t0).is_none());
        assert_eq!(limiter.keys(), 3);
    }

    #[test]
    fn the_table_cannot_grow_past_its_cap() {
        let limiter = limiter();
        let t0 = Instant::now();
        for i in 0..MAX_KEYS + 10 {
            let _ = limiter.allow("a handler", &format!("/n{i}"), t0);
        }
        assert_eq!(limiter.keys(), MAX_KEYS);
        assert!(limiter.allow("a handler", "/new", t0).is_none());
        assert!(limiter.allow("another kind", "/n0", t0).is_none(), "the cap counts the pairs of every kind");
    }
}
