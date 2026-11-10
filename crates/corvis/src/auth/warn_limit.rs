//! A limiter for warnings a client can trigger: at most one line per reason and interval.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The most reasons remembered at once; the reasons of a caller are a small fixed set, so this only bounds a bug.
const MAX_REASONS: usize = 64;

/// At most one warning per reason per interval, counting the ones suppressed in between.
pub(crate) struct WarnLimiter {
    interval: Duration,
    state: Mutex<HashMap<String, (Instant, u64)>>,
}

impl WarnLimiter {
    pub(crate) fn new(interval: Duration) -> Self {
        WarnLimiter { interval, state: Mutex::new(HashMap::new()) }
    }

    /// `Some(suppressed)` when a warning for `reason` may be written now (`suppressed` is the count since the last
    /// one), `None` when it must be dropped.
    pub(crate) fn allow(&self, reason: &str, now: Instant) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((last, suppressed)) = state.get_mut(reason) {
            if now.saturating_duration_since(*last) >= self.interval {
                let count = *suppressed;
                (*last, *suppressed) = (now, 0);
                return Some(count);
            }
            *suppressed += 1;
            return None;
        }
        if state.len() >= MAX_REASONS {
            return None;
        }
        state.insert(reason.to_string(), (now, 0));
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_warning_per_reason_and_interval_with_the_suppressed_count() {
        let limiter = WarnLimiter::new(Duration::from_secs(60));
        let start = Instant::now();
        assert_eq!(limiter.allow("a", start), Some(0));
        assert_eq!(limiter.allow("a", start + Duration::from_secs(1)), None);
        assert_eq!(limiter.allow("a", start + Duration::from_secs(2)), None);
        assert_eq!(limiter.allow("b", start + Duration::from_secs(2)), Some(0));
        assert_eq!(limiter.allow("a", start + Duration::from_secs(61)), Some(2));
        assert_eq!(limiter.allow("a", start + Duration::from_secs(62)), None);
    }

    #[test]
    fn the_number_of_remembered_reasons_is_bounded() {
        let limiter = WarnLimiter::new(Duration::from_secs(60));
        let now = Instant::now();
        for i in 0..MAX_REASONS {
            assert_eq!(limiter.allow(&format!("r{i}"), now), Some(0));
        }
        assert_eq!(limiter.allow("one-more", now), None);
    }
}
