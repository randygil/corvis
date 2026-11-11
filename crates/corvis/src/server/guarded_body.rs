//! The response body as the server hands it to hyper: in-flight accounting, HTTP/2 slices, the budget charge and the
//! send-timeout clock.

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use bytes::Bytes;
use hyper::body::{Frame, SizeHint};

use super::watchdog::BodyClock;
use super::{ConnState, ResBody};

/// The requests in flight on a connection, which its IO reads for the idle timeouts and the framing check.
#[derive(Default)]
pub(super) struct Activity {
    /// Requests whose response has not been fully written yet.
    pub(super) inflight: AtomicUsize,
    /// Bumped when a request starts or ends.
    pub(super) generation: AtomicU64,
    /// Requests whose response is done (fully written or dropped). The framing check compares it
    /// with the requests it forwarded to know when a connection has nothing in flight.
    pub(super) finished: AtomicU64,
    /// Task waiting on the IO, woken when the last in-flight request ends.
    pub(super) waker: Mutex<Option<Waker>>,
}

/// Keeps a request in flight until its response body is dropped (fully written or aborted).
pub(super) struct InflightGuard(pub(super) Arc<ConnState>);

impl InflightGuard {
    pub(super) fn new(state: Arc<ConnState>) -> Self {
        state.activity.inflight.fetch_add(1, Ordering::SeqCst);
        state.activity.generation.fetch_add(1, Ordering::SeqCst);
        InflightGuard(state)
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.activity.inflight.fetch_sub(1, Ordering::SeqCst);
        self.0.activity.finished.fetch_add(1, Ordering::SeqCst);
        self.0.activity.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(w) = self.0.activity.waker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            w.wake();
        }
    }
}

/// Largest data frame of an HTTP/2 response body (the default `SETTINGS_MAX_FRAME_SIZE`).
const H2_SLICE: usize = 16 * 1024;
/// What hyper may buffer per HTTP/2 stream (`max_send_buf_size`; its default is 400 KiB).
pub(super) const H2_SEND_BUF: usize = 32 * 1024;
/// h2 can still hold this much of a stream after its body ended: the send buffer, the slice that was handed over
/// when there was room, and the one data frame its codec keeps after taking it out of the stream's queue (it is not
/// counted in the buffer). Slices that may be part of it are copied, so they do not keep the whole response alive.
const H2_TAIL: usize = H2_SEND_BUF + 2 * H2_SLICE;

/// Cuts the first [`H2_SLICE`] bytes off `data` for hyper. A slice that may outlive the body (fewer than
/// [`H2_TAIL`] bytes of the frame remain after it) is copied into an allocation of its own.
fn cut_slice(data: &mut Bytes) -> Bytes {
    let part = data.split_to(H2_SLICE.min(data.len()));
    if data.is_empty() {
        // An empty split still points into the allocation of the frame.
        *data = Bytes::new();
    }
    if data.len() < H2_TAIL { Bytes::copy_from_slice(&part) } else { part }
}

/// A response body that releases its [`InflightGuard`] when hyper drops it, and keeps a [`BodyClock`] so that the
/// watchdog of the connection can tell when the client does not let it move (`send_timeout`).
pub(super) struct GuardedBody {
    inner: ResBody,
    clock: Arc<BodyClock>,
    /// HTTP/2: data frames are handed to hyper in slices of [`H2_SLICE`], so the body does not finish (and leave the
    /// watchdog's sight) while a window of a few bytes holds back megabytes already given to h2.
    slice: bool,
    /// What is left of a data frame that was cut.
    rest: Bytes,
    /// Hyper has polled the body at least once.
    polled: bool,
    guard: InflightGuard,
    /// Bytes this body adds to the connection's pending response bytes right now (`max_pending_response_bytes`).
    charge: usize,
    /// In-memory body: its whole size stays charged until it ends or is dropped. Otherwise the charge follows `rest`.
    fixed: bool,
    /// `max_pending_response_bytes` is on.
    track: bool,
    /// The body reported its end (or had nothing to send).
    ended: bool,
    /// A data frame was handed to hyper.
    sent_data: bool,
    /// The body records the outcome of its request for the rapid-reset rule: completed when it ended, cancelled when
    /// it is dropped before.
    record: bool,
}

impl GuardedBody {
    #[cfg(test)]
    pub(super) fn new(inner: ResBody, guard: InflightGuard, http2: bool) -> Self {
        Self::with_charge(inner, guard, http2, None, false)
    }

    pub(super) fn with_charge(inner: ResBody, guard: InflightGuard, http2: bool, charge: Option<usize>, record: bool) -> Self {
        let clock = Arc::new(BodyClock::default());
        clock.wait_for_hyper(&guard.0.clocks.epoch);
        guard.0.clocks.bodies.lock().unwrap_or_else(|e| e.into_inner()).push(clock.clone());
        guard.0.clocks.changed.notify_one();
        GuardedBody {
            inner,
            clock,
            slice: http2,
            rest: Bytes::new(),
            polled: false,
            guard,
            charge: charge.unwrap_or(0),
            fixed: charge.is_some_and(|c| c > 0),
            track: charge.is_some(),
            ended: false,
            sent_data: false,
            record,
        }
    }

    /// Follows the part of a cut frame not yet handed to hyper (bodies of unknown size).
    fn charge_rest(&mut self) {
        if !self.track || self.fixed {
            return;
        }
        let now = self.rest.len();
        if now > self.charge {
            self.guard.0.budget.held.lock().unwrap_or_else(|e| e.into_inner()).pending += now - self.charge;
        } else if now < self.charge {
            self.guard.0.budget.release(self.charge - now);
        }
        self.charge = now;
    }
}

impl Drop for GuardedBody {
    fn drop(&mut self) {
        let ended = self.ended || hyper::body::Body::is_end_stream(&*self);
        let charge = std::mem::take(&mut self.charge);
        if self.slice || !self.track {
            self.guard.0.budget.release(charge);
        } else if charge > 0 {
            // HTTP/1 hyper queued the whole frame: the memory is held until the socket accepts it.
            self.guard.0.budget.defer_release(charge);
        }
        if self.record {
            self.guard.0.cancels.record(!ended);
        }
        let mut clocks = self.guard.0.clocks.bodies.lock().unwrap_or_else(|e| e.into_inner());
        clocks.retain(|c| !Arc::ptr_eq(c, &self.clock));
    }
}

impl hyper::body::Body for GuardedBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = &mut *self;
        // Hyper asks for the next frame only once it took the previous one: the connection moves. The first poll is
        // not that: it takes nothing.
        if std::mem::replace(&mut this.polled, true) && this.clock.since(&this.guard.0.clocks.epoch).is_some() {
            this.guard.0.clocks.note_progress();
        }
        if !this.rest.is_empty() {
            let part = cut_slice(&mut this.rest);
            this.sent_data = true;
            this.charge_rest();
            this.clock.wait_for_hyper(&this.guard.0.clocks.epoch);
            return Poll::Ready(Some(Ok(Frame::data(part))));
        }
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Pending => {
                this.clock.wait_for_application();
                Poll::Pending
            }
            Poll::Ready(Some(Ok(frame))) => {
                this.clock.wait_for_hyper(&this.guard.0.clocks.epoch);
                this.sent_data |= frame.is_data();
                if this.slice {
                    match frame.into_data() {
                        Ok(mut data) if data.len() > H2_SLICE => {
                            let first = cut_slice(&mut data);
                            this.rest = data;
                            this.charge_rest();
                            return Poll::Ready(Some(Ok(Frame::data(first))));
                        }
                        Ok(data) => return Poll::Ready(Some(Ok(Frame::data(data)))),
                        Err(frame) => return Poll::Ready(Some(Ok(frame))),
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            other => {
                this.clock.wait_for_application();
                if matches!(other, Poll::Ready(None)) {
                    this.ended = true;
                }
                other
            }
        }
    }
    fn is_end_stream(&self) -> bool {
        let drained = self.rest.is_empty() && self.inner.is_end_stream();
        // Over HTTP/2 hyper asks this right after taking a frame, before h2 has room for it: a response that is a
        // single small frame would count as finished while no byte left. It ends when `poll_frame` says so (at the
        // cost of an empty DATA frame with END_STREAM).
        if self.slice && self.sent_data { drained && self.ended } else { drained }
    }
    fn size_hint(&self) -> SizeHint {
        let mut hint = self.inner.size_hint();
        let rest = self.rest.len() as u64;
        if rest > 0 {
            hint.set_lower(hint.lower().saturating_add(rest));
            if let Some(upper) = hint.upper() {
                hint.set_upper(upper.saturating_add(rest));
            }
        }
        hint
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    use http_body_util::BodyExt;

    use crate::server::response::{empty_body, full};

    pub(in crate::server) struct Script {
        pub(in crate::server) parts: std::collections::VecDeque<Option<Result<Frame<Bytes>, io::Error>>>,
        pub(in crate::server) pending_first: bool,
    }

    impl hyper::body::Body for Script {
        type Data = Bytes;
        type Error = io::Error;
        fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
            if std::mem::take(&mut self.pending_first) {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            Poll::Ready(self.parts.pop_front().flatten())
        }
    }

    pub(in crate::server) fn body_of(
        parts: Vec<Option<Result<Frame<Bytes>, io::Error>>>,
        pending_first: bool,
        http2: bool,
    ) -> (GuardedBody, Arc<ConnState>) {
        let state = Arc::new(ConnState::default());
        let inner = Script { parts: parts.into(), pending_first }.boxed_unsync();
        (GuardedBody::new(inner, InflightGuard::new(state.clone()), http2), state)
    }

    pub(in crate::server) fn poll_once(body: &mut GuardedBody) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        hyper::body::Body::poll_frame(Pin::new(body), &mut cx)
    }

    fn waiting(state: &ConnState) -> Vec<bool> {
        state.clocks.bodies.lock().unwrap().iter().map(|c| c.since(&state.clocks.epoch).is_some()).collect()
    }

    #[test]
    fn a_body_waits_for_hyper_until_it_waits_for_the_application() {
        let (mut body, state) = body_of(vec![Some(Ok(Frame::data(Bytes::from_static(b"a"))))], true, false);
        // Created, never polled: hyper has to take its first frame.
        assert_eq!(waiting(&state), vec![true]);
        assert!(poll_once(&mut body).is_pending());
        assert_eq!(waiting(&state), vec![false], "pending: it waits for the application");
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(waiting(&state), vec![true], "a frame was produced: hyper has to take it");
        assert!(matches!(poll_once(&mut body), Poll::Ready(None)));
        assert_eq!(waiting(&state), vec![false], "ended");
        drop(body);
        assert!(state.clocks.bodies.lock().unwrap().is_empty(), "the clock goes with the body");
        assert_eq!(state.activity.inflight.load(Ordering::SeqCst), 0);
    }

    pub(in crate::server) fn many_frames(n: usize) -> Vec<Option<Result<Frame<Bytes>, io::Error>>> {
        (0..n).map(|_| Some(Ok(Frame::data(Bytes::from_static(b"x"))))).collect()
    }

    #[test]
    fn http2_data_is_handed_over_in_slices_of_sixteen_kib() {
        let big = Bytes::from(vec![7u8; 40 * 1024]);
        let (mut body, _state) = body_of(vec![Some(Ok(Frame::data(big)))], false, true);
        let mut sizes = Vec::new();
        while let Poll::Ready(Some(Ok(frame))) = poll_once(&mut body) {
            sizes.push(frame.into_data().unwrap().len());
            if sizes.len() < 3 {
                assert!(!hyper::body::Body::is_end_stream(&body), "a body with data left must not report its end");
            }
        }
        assert_eq!(sizes, vec![16 * 1024, 16 * 1024, 8 * 1024]);
    }

    fn charged_body(
        parts: Vec<Option<Result<Frame<Bytes>, io::Error>>>,
        http2: bool,
        charge: Option<usize>,
        state: &Arc<ConnState>,
    ) -> GuardedBody {
        let inner = Script { parts: parts.into(), pending_first: false }.boxed_unsync();
        if let Some(c) = charge {
            state.budget.held.lock().unwrap().pending += c;
        }
        GuardedBody::with_charge(inner, InflightGuard::new(state.clone()), http2, charge, true)
    }

    fn pending(state: &ConnState) -> usize {
        state.budget.held.lock().unwrap().pending
    }

    #[test]
    fn an_in_memory_body_stays_charged_until_it_ends_or_is_dropped() {
        let state = Arc::new(ConnState::default());
        let mut body = charged_body(vec![Some(Ok(Frame::data(Bytes::from(vec![1u8; 100 * 1024]))))], true, Some(100 * 1024), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(pending(&state), 100 * 1024, "slices went out; the allocation is still alive");
        drop(body);
        assert_eq!(pending(&state), 0);
    }

    /// HTTP/1: hyper takes a whole frame into its write buffer, so the body ending does not free the memory; the
    /// flush of the socket (hyper flushes only with an empty write buffer) does.
    #[test]
    fn an_http1_body_stays_charged_until_hyper_flushes_the_socket() {
        let state = Arc::new(ConnState::default());
        let mut body = charged_body(vec![Some(Ok(Frame::data(Bytes::from(vec![1u8; 1000]))))], false, Some(1000), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(matches!(poll_once(&mut body), Poll::Ready(None)));
        drop(body);
        assert_eq!(pending(&state), 1000, "the data is still in hyper's write buffer");
        state.budget.flush_deferred();
        assert_eq!(pending(&state), 0);
        state.budget.flush_deferred();
        assert_eq!(pending(&state), 0, "a flush with nothing deferred changes nothing");
    }

    /// Bytes hyper drops after taking them (a body longer than its `Content-Length`) never reach the socket: the
    /// charge must not depend on counting them, or the connection waits for ever.
    #[test]
    fn an_http1_charge_is_released_even_when_hyper_wrote_fewer_bytes_than_it_took() {
        let state = Arc::new(ConnState::default());
        for _ in 0..1000 {
            let mut body = charged_body(vec![Some(Ok(Frame::data(Bytes::from(vec![1u8; 64 * 1024]))))], false, Some(64 * 1024), &state);
            assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
            drop(body);
            // Only ten bytes of it were written; hyper still flushes with its buffer empty.
            state.budget.flush_deferred();
            assert_eq!(pending(&state), 0);
        }
    }

    /// An HTTP/2 body of two bytes in one frame, as handlers of small JSON responses produce.
    fn full_body(state: &Arc<ConnState>) -> GuardedBody {
        state.budget.held.lock().unwrap().pending += 2;
        GuardedBody::with_charge(full(Bytes::from_static(b"{}")), InflightGuard::new(state.clone()), true, Some(2), true)
    }

    #[test]
    fn an_http2_response_that_fits_one_frame_is_not_finished_until_its_body_says_so() {
        let state = Arc::new(ConnState::default());
        let mut body = full_body(&state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(!hyper::body::Body::is_end_stream(&body), "the frame is with hyper but may not have left");
        // Dropped now (a client reset while h2 had no room): cancelled, not completed.
        drop(body);
        let w = state.cancels.window.lock().unwrap().unwrap();
        assert_eq!((w.cancelled, w.completed), (1, 0));
        let mut body = full_body(&state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(matches!(poll_once(&mut body), Poll::Ready(None)));
        assert!(hyper::body::Body::is_end_stream(&body));
        drop(body);
        let w = state.cancels.window.lock().unwrap().unwrap();
        assert_eq!((w.cancelled, w.completed), (1, 1));
    }

    #[test]
    fn an_empty_http2_body_is_finished_from_the_start() {
        let body = GuardedBody::with_charge(empty_body(), InflightGuard::new(Arc::new(ConnState::default())), true, Some(0), true);
        assert!(hyper::body::Body::is_end_stream(&body));
    }

    #[test]
    fn a_streamed_body_charges_only_what_it_has_not_handed_over() {
        let state = Arc::new(ConnState::default());
        let frame = Frame::data(Bytes::from(vec![1u8; 64 * 1024]));
        let mut body = charged_body(vec![Some(Ok(frame))], true, Some(0), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(pending(&state), 48 * 1024);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(pending(&state), 32 * 1024);
        drop(body);
        assert_eq!(pending(&state), 0, "dropping releases the rest");
    }

    #[test]
    fn an_uncharged_body_leaves_the_budget_alone() {
        let state = Arc::new(ConnState::default());
        let frame = Frame::data(Bytes::from(vec![1u8; 64 * 1024]));
        let mut body = charged_body(vec![Some(Ok(frame))], true, None, &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(pending(&state), 0);
    }

    /// Whether `part` lies inside the allocation of `outer`.
    fn inside(outer: &[u8], part: &Bytes) -> bool {
        let (o, p) = (outer.as_ptr() as usize, part.as_ptr() as usize);
        p >= o && p + part.len() <= o + outer.len()
    }

    #[test]
    fn the_last_slices_of_an_http2_body_are_copies() {
        let big = Bytes::from(vec![7u8; 1024 * 1024]);
        let original = big.clone();
        let (mut body, _state) = body_of(vec![Some(Ok(Frame::data(big)))], false, true);
        let mut slices = Vec::new();
        while let Poll::Ready(Some(Ok(frame))) = poll_once(&mut body) {
            slices.push(frame.into_data().unwrap());
        }
        assert_eq!(slices.len(), 64);
        let shared: Vec<bool> = slices.iter().map(|s| inside(&original, s)).collect();
        assert!(shared[..60].iter().all(|s| *s), "the earlier slices keep pointing into the response");
        assert!(shared[60..].iter().all(|s| !*s), "the last 64 KiB (send buffer, one slice, one frame in the codec) are copies");
        assert!(slices.iter().all(|s| s.iter().all(|b| *b == 7)));
    }

    #[test]
    fn a_body_records_completed_when_it_ends_and_cancelled_when_dropped_before() {
        let state = Arc::new(ConnState::new(2000));
        let recorded = |state: &ConnState| {
            let w = state.cancels.window.lock().unwrap();
            w.map_or((0, 0), |w| (w.cancelled, w.completed))
        };
        let mut body = charged_body(vec![Some(Ok(Frame::data(Bytes::from_static(b"abc"))))], false, Some(3), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(matches!(poll_once(&mut body), Poll::Ready(None)));
        drop(body);
        assert_eq!(recorded(&state), (0, 1));
        let big = Bytes::from(vec![7u8; 100 * 1024]);
        let mut body = charged_body(vec![Some(Ok(Frame::data(big)))], true, Some(100 * 1024), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        drop(body);
        assert_eq!(recorded(&state), (1, 1), "dropped with data left: cancelled");
        let body = GuardedBody::with_charge(empty_body(), InflightGuard::new(state.clone()), false, Some(0), true);
        drop(body);
        assert_eq!(recorded(&state), (1, 2), "a body with nothing to send is completed");
    }

    #[test]
    fn http1_data_is_not_sliced() {
        let big = Bytes::from(vec![7u8; 40 * 1024]);
        let (mut body, _state) = body_of(vec![Some(Ok(Frame::data(big)))], false, false);
        let Poll::Ready(Some(Ok(frame))) = poll_once(&mut body) else { panic!("a frame") };
        assert_eq!(frame.into_data().unwrap().len(), 40 * 1024);
    }
}
