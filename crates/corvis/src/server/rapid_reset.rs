//! Rapid reset (CVE-2023-44487) detection: requests cancelled before their response.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

use super::ConnState;

/// Rapid reset (CVE-2023-44487) detection. A connection is closed when the requests it cancelled before their
/// response, over the last [`CANCEL_WINDOW`] (the current window plus the previous one weighted by how much of it
/// still falls inside), exceed `max(`[`CANCEL_FLOOR`]`, completed / `[`COMPLETED_CREDIT`]`)` (completed requests over the
/// same span count for a quarter, so a proxy tolerates about 20 percent of aborts while an attacker pays four
/// completed requests per reset), or exceed `http2_max_cancelled_streams` whatever was completed.
const CANCEL_FLOOR: f64 = 500.0;
const COMPLETED_CREDIT: f64 = 4.0;
const CANCEL_WINDOW: Duration = Duration::from_secs(10);
/// After the GOAWAY of a flood, the connection is dropped when its streams have not ended by then.
pub(super) const FLOOD_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
pub(super) struct CancelWindow {
    start: Instant,
    pub(super) cancelled: u32,
    pub(super) completed: u32,
    prev_cancelled: u32,
    prev_completed: u32,
}

/// The requests a connection cancelled before their response, and the verdict when there are too many.
#[derive(Default)]
pub(super) struct CancelState {
    /// Requests cancelled and completed in the current and the previous window (rapid reset detection).
    pub(super) window: Mutex<Option<CancelWindow>>,
    /// Signalled when too many requests were cancelled before their response (rapid reset): the
    /// connection task closes the connection.
    pub(super) reset_flood: Notify,
    /// Set when the flood was detected: no further request of the connection reaches a handler.
    pub(super) flooded: AtomicBool,
    /// `http2_max_cancelled_streams`: cancellations over the window that close the connection whatever it completed.
    ceiling: u32,
}

impl CancelState {
    pub(super) fn new(ceiling: u32) -> Self {
        CancelState { ceiling, ..CancelState::default() }
    }

    /// Adds one request outcome at `now`; true when the connection has to be closed.
    fn tally(&self, now: Instant, cancelled: bool) -> bool {
        let mut guard = self.window.lock().unwrap_or_else(|e| e.into_inner());
        let w = guard.get_or_insert(CancelWindow { start: now, cancelled: 0, completed: 0, prev_cancelled: 0, prev_completed: 0 });
        let elapsed = now.saturating_duration_since(w.start);
        if elapsed >= CANCEL_WINDOW * 2 {
            *w = CancelWindow { start: now, cancelled: 0, completed: 0, prev_cancelled: 0, prev_completed: 0 };
        } else if elapsed >= CANCEL_WINDOW {
            w.prev_cancelled = w.cancelled;
            w.prev_completed = w.completed;
            w.cancelled = 0;
            w.completed = 0;
            w.start += CANCEL_WINDOW;
        }
        if cancelled {
            w.cancelled = w.cancelled.saturating_add(1);
        } else {
            w.completed = w.completed.saturating_add(1);
        }
        let inside = 1.0 - (now.saturating_duration_since(w.start).as_secs_f64() / CANCEL_WINDOW.as_secs_f64()).min(1.0);
        let cancelled_now = f64::from(w.cancelled) + f64::from(w.prev_cancelled) * inside;
        let completed_now = f64::from(w.completed) + f64::from(w.prev_completed) * inside;
        cancelled && (cancelled_now > CANCEL_FLOOR.max(completed_now / COMPLETED_CREDIT) || cancelled_now > f64::from(self.ceiling))
    }

    pub(super) fn record(&self, cancelled: bool) {
        if self.tally(Instant::now(), cancelled) {
            self.flooded.store(true, Ordering::SeqCst);
            self.reset_flood.notify_one();
        }
    }
}

pub(super) enum Outcome {
    /// The service future was dropped before producing a response.
    Pending,
    Completed,
    /// Refused without running because the connection was flooded: neither cancelled nor completed.
    Refused,
    /// The client reset the stream while its body was being read: nobody waits for the response.
    ClientAborted,
    /// The server dropped the response for exceeding `max_pending_response_bytes`: nothing was sent.
    Dropped,
    /// The response body records the outcome: completed when it ends, cancelled when it is dropped before.
    Body,
}

/// Counts a request as cancelled when its handler future is dropped before producing a response, or when the
/// client reset the stream while its body was read; a response body counts it itself (see [`GuardedBody`]).
pub(super) struct CancelWatch {
    pub(super) state: Arc<ConnState>,
    pub(super) outcome: Outcome,
}

impl Drop for CancelWatch {
    fn drop(&mut self) {
        if !matches!(self.outcome, Outcome::Refused | Outcome::Body) {
            self.state.cancels.record(!matches!(self.outcome, Outcome::Completed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `events` (seconds from a start, cancelled or not) and returns the time of the first close verdict.
    fn first_close(events: impl IntoIterator<Item = (f64, bool)>) -> Option<f64> {
        let state = ConnState::new(2000);
        let start = Instant::now();
        events.into_iter().find(|&(t, cancelled)| state.cancels.tally(start + Duration::from_secs_f64(t), cancelled)).map(|(t, _)| t)
    }

    #[test]
    fn cancel_floor_closes_a_pure_flood() {
        let flood = (0..2000).map(|i| (i as f64 * 0.001, true));
        assert_eq!(first_close(flood), Some(0.5));
    }

    #[test]
    fn about_a_thousand_cancels_do_not_fit_across_a_window_edge() {
        // 500 at the end of the first window, 500 at the start of the next: the weighted count reaches the floor.
        let mut events: Vec<(f64, bool)> = (0..500).map(|i| (9.9 + i as f64 * 0.0001, true)).collect();
        events.extend((0..500).map(|i| (10.0 + i as f64 * 0.0001, true)));
        let closed = first_close(events).expect("closed");
        assert!(closed < 10.06, "{closed}");
    }

    #[test]
    fn one_cheap_completed_request_per_reset_does_not_hide_a_flood() {
        let events = (0..20_000).map(|i| (i as f64 * 0.0005, i % 2 == 0));
        let closed = first_close(events).expect("closed");
        assert!(closed < 1.5, "{closed}");
    }

    #[test]
    fn the_ceiling_closes_a_connection_that_completes_plenty() {
        // 5,000 requests/s with 10% cancelled (500 per second): within the proportional rule, over the ceiling.
        let events = (0..50_000).map(|i| (i as f64 / 5000.0, i % 10 == 0));
        let closed = first_close(events).expect("closed");
        assert!((3.5..4.5).contains(&closed), "{closed}");
    }

    #[test]
    fn a_proxy_with_aborts_within_the_credit_is_not_closed() {
        // 5,000 requests/s with 10% cancelled would trip the ceiling; 150 per second (3%) does not.
        let events = (0..150_000).map(|i| (i as f64 / 5000.0, i % 33 == 0));
        assert_eq!(first_close(events), None);
    }

    #[test]
    fn a_proxy_that_completes_more_than_it_cancels_is_not_closed() {
        // 5,000 requests/s for 30 s with 2% cancelled (100 per second, 3,000 in total).
        let events = (0..150_000).map(|i| (i as f64 / 5000.0, i % 50 == 0));
        assert_eq!(first_close(events), None);
    }

    #[test]
    fn an_old_burst_is_forgotten() {
        let mut events: Vec<(f64, bool)> = (0..400).map(|i| (i as f64 * 0.001, true)).collect();
        events.extend((0..400).map(|i| (25.0 + i as f64 * 0.001, true)));
        assert_eq!(first_close(events), None);
    }
}
