//! The response budget of a connection (`max_pending_response_bytes`): admission of handlers and the bytes charged.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;

use super::ConnState;

/// What a connection holds of the responses of its handlers (design of `e0-cap-pending-response-bytes`).
#[derive(Default)]
pub(super) struct Budget {
    /// Bytes of response bodies that have neither ended nor been dropped.
    pub(super) pending: usize,
    /// Handlers admitted that have not returned yet, each with a reserved share.
    running: usize,
    /// The reset for exceeding the cap was logged already.
    warned: bool,
    /// The refusal of a request that waited too long was logged already.
    warned_wait: bool,
    /// Requests waiting for room.
    waiting: usize,
    /// Share and cap of the last admission attempt (all requests of a connection use the same ones).
    share: usize,
    cap: usize,
}

/// What to do with the response a handler returned.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    Keep,
    Reset,
}

impl Budget {
    /// Reserves `share` for a handler when `pending + reservations + share` fits `cap`, or when the connection holds
    /// nothing at all.
    fn try_admit(&mut self, share: usize, cap: usize) -> bool {
        (self.share, self.cap) = (share, cap);
        let idle = self.pending == 0 && self.running == 0;
        let fits = self.pending.saturating_add(self.running.saturating_add(1).saturating_mul(share)) <= cap;
        if idle || fits {
            self.running += 1;
        }
        idle || fits
    }

    /// A handler returned a response charging `charge`: its reservation ends and the response is kept (and charged)
    /// unless, over HTTP/2, the connection already holds bytes and the response would take it above `cap`.
    fn finish_handler(&mut self, charge: usize, cap: usize, http2: bool) -> Verdict {
        self.running = self.running.saturating_sub(1);
        if http2 && self.pending > 0 && self.pending.saturating_add(charge) > cap {
            return Verdict::Reset;
        }
        self.pending = self.pending.saturating_add(charge);
        Verdict::Keep
    }

    fn release(&mut self, charge: usize) {
        self.pending = self.pending.saturating_sub(charge);
    }

    /// A request is waiting and would be admitted now: only then is waking the waiters worth the work.
    fn should_wake(&self) -> bool {
        let idle = self.pending == 0 && self.running == 0;
        self.waiting > 0 && (idle || self.pending.saturating_add(self.running.saturating_add(1).saturating_mul(self.share)) <= self.cap)
    }
}

/// What one connection holds of its responses, and the requests that wait for room.
#[derive(Default)]
pub(super) struct ResponseBudget {
    /// Response bytes the connection holds (`max_pending_response_bytes`).
    pub(super) held: Mutex<Budget>,
    /// Signalled whenever pending bytes or running handlers drop, so waiting requests look again.
    freed: Notify,
    /// HTTP/1: charges of bodies hyper already took into its write buffer, released when hyper flushes the socket
    /// (it only does so with its write buffer empty).
    deferred: AtomicUsize,
}

impl ResponseBudget {
    /// HTTP/1: hyper took a body into its write buffer; the memory is held until the buffer is flushed.
    pub(super) fn defer_release(&self, charge: usize) {
        self.deferred.fetch_add(charge, Ordering::Relaxed);
    }

    /// Hyper flushed the socket, which it only does with its write buffer empty: what it took is out.
    pub(super) fn flush_deferred(&self) {
        let charge = self.deferred.swap(0, Ordering::Relaxed);
        self.release(charge);
    }

    pub(super) fn release(&self, charge: usize) {
        if charge > 0 {
            let wake = {
                let mut b = self.held.lock().unwrap_or_else(|e| e.into_inner());
                b.release(charge);
                b.should_wake()
            };
            if wake {
                self.freed.notify_waiters();
            }
        }
    }

    /// Wakes the waiting requests when one of them could be admitted.
    fn wake_if_room(&self) {
        if self.held.lock().unwrap_or_else(|e| e.into_inner()).should_wake() {
            self.freed.notify_waiters();
        }
    }

    /// Logs, once per connection, that a request waited too long for room.
    pub(super) fn warn_wait_once(&self, wait: Duration) {
        let first = !std::mem::replace(&mut self.held.lock().unwrap_or_else(|e| e.into_inner()).warned_wait, true);
        if first {
            crate::logger::Logger::warn_with(
                format!(
                    "A request waited more than {wait:?} for room in the connection's response budget \
                     (ServerOptions::max_pending_response_bytes) and its HTTP/2 stream was refused (REFUSED_STREAM); its handler never ran. Raise \
                     ServerOptions::max_pending_response_wait or the byte limit if clients read large responses slowly. \
                     Further refusals on this connection are not logged."
                ),
                "HttpServer",
            );
        }
    }
}

impl ConnState {
    /// Waits until the budget has room for a handler with this `share`; the reservation lasts until the handler
    /// returns or the returned value is dropped.
    pub(super) async fn admit(self: &Arc<Self>, share: usize, cap: usize) -> Admission {
        let mut waiting: Option<Waiting> = None;
        loop {
            let notified = self.budget.freed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut b = self.budget.held.lock().unwrap_or_else(|e| e.into_inner());
                if b.try_admit(share, cap) {
                    return Admission { state: Some(self.clone()) };
                }
                if waiting.is_none() {
                    b.waiting += 1;
                    waiting = Some(Waiting(self.clone()));
                }
            }
            notified.await;
        }
    }
}

/// A request counted as waiting for room, until it is admitted or gives up.
struct Waiting(Arc<ConnState>);

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut b = self.0.budget.held.lock().unwrap_or_else(|e| e.into_inner());
        b.waiting = b.waiting.saturating_sub(1);
    }
}

/// A reserved share of the budget for a running handler.
pub(super) struct Admission {
    state: Option<Arc<ConnState>>,
}

impl Admission {
    /// The handler returned: end the reservation and decide about its response.
    pub(super) fn finish(mut self, charge: usize, cap: usize, http2: bool) -> Verdict {
        let state = self.state.take().expect("an admission finishes once");
        let (verdict, first) = {
            let mut b = state.budget.held.lock().unwrap_or_else(|e| e.into_inner());
            let verdict = b.finish_handler(charge, cap, http2);
            let first = verdict == Verdict::Reset && !std::mem::replace(&mut b.warned, true);
            (verdict, first)
        };
        state.budget.wake_if_room();
        if first {
            crate::logger::Logger::warn_with(
                format!(
                    "A response was dropped and its HTTP/2 stream reset (ENHANCE_YOUR_CALM): the connection holds more than \
                     {cap} bytes of pending responses (ServerOptions::max_pending_response_bytes). Raise the option or stream \
                     large bodies. Further resets on this connection are not logged."
                ),
                "HttpServer",
            );
        }
        verdict
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let mut b = state.budget.held.lock().unwrap_or_else(|e| e.into_inner());
            b.running = b.running.saturating_sub(1);
            drop(b);
            state.budget.wake_if_room();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handler_is_admitted_from_idle_whatever_the_share() {
        let mut b = Budget::default();
        assert!(b.try_admit(10 * 1024 * 1024, 1024));
        assert_eq!(b.running, 1);
    }

    #[test]
    fn admission_reserves_a_share_per_running_handler() {
        let mut b = Budget::default();
        // cap 100, share 25: four handlers fit, the fifth waits.
        for _ in 0..4 {
            assert!(b.try_admit(25, 100));
        }
        assert!(!b.try_admit(25, 100));
        assert_eq!(b.running, 4);
        // One returns a small response: its reservation is replaced by the charge.
        assert_eq!(b.finish_handler(10, 100, true), Verdict::Keep);
        assert_eq!((b.running, b.pending), (3, 10));
        assert!(!b.try_admit(25, 100), "10 pending + 3 reserved + its own 25 does not fit");
        b.release(10);
        assert!(b.try_admit(25, 100), "3 reserved + its own 25 fits again");
    }

    #[test]
    fn pending_bytes_alone_block_admission_and_a_release_frees_it() {
        let mut b = Budget::default();
        assert!(b.try_admit(10, 100));
        assert_eq!(b.finish_handler(95, 100, true), Verdict::Keep);
        assert!(!b.try_admit(10, 100), "95 pending leave no room for a share of 10");
        b.release(95);
        assert!(b.try_admit(10, 100));
    }

    #[test]
    fn a_response_is_kept_when_nothing_is_pending_and_reset_only_over_http2_above_the_cap() {
        let mut b = Budget::default();
        assert!(b.try_admit(10, 100));
        assert_eq!(b.finish_handler(1000, 100, true), Verdict::Keep, "pending was 0: one response is always sent");
        assert_eq!(b.pending, 1000);
        let mut b = Budget { pending: 90, running: 1, ..Budget::default() };
        assert_eq!(b.finish_handler(20, 100, true), Verdict::Reset);
        assert_eq!((b.pending, b.running), (90, 0), "a reset response is not charged");
        let mut b = Budget { pending: 90, running: 1, ..Budget::default() };
        assert_eq!(b.finish_handler(20, 100, false), Verdict::Keep, "HTTP/1 never resets");
    }

    #[test]
    fn responses_within_their_share_are_never_reset() {
        // cap 1000 over 10 handlers: share 100. Ten handlers each return exactly their share.
        let mut b = Budget::default();
        for _ in 0..10 {
            assert!(b.try_admit(100, 1000));
        }
        for _ in 0..10 {
            assert_eq!(b.finish_handler(100, 1000, true), Verdict::Keep);
        }
        assert_eq!(b.pending, 1000);
    }

    #[test]
    fn a_dropped_admission_gives_its_reservation_back() {
        let state = Arc::new(ConnState::default());
        let admission = futures::executor::block_on(state.admit(10, 100));
        assert_eq!(state.budget.held.lock().unwrap().running, 1);
        drop(admission);
        assert_eq!(state.budget.held.lock().unwrap().running, 0);
    }

    #[test]
    fn waiters_are_woken_only_when_one_of_them_could_be_admitted() {
        let mut b = Budget { pending: 95, running: 0, waiting: 0, share: 10, cap: 100, ..Budget::default() };
        assert!(!b.should_wake(), "nobody waits");
        b.waiting = 3;
        assert!(!b.should_wake(), "95 pending leave no room for a share of 10");
        b.release(10);
        assert!(b.should_wake());
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_that_waits_too_long_gives_up_without_a_reservation() {
        let state = Arc::new(ConnState::default());
        let held = state.admit(60, 100).await;
        state.budget.held.lock().unwrap().pending = 100;
        let waited = tokio::time::timeout(Duration::from_secs(5), state.admit(10, 100)).await;
        assert!(waited.is_err(), "no room: the wait times out");
        {
            let b = state.budget.held.lock().unwrap();
            assert_eq!((b.running, b.waiting), (1, 0), "the abandoned wait holds nothing");
        }
        drop(held);
    }
}
