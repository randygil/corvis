//! Plain RFC 6455 WebSockets on the application's HTTP server, modelled after the
//! [`ws`](https://github.com/websockets/ws) package: `new WebSocketServer({ noServer: true })`
//! + `wss.handleUpgrade(req, socket, head, cb)` from an `upgrade` listener
//! (see [`crate::http_adapter`]).
//!
//! Frames are raw text/binary messages (no `{ event, data }` gateway protocol).
//!
//! ```ignore
//! let wss = WebSocketServer::new(WebSocketServerOptions::default());
//! server.on_upgrade(move |req| {
//!     if req.pathname() != "/ws/printer" {
//!         req.destroy();
//!         return;
//!     }
//!     wss.handle_upgrade(&req, |ws| {
//!         let peer = ws.clone();
//!         ws.on_message(move |msg| {
//!             let _ = peer.send(format!("echo: {msg}"));
//!         });
//!         ws.on_close(|code, reason| println!("closed {code} {reason}"));
//!     });
//! });
//! ```

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::{Sink, SinkExt, Stream, StreamExt};
use http::{HeaderMap, HeaderValue, StatusCode};
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};

use crate::http::{Request, Response};
use crate::http_adapter::{UpgradeRequest, UpgradedIo};
use crate::logger::Logger;

/// Default limit of bytes queued for sending on one connection (`max_buffered_amount`).
pub const DEFAULT_MAX_BUFFERED_AMOUNT: usize = 8 * 1024 * 1024;
/// Default time a closing connection waits for the peer's close frame before the TCP
/// connection is destroyed (`ws` uses 30 s).
pub const DEFAULT_CLOSE_TIMEOUT: Duration = Duration::from_secs(30);
/// Default limit of received bytes waiting to be read from [`WebSocket::messages`].
pub const DEFAULT_MAX_UNREAD_BYTES: usize = 16 * 1024 * 1024;

/// Close code sent when a peer does not read fast enough and the send buffer is full.
pub(crate) const CLOSE_POLICY_VIOLATION: u16 = 1008;
/// Close code sent when a listener or hook panicked.
pub(crate) const CLOSE_INTERNAL_ERROR: u16 = 1011;

pub(crate) type Ack = oneshot::Sender<Result<(), WsError>>;

/// Why a frame could not be queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PushError {
    /// The connection is closing or closed.
    Closed,
    /// The frame does not fit in the send buffer (`max_buffered_amount`).
    Full,
}

/// What a queue holds: it reports the bytes an item counts against the queue's limit.
pub(crate) trait QueueItem {
    fn size(&self) -> usize;
}

impl QueueItem for Message {
    fn size(&self) -> usize {
        self.len() + 16
    }
}

struct OutState<M> {
    items: VecDeque<(M, usize, Option<Ack>)>,
    queued: usize,
    writing: usize,
    closed: bool,
}

impl<M> Default for OutState<M> {
    fn default() -> Self {
        OutState { items: VecDeque::new(), queued: 0, writing: 0, closed: false }
    }
}

/// The outgoing queue of a connection, bounded in bytes. A single writer task drains it,
/// so a peer that does not read only blocks its own writer; [`OutQueue::buffered`] is the
/// `bufferedAmount` of `ws` (queued bytes plus the frame being written).
///
/// It is generic over its item so Engine.IO sessions reuse the same bound: the WebSocket
/// queues are `OutQueue<Message>` (the default), an Engine.IO session's outbox queues packets.
pub(crate) struct OutQueue<M: QueueItem = Message> {
    state: Mutex<OutState<M>>,
    notify: Notify,
    /// Woken with `notify_waiters` on every change, for consumers that only look at the queue
    /// (a pending long-poll) and must not take the single wake-up of the writer.
    watchers: Notify,
    limit: usize,
}

impl<M: QueueItem> OutQueue<M> {
    pub(crate) fn new(limit: usize) -> Self {
        OutQueue { state: Mutex::new(OutState::default()), notify: Notify::new(), watchers: Notify::new(), limit: limit.max(1) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, OutState<M>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wake(&self) {
        self.notify.notify_one();
        self.watchers.notify_waiters();
    }

    /// Queues an item. An item is always accepted when nothing is pending (so one message
    /// larger than the limit can still be sent); otherwise the total must stay within it.
    pub(crate) fn push(&self, m: M, ack: Option<Ack>) -> Result<(), PushError> {
        let size = m.size();
        let mut s = self.lock();
        if s.closed {
            return Err(PushError::Closed);
        }
        let pending = s.queued + s.writing;
        if pending > 0 && pending + size > self.limit {
            return Err(PushError::Full);
        }
        s.items.push_back((m, size, ack));
        s.queued += size;
        drop(s);
        self.wake();
        Ok(())
    }

    /// Queues several items as one unit under a single lock: nothing else can be queued between them, and the group
    /// is accepted or refused as a whole by the rule of [`push`](Self::push) applied to the sum of the sizes.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn push_group(&self, items: Vec<M>) -> Result<(), PushError> {
        if items.is_empty() {
            return Ok(());
        }
        let sizes: Vec<usize> = items.iter().map(|m| m.size()).collect();
        let total: usize = sizes.iter().sum();
        let mut s = self.lock();
        if s.closed {
            return Err(PushError::Closed);
        }
        let pending = s.queued + s.writing;
        if pending > 0 && pending + total > self.limit {
            return Err(PushError::Full);
        }
        for (m, size) in items.into_iter().zip(sizes) {
            s.items.push_back((m, size, None));
        }
        s.queued += total;
        drop(s);
        self.wake();
        Ok(())
    }

    /// Rejects any later item. The pending ones stay queued for the writer, or are dropped with
    /// `discard`. Returns `false` if the queue was already closed.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn seal(&self, discard: bool) -> bool {
        let mut s = self.lock();
        if s.closed && !discard {
            return false;
        }
        if discard {
            s.items.clear();
            s.queued = 0;
        }
        s.closed = true;
        drop(s);
        self.wake();
        true
    }

    /// `true` once sealed and every queued item was taken.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn is_drained(&self) -> bool {
        let s = self.lock();
        s.closed && s.items.is_empty()
    }

    /// Takes queued items in order while `cost` of the taken ones stays within `budget`; always at least one
    /// when any is queued. Taken items no longer count against the limit.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn take_batch(&self, budget: usize, mut cost: impl FnMut(&M) -> usize) -> Vec<M> {
        let mut s = self.lock();
        let mut taken = Vec::new();
        let mut spent = 0usize;
        while let Some((m, _, _)) = s.items.front() {
            let c = cost(m);
            if !taken.is_empty() && spent + c > budget {
                break;
            }
            spent += c;
            let (m, size, _) = s.items.pop_front().expect("front exists");
            s.queued -= size;
            taken.push(m);
        }
        taken
    }

    /// Resolves when an item is queued or the queue is sealed, without taking anything.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) async fn wait_ready(&self) {
        loop {
            let notified = self.watchers.notified();
            {
                let s = self.lock();
                if !s.items.is_empty() || s.closed {
                    return;
                }
            }
            notified.await;
        }
    }

    /// Next item to write; `None` once closed and drained.
    pub(crate) async fn next(&self) -> Option<(M, usize, Option<Ack>)> {
        loop {
            let notified = self.notify.notified();
            {
                let mut s = self.lock();
                if let Some((m, size, ack)) = s.items.pop_front() {
                    s.queued -= size;
                    s.writing += size;
                    return Some((m, size, ack));
                }
                if s.closed {
                    return None;
                }
            }
            notified.await;
        }
    }

    pub(crate) fn written(&self, size: usize) {
        let mut s = self.lock();
        s.writing = s.writing.saturating_sub(size);
    }

    pub(crate) fn buffered(&self) -> usize {
        let s = self.lock();
        s.queued + s.writing
    }

    /// Drops every pending frame (their acks fail) and rejects new ones.
    pub(crate) fn shutdown(&self) {
        let mut s = self.lock();
        s.closed = true;
        s.items.clear();
        s.queued = 0;
        s.writing = 0;
        drop(s);
        self.wake();
    }
}

impl OutQueue<Message> {
    /// Queues a close frame after the pending frames (or instead of them with `discard`) and
    /// rejects any later frame. Returns `false` if the queue was already closed.
    pub(crate) fn close(&self, frame: Option<CloseFrame<'static>>, discard: bool) -> bool {
        let mut s = self.lock();
        if s.closed && !discard {
            return false;
        }
        if discard {
            s.items.clear();
            s.queued = 0;
        }
        let m = Message::Close(frame);
        let size = m.size();
        s.items.push_back((m, size, None));
        s.queued += size;
        s.closed = true;
        drop(s);
        self.wake();
        true
    }
}

/// Why [`write_loop`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteEnd {
    /// A close frame was written or the queue was shut down.
    Closed,
    /// A write failed at the socket (a peer that accepts nothing hits the server's `send_timeout` as `TimedOut`).
    Failed(std::io::ErrorKind),
    /// The protocol refused a frame (for instance a send after the closing handshake began); the socket is fine.
    Refused,
}

/// Writes the frames of `queue` to `sink` until a close frame is written, a write fails or
/// the queue is shut down.
pub(crate) async fn write_loop<S>(sink: S, queue: Arc<OutQueue>) -> WriteEnd
where
    S: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    write_loop_with(sink, queue, |m| m).await.0
}

/// [`write_loop`] for a queue of any item, with `encode` turning each item into the frame to write. A queue that
/// ends (sealed and drained) ends the loop with [`WriteEnd::Closed`], like a written close frame. The sink is
/// handed back so the caller can still write a close frame after a queue that carries no close item.
pub(crate) async fn write_loop_with<S, M, F>(mut sink: S, queue: Arc<OutQueue<M>>, encode: F) -> (WriteEnd, S)
where
    S: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    M: QueueItem,
    F: Fn(M) -> Message,
{
    while let Some((m, size, ack)) = queue.next().await {
        let m = encode(m);
        let close = matches!(m, Message::Close(_));
        let result = sink.send(m).await;
        queue.written(size);
        let kind = match &result {
            Ok(()) => None,
            Err(tokio_tungstenite::tungstenite::Error::Io(e)) => Some(e.kind()),
            Err(_) => None,
        };
        let refused = result.is_err() && kind.is_none();
        if let Some(a) = ack {
            let _ = a.send(result.map_err(|e| WsError::new(e.to_string(), None)));
        }
        if let Some(kind) = kind {
            return (WriteEnd::Failed(kind), sink);
        }
        if refused {
            return (WriteEnd::Refused, sink);
        }
        if close {
            break;
        }
    }
    (WriteEnd::Closed, sink)
}

/// Runs a listener or hook, turning a panic into an error log. Returns `false` if it panicked.
pub(crate) fn guarded(context: &str, f: impl FnOnce()) -> bool {
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => true,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_string());
            Logger::new("WebSocket").error(format!("{context} panicked: {msg}"));
            false
        }
    }
}

/// Bytes received and not yet taken from the `messages()` streams; the read loop stops
/// reading from the socket while it is above the limit (TCP backpressure).
///
/// The Engine.IO sessions chain their gauge to one shared by the whole engine (`parent`): a session is over its
/// limit when either is.
pub(crate) struct UnreadGauge {
    bytes: AtomicUsize,
    pub(crate) notify: Notify,
    limit: usize,
    /// `true`: a release wakes every waiter (a held `POST` and the WebSocket reader can both wait). The gateway
    /// has a single waiter, the read loop, and keeps `notify_one`, which cannot lose a release that happens
    /// between its check and its wait.
    wake_all: bool,
    /// With a parent: while the parent is over its limit this gauge is only over its own when it holds more than
    /// this many bytes, so a session that is drained is never paused by the hoarding of others.
    fair: usize,
    parent: Option<Arc<UnreadGauge>>,
    /// Whether exceeding `limit` is the fault of whoever holds the bytes (a session, an address) and not pressure
    /// from the pool (the server-wide gauge).
    blame: bool,
    /// What a guaranteed charge may bring this gauge to (see `try_charge_share`): an address may hold its quota
    /// plus the guaranteed shares of the sessions it may have open.
    guarantee_cap: usize,
    /// Runs when the gauge is dropped (an address gauge forgets its entry).
    on_drop: Option<Box<dyn Fn() + Send + Sync>>,
    /// How many charges this level has refused: a rollback compares it with the value seen when its charge was taken,
    /// to know whether anyone was refused while this level held a charge that did not stay.
    refusals: AtomicUsize,
}

/// A point of a refusal or an observation where another thread may run (see `UnreadGauge::observe`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// The decision was taken on a value read earlier; nothing is counted yet.
    Decided,
    /// The refusal is counted; the level is not read again yet.
    Counted,
}

/// Why a gauge says to stop reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
pub(crate) enum Pause {
    /// Reading may go on.
    No,
    /// Over a limit of its own: the session's, or its address's.
    Own,
    /// Over its fair share while the server-wide budget is exceeded, whoever exceeded it.
    Pressure,
}

impl UnreadGauge {
    pub(crate) fn gateway(limit: usize) -> Self {
        UnreadGauge {
            bytes: AtomicUsize::new(0),
            notify: Notify::new(),
            limit,
            wake_all: false,
            fair: 0,
            parent: None,
            blame: true,
            guarantee_cap: usize::MAX,
            on_drop: None,
            refusals: AtomicUsize::new(0),
        }
    }

    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn new(limit: usize, parent: Option<Arc<UnreadGauge>>) -> Self {
        UnreadGauge {
            bytes: AtomicUsize::new(0),
            notify: Notify::new(),
            limit,
            wake_all: true,
            fair: 0,
            parent,
            blame: true,
            guarantee_cap: usize::MAX,
            on_drop: None,
            refusals: AtomicUsize::new(0),
        }
    }

    /// Marks the gauge as the shared pool: going over its limit is nobody's own fault.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn pooled(mut self) -> Self {
        self.blame = false;
        self
    }

    /// Sets what guaranteed charges may bring this gauge to.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn with_guarantee_cap(mut self, cap: usize) -> Self {
        self.guarantee_cap = cap;
        self
    }

    /// Runs `f` when the gauge is dropped.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn with_on_drop(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_drop = Some(Box::new(f));
        self
    }

    /// Sets the fair share (see the field). Only meaningful with a parent.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn with_fair_share(mut self, fair: usize) -> Self {
        self.fair = fair;
        self
    }

    pub(crate) fn over_limit(&self) -> bool {
        self.observe().0
    }

    /// One reading of the chain of gauges, each level read once: whether reading must stop, and whether the cause is
    /// the holder's own doing (the session's limit, its address's quota) and not the pool's.
    ///
    /// A caller that gets "over" is about to wait for a release, and a rollback elsewhere (a charge taken at a lower
    /// level that a level above then refused) is a release no one announces, so the observer registers itself like a
    /// refused charge does: it counts a refusal at the level it found over, and reads the level again, retrying if it
    /// changed. The rollback subtracts first and reads the counter second, so with sequentially consistent ordering at
    /// least one of the two sees the other: either the observer sees the level already lowered, or the rollback sees
    /// the count and wakes the waiters. Nothing is counted for a level that is not over, and nothing is woken here.
    fn observe(&self) -> (bool, bool) {
        self.observe_with(&mut |_| {})
    }

    fn observe_with(&self, step: &mut dyn FnMut(Step)) -> (bool, bool) {
        loop {
            let own = self.bytes.load(Ordering::SeqCst);
            let own_over = own > self.limit;
            let (parent_over, parent_fault) = match &self.parent {
                Some(p) if own > self.fair || own_over => p.observe_with(step),
                _ => (false, false),
            };
            let over = own_over || (own > self.fair && parent_over);
            if !over {
                return (false, false);
            }
            step(Step::Decided);
            self.refusals.fetch_add(1, Ordering::SeqCst);
            step(Step::Counted);
            if self.bytes.load(Ordering::SeqCst) != own {
                continue;
            }
            return (true, (self.blame && own_over) || parent_fault);
        }
    }

    /// Bytes received and not yet taken.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn unread(&self) -> usize {
        self.bytes.load(Ordering::SeqCst)
    }

    /// Resolves at the next release of this gauge or of any gauge above it (its address, the server-wide one). The
    /// notifications are registered when this is called, before the caller checks `over_limit`, so a release in
    /// between is not lost.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn changed(&self) -> impl std::future::Future<Output = ()> + '_ {
        let mut waits = Vec::with_capacity(3);
        let mut gauge = Some(self);
        while let Some(g) = gauge {
            waits.push(Box::pin(g.notify.notified()));
            gauge = g.parent.as_deref();
        }
        async move {
            futures::future::select_all(waits).await;
        }
    }

    /// Whether reading must stop, and whose doing it is.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn pause(&self) -> Pause {
        match self.observe() {
            (false, _) => Pause::No,
            (true, true) => Pause::Own,
            (true, false) => Pause::Pressure,
        }
    }

    fn charge(&self, size: usize) {
        self.bytes.fetch_add(size, Ordering::SeqCst);
        if let Some(parent) = &self.parent {
            parent.charge(size);
        }
    }

    /// Adds `size` only if this gauge stays at or below its fair share and every gauge above it at or below its
    /// guarantee cap (an address: its quota plus the guaranteed shares of its sessions; the server-wide one has
    /// none): what a session may always take, so one that holds little is never starved by the hoarding of others.
    /// A compare-and-swap at every level; `false` leaves everything unchanged.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    fn try_charge_share(&self, size: usize, credit: usize) -> bool {
        let Some(seen) = self.try_add_up_to(size, self.fair.saturating_add(credit)) else { return false };
        if let Some(parent) = &self.parent
            && !parent.try_charge_guaranteed(size, credit)
        {
            self.roll_back(size, seen);
            return false;
        }
        true
    }

    fn try_charge_guaranteed(&self, size: usize, credit: usize) -> bool {
        let Some(seen) = self.try_add_up_to(size, self.guarantee_cap.saturating_add(credit)) else { return false };
        if let Some(parent) = &self.parent
            && !parent.try_charge_guaranteed(size, credit)
        {
            self.roll_back(size, seen);
            return false;
        }
        true
    }

    /// Adds `size` here if the total then stays at or below `limit`. `Some` carries the count of refusals this level had
    /// made when the charge was taken (for `roll_back`); a refusal is counted.
    ///
    /// The refusal is counted before it is final: a refused caller increments the count FIRST, then reads the level
    /// again and decides again if it changed. `roll_back` subtracts FIRST and then reads the count. With sequentially
    /// consistent ordering at least one side sees the other: if the refusal's re-read comes before the subtraction, the
    /// increment came before the re-read and so before the rollback's read of the count, which wakes the waiters; if
    /// it comes after, it sees the lowered level and retries instead of waiting for a wakeup that would not come.
    pub(crate) fn try_add_up_to(&self, size: usize, limit: usize) -> Option<usize> {
        self.try_add_up_to_with(size, limit, &mut |_| {})
    }

    /// `try_add_up_to` calling `step` at the points where another thread could interleave (the tests run the other side
    /// there, one step at a time).
    fn try_add_up_to_with(&self, size: usize, limit: usize, step: &mut dyn FnMut(Step)) -> Option<usize> {
        let seen = self.refusals.load(Ordering::SeqCst);
        let mut current = self.bytes.load(Ordering::SeqCst);
        loop {
            let Some(next) = current.checked_add(size).filter(|n| *n <= limit) else {
                step(Step::Decided);
                self.refusals.fetch_add(1, Ordering::SeqCst);
                step(Step::Counted);
                let again = self.bytes.load(Ordering::SeqCst);
                if again != current {
                    current = again;
                    continue;
                }
                return None;
            };
            match self.bytes.compare_exchange_weak(current, next, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Some(seen),
                Err(actual) => current = actual,
            }
        }
    }

    /// Adds `size` to this gauge and its ancestors only if every ancestor stays at or below its own limit (a
    /// compare-and-swap, so concurrent callers cannot overshoot together). The gauge's own limit is not applied: it
    /// decides whether a session reads, not what one decoded packet may add. `Err` leaves everything unchanged.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    fn try_charge_within_ancestors(&self, size: usize, credit: usize) -> bool {
        if let Some(parent) = &self.parent
            && !parent.try_charge_within(size, credit)
        {
            return false;
        }
        self.bytes.fetch_add(size, Ordering::SeqCst);
        true
    }

    /// Adds `size` here and above only if this gauge and its ancestors stay at or below their limits.
    fn try_charge_within(&self, size: usize, credit: usize) -> bool {
        let Some(seen) = self.try_add_up_to(size, self.limit.saturating_add(credit)) else { return false };
        if let Some(parent) = &self.parent
            && !parent.try_charge_within(size, credit)
        {
            self.roll_back(size, seen);
            return false;
        }
        true
    }

    /// Lowers a charge already taken, and wakes the waiters.
    fn shrink(&self, by: usize) {
        self.release(by);
    }

    fn wake(&self) {
        if self.wake_all {
            self.notify.notify_waiters();
        } else {
            self.notify.notify_one();
        }
    }

    /// Undoes a charge of this level that a failed charge higher up made void. For an instant this level was
    /// higher than it ever stays, and a sibling that was refused then is waiting for it to drop, so the waiters are
    /// woken when this level refused anything since the charge was taken (`seen` is the refusal count then). A
    /// rollback that nobody could have noticed wakes no one: the asker itself waits on these levels too, and waking it
    /// for its own rollback would make it retry in a loop for as long as a level above stays full.
    pub(crate) fn roll_back(&self, size: usize, seen: usize) {
        self.bytes.fetch_sub(size, Ordering::SeqCst);
        if self.refusals.load(Ordering::SeqCst) != seen {
            self.wake();
        }
    }

    fn release(&self, size: usize) {
        self.bytes.fetch_sub(size, Ordering::SeqCst);
        self.wake();
        if let Some(parent) = &self.parent {
            parent.release(size);
        }
    }
}

/// Accounts one message queued in a `messages()` stream until it is taken (or dropped).
pub(crate) struct UnreadTicket {
    gauge: Arc<UnreadGauge>,
    size: usize,
}

impl UnreadTicket {
    pub(crate) fn new(gauge: &Arc<UnreadGauge>, size: usize) -> Self {
        gauge.charge(size);
        UnreadTicket { gauge: gauge.clone(), size }
    }

    /// Charges `size` only if the gauge stays within its fair share and its ancestors within their guarantee caps (see
    /// `UnreadGauge::try_charge_share`).
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn try_new_within_share(gauge: &Arc<UnreadGauge>, size: usize, credit: usize) -> Option<Self> {
        gauge.try_charge_share(size, credit).then(|| UnreadTicket { gauge: gauge.clone(), size })
    }

    /// Charges `size` only if the gauge's ancestors stay within their limits (see `try_charge_within_ancestors`).
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn try_new_within_ancestors(gauge: &Arc<UnreadGauge>, size: usize, credit: usize) -> Option<Self> {
        gauge.try_charge_within_ancestors(size, credit).then(|| UnreadTicket { gauge: gauge.clone(), size })
    }

    /// Lowers the charge to `size` (never raises it), releasing the difference and waking the waiters.
    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn shrink_to(&mut self, size: usize) {
        if size < self.size {
            self.gauge.shrink(self.size - size);
            self.size = size;
        }
    }

    #[cfg_attr(not(feature = "socket-io"), allow(dead_code))]
    pub(crate) fn size(&self) -> usize {
        self.size
    }
}

impl Drop for UnreadGauge {
    fn drop(&mut self) {
        if let Some(f) = self.on_drop.take() {
            f();
        }
    }
}

impl Drop for UnreadTicket {
    fn drop(&mut self) {
        self.gauge.release(self.size);
    }
}

/// A message received or sent on a [`WebSocket`] (`data` + `isBinary` in `ws`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Bytes),
}

impl WsMessage {
    pub fn is_binary(&self) -> bool {
        matches!(self, WsMessage::Binary(_))
    }
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            WsMessage::Text(t) => t.as_bytes(),
            WsMessage::Binary(b) => b,
        }
    }
    /// The payload decoded as UTF-8 (lossy), like `String(data)` in Node.
    pub fn to_text(&self) -> Cow<'_, str> {
        match self {
            WsMessage::Text(t) => Cow::Borrowed(t),
            WsMessage::Binary(b) => String::from_utf8_lossy(b),
        }
    }
    pub fn into_bytes(self) -> Bytes {
        match self {
            WsMessage::Text(t) => Bytes::from(t),
            WsMessage::Binary(b) => b,
        }
    }
    fn into_tungstenite(self) -> Message {
        match self {
            WsMessage::Text(t) => Message::Text(t),
            WsMessage::Binary(b) => Message::Binary(b.to_vec()),
        }
    }
}

impl std::fmt::Display for WsMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_text())
    }
}
impl From<String> for WsMessage {
    fn from(v: String) -> Self {
        WsMessage::Text(v)
    }
}
impl From<&str> for WsMessage {
    fn from(v: &str) -> Self {
        WsMessage::Text(v.to_string())
    }
}
impl From<&String> for WsMessage {
    fn from(v: &String) -> Self {
        WsMessage::Text(v.clone())
    }
}
impl From<serde_json::Value> for WsMessage {
    /// `JSON.stringify(value)` sent as a text frame.
    fn from(v: serde_json::Value) -> Self {
        WsMessage::Text(v.to_string())
    }
}
impl From<Bytes> for WsMessage {
    fn from(v: Bytes) -> Self {
        WsMessage::Binary(v)
    }
}
impl From<Vec<u8>> for WsMessage {
    fn from(v: Vec<u8>) -> Self {
        WsMessage::Binary(Bytes::from(v))
    }
}
impl From<&[u8]> for WsMessage {
    fn from(v: &[u8]) -> Self {
        WsMessage::Binary(Bytes::copy_from_slice(v))
    }
}

/// `ws.readyState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ReadyState {
    Connecting = 0,
    Open = 1,
    Closing = 2,
    Closed = 3,
}

impl ReadyState {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
    /// `'CONNECTING' | 'OPEN' | 'CLOSING' | 'CLOSED'`.
    pub fn name(self) -> &'static str {
        match self {
            ReadyState::Connecting => "CONNECTING",
            ReadyState::Open => "OPEN",
            ReadyState::Closing => "CLOSING",
            ReadyState::Closed => "CLOSED",
        }
    }
    fn from_u8(v: u8) -> Self {
        match v {
            0 => ReadyState::Connecting,
            1 => ReadyState::Open,
            2 => ReadyState::Closing,
            _ => ReadyState::Closed,
        }
    }
}

/// An error of a [`WebSocket`] (`'error'` event, failed `send`, invalid `close` arguments).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsError {
    message: String,
    code: Option<&'static str>,
}

impl WsError {
    fn new(message: impl Into<String>, code: Option<&'static str>) -> Self {
        WsError { message: message.into(), code }
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    /// The `ws` error code (`'WS_ERR_INVALID_UTF8'`, ...) for protocol errors.
    pub fn code(&self) -> Option<&'static str> {
        self.code
    }
}
impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for WsError {}

type MessageListener = Arc<dyn Fn(WsMessage) + Send + Sync>;
type CloseListener = Arc<dyn Fn(u16, &str) + Send + Sync>;
type ErrorListener = Arc<dyn Fn(&WsError) + Send + Sync>;
type BytesListener = Arc<dyn Fn(Bytes) + Send + Sync>;

#[derive(Default)]
struct Listeners {
    message: Vec<MessageListener>,
    close: Vec<CloseListener>,
    error: Vec<ErrorListener>,
    ping: Vec<BytesListener>,
    pong: Vec<BytesListener>,
}

/// Requests to the connection task (frames go through the [`OutQueue`]).
enum Control {
    /// A close frame was queued: arm the close deadline.
    Close,
    /// Destroy the connection now.
    Terminate,
}

static SOCKET_IDS: AtomicU64 = AtomicU64::new(1);

struct WsInner {
    id: u64,
    state: AtomicU8,
    protocol: String,
    request: Request,
    queue: Arc<OutQueue>,
    control: mpsc::UnboundedSender<Control>,
    unread: Arc<UnreadGauge>,
    listeners: Mutex<Listeners>,
    closed: watch::Sender<Option<(u16, String)>>,
}

/// A server-side WebSocket connection (`ws`' `WebSocket`). Cheap to clone; clones refer
/// to the same connection (equality and hashing are by connection, so it can be kept
/// in a `HashSet` like `new Set<WebSocket>()`).
///
/// Event listeners (`on_message`, `on_close`, ...) should be registered inside the
/// `handle_upgrade` callback: frames are only read after it returns, so none is lost.
#[derive(Clone)]
pub struct WebSocket {
    inner: Arc<WsInner>,
}

impl PartialEq for WebSocket {
    fn eq(&self, other: &Self) -> bool {
        self.inner.id == other.inner.id
    }
}
impl Eq for WebSocket {}
impl std::hash::Hash for WebSocket {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.inner.id.hash(state)
    }
}
impl std::fmt::Debug for WebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocket").field("id", &self.inner.id).field("ready_state", &self.ready_state()).finish()
    }
}

impl WebSocket {
    /// A process-unique id of the connection.
    pub fn id(&self) -> u64 {
        self.inner.id
    }
    /// `ws.readyState`.
    pub fn ready_state(&self) -> ReadyState {
        ReadyState::from_u8(self.inner.state.load(Ordering::SeqCst))
    }
    /// `ws.readyState === WebSocket.OPEN`.
    pub fn is_open(&self) -> bool {
        self.ready_state() == ReadyState::Open
    }
    /// `ws.protocol`: the negotiated subprotocol (empty when none).
    pub fn protocol(&self) -> &str {
        &self.inner.protocol
    }
    /// The HTTP upgrade request (`req` of the `connection` event).
    pub fn upgrade_request(&self) -> &Request {
        &self.inner.request
    }
    pub fn remote_addr(&self) -> Option<std::net::SocketAddr> {
        self.inner.request.remote_addr()
    }

    fn not_open(&self) -> WsError {
        let s = self.ready_state();
        WsError::new(format!("WebSocket is not open: readyState {} ({})", s.as_u8(), s.name()), None)
    }

    /// `ws.bufferedAmount`: bytes queued for sending and not yet written to the socket.
    pub fn buffered_amount(&self) -> usize {
        self.inner.queue.buffered()
    }

    fn enqueue(&self, m: Message, ack: Option<Ack>) -> Result<(), WsError> {
        if !self.is_open() {
            return Err(self.not_open());
        }
        match self.inner.queue.push(m, ack) {
            Ok(()) => Ok(()),
            Err(PushError::Closed) => Err(self.not_open()),
            Err(PushError::Full) => {
                // The peer does not read: drop what is pending and close with 1008.
                Logger::new("WebSocket").warn(format!("socket {}: send buffer limit exceeded, closing with 1008", self.id()));
                self.close_internal(CLOSE_POLICY_VIOLATION, "Send buffer limit exceeded", true);
                Err(WsError::new("WebSocket send buffer limit exceeded (max_buffered_amount)", Some("WS_ERR_BUFFER_FULL")))
            }
        }
    }

    fn close_internal(&self, code: u16, reason: &str, discard: bool) {
        if self.ready_state() == ReadyState::Closed {
            return;
        }
        self.inner.state.store(ReadyState::Closing as u8, Ordering::SeqCst);
        let frame = CloseFrame { code: CloseCode::from(code), reason: Cow::Owned(reason.to_string()) };
        if self.inner.queue.close(Some(frame), discard) {
            let _ = self.inner.control.send(Control::Close);
        }
    }

    /// `ws.send(data)`: queues a text (`String`, `&str`, `serde_json::Value`) or binary
    /// (`Vec<u8>`, `Bytes`) frame. Fails if the socket is not `OPEN`, or if the send buffer
    /// would exceed `max_buffered_amount`; in that case the connection is closed with `1008`.
    pub fn send(&self, data: impl Into<WsMessage>) -> Result<(), WsError> {
        self.enqueue(data.into().into_tungstenite(), None)
    }

    /// `ws.send(data, (err) => ...)`: like [`send`](Self::send) but resolves once the frame
    /// was written to the socket, with the write error if any.
    pub async fn send_and_wait(&self, data: impl Into<WsMessage>) -> Result<(), WsError> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(data.into().into_tungstenite(), Some(tx))?;
        rx.await.unwrap_or_else(|_| Err(self.not_open()))
    }

    /// `ws.ping(data)`.
    pub fn ping(&self, data: impl Into<Bytes>) -> Result<(), WsError> {
        self.enqueue(Message::Ping(data.into().to_vec()), None)
    }

    /// `ws.pong(data)`.
    pub fn pong(&self, data: impl Into<Bytes>) -> Result<(), WsError> {
        self.enqueue(Message::Pong(data.into().to_vec()), None)
    }

    /// `ws.close([code[, reason]])`: starts the closing handshake (the close frame goes after
    /// the frames already queued). The connection is destroyed if the peer does not answer
    /// within `close_timeout` (30 seconds by default, like `ws`).
    /// Valid codes: 1000, 1001-1003, 1007-1014 and 3000-4999; the reason must fit in 123 bytes.
    pub fn close(&self, code: Option<u16>, reason: &str) -> Result<(), WsError> {
        if let Some(c) = code {
            let valid = matches!(c, 1000..=1003 | 1007..=1014 | 3000..=4999);
            if !valid {
                return Err(WsError::new("First argument must be a valid error code number", None));
            }
        }
        if reason.len() > 123 {
            return Err(WsError::new("The message must not be greater than 123 bytes", None));
        }
        if self.ready_state() != ReadyState::Open {
            return Ok(());
        }
        self.inner.state.store(ReadyState::Closing as u8, Ordering::SeqCst);
        let frame = code.map(|c| CloseFrame { code: CloseCode::from(c), reason: Cow::Owned(reason.to_string()) });
        if self.inner.queue.close(frame, false) {
            let _ = self.inner.control.send(Control::Close);
        }
        Ok(())
    }

    /// `ws.terminate()`: destroys the connection immediately (close code `1006`).
    pub fn terminate(&self) {
        if self.ready_state() == ReadyState::Closed {
            return;
        }
        self.inner.state.store(ReadyState::Closing as u8, Ordering::SeqCst);
        let _ = self.inner.control.send(Control::Terminate);
    }

    /// `ws.on('message', (data, isBinary) => ...)`.
    pub fn on_message(&self, f: impl Fn(WsMessage) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().message.push(Arc::new(f));
        self
    }
    /// `ws.on('close', (code, reason) => ...)`: `1005` when the peer sent no status code,
    /// `1006` when the connection was lost or terminated without a closing handshake.
    pub fn on_close(&self, f: impl Fn(u16, &str) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().close.push(Arc::new(f));
        self
    }
    /// `ws.on('error', (err) => ...)`: protocol errors (invalid UTF-8, payload too big, ...).
    pub fn on_error(&self, f: impl Fn(&WsError) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().error.push(Arc::new(f));
        self
    }
    /// `ws.on('ping', (data) => ...)` (the pong is sent automatically).
    pub fn on_ping(&self, f: impl Fn(Bytes) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().ping.push(Arc::new(f));
        self
    }
    /// `ws.on('pong', (data) => ...)`.
    pub fn on_pong(&self, f: impl Fn(Bytes) + Send + Sync + 'static) -> &Self {
        self.inner.listeners.lock().unwrap().pong.push(Arc::new(f));
        self
    }

    /// The incoming messages as a `Stream` that ends when the socket closes. Call it
    /// inside the `handle_upgrade` callback so no message is missed.
    ///
    /// Backpressure: while more than `max_unread_bytes` received bytes wait in the streams,
    /// the connection stops reading from the socket (the peer's sends block in TCP).
    pub fn messages(&self) -> std::pin::Pin<Box<dyn Stream<Item = WsMessage> + Send + 'static>> {
        let (tx, rx) = mpsc::unbounded_channel::<(WsMessage, UnreadTicket)>();
        if self.ready_state() != ReadyState::Closed {
            let tx = Arc::new(Mutex::new(Some(tx)));
            let t1 = tx.clone();
            let gauge = self.inner.unread.clone();
            self.on_message(move |m| {
                if let Some(tx) = t1.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    let ticket = UnreadTicket::new(&gauge, m.as_bytes().len() + 16);
                    let _ = tx.send((m, ticket));
                }
            });
            self.on_close(move |_, _| {
                tx.lock().unwrap_or_else(|e| e.into_inner()).take();
            });
        }
        Box::pin(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|(m, _ticket)| (m, rx)) }))
    }

    /// Resolves with `(code, reason)` once the connection is closed.
    pub async fn closed(&self) -> (u16, String) {
        let mut rx = self.inner.closed.subscribe();
        loop {
            if let Some(v) = rx.borrow_and_update().clone() {
                return v;
            }
            if rx.changed().await.is_err() {
                return (1006, String::new());
            }
        }
    }

    fn listeners(&self) -> std::sync::MutexGuard<'_, Listeners> {
        self.inner.listeners.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// A listener that panicked is logged and the connection is closed with `1011`, so the
    /// socket never stays half-alive.
    fn listener_panicked(&self) {
        self.close_internal(CLOSE_INTERNAL_ERROR, "", false);
    }
    fn emit_message(&self, m: WsMessage) {
        let ls = self.listeners().message.clone();
        for l in ls {
            if !guarded("a 'message' listener", || l(m.clone())) {
                self.listener_panicked();
            }
        }
    }
    fn emit_error(&self, e: &WsError) {
        let ls = self.listeners().error.clone();
        for l in ls {
            if !guarded("an 'error' listener", || l(e)) {
                self.listener_panicked();
            }
        }
    }
    fn emit_bytes(&self, ping: bool, data: Bytes) {
        let ls = {
            let g = self.listeners();
            if ping { g.ping.clone() } else { g.pong.clone() }
        };
        for l in ls {
            if !guarded("a 'ping'/'pong' listener", || l(data.clone())) {
                self.listener_panicked();
            }
        }
    }
    fn emit_close(&self, code: u16, reason: &str) {
        let ls = std::mem::take(&mut self.listeners().close);
        for l in ls {
            guarded("a 'close' listener", || l(code, reason));
        }
        // Drop the remaining listeners: they may hold clones of this socket.
        *self.listeners() = Listeners::default();
        self.inner.closed.send_replace(Some((code, reason.to_string())));
    }
}

/// Handles the protocol selection (`handleProtocols(protocols, req)`).
pub type HandleProtocols = Arc<dyn Fn(&[String], &Request) -> Option<String> + Send + Sync>;

/// Options of [`WebSocketServer::new`] (the `noServer: true` mode of `ws`).
#[derive(Clone)]
pub struct WebSocketServerOptions {
    /// `maxPayload` in bytes (default 100 MiB).
    pub max_payload: usize,
    /// `clientTracking` (default `true`): keep the connected sockets in `clients()`.
    pub client_tracking: bool,
    /// `handleProtocols`; by default the first requested subprotocol is accepted.
    pub handle_protocols: Option<HandleProtocols>,
    /// Maximum bytes queued for sending on one connection (default 8 MiB). When `send` would
    /// exceed it (the peer does not read), the pending frames are dropped, `send` fails with
    /// `WS_ERR_BUFFER_FULL` and the connection is closed with `1008`. Not in `ws`, which
    /// buffers without limit and leaves the check of `bufferedAmount` to the application.
    pub max_buffered_amount: usize,
    /// How long a closing connection waits for the peer's close frame before the TCP
    /// connection is destroyed (default 30 s, like `ws`).
    pub close_timeout: Duration,
    /// Received bytes allowed to wait in [`WebSocket::messages`] streams before the
    /// connection stops reading from the socket (default 16 MiB).
    pub max_unread_bytes: usize,
}

impl Default for WebSocketServerOptions {
    fn default() -> Self {
        WebSocketServerOptions {
            max_payload: 100 * 1024 * 1024,
            client_tracking: true,
            handle_protocols: None,
            max_buffered_amount: DEFAULT_MAX_BUFFERED_AMOUNT,
            close_timeout: DEFAULT_CLOSE_TIMEOUT,
            max_unread_bytes: DEFAULT_MAX_UNREAD_BYTES,
        }
    }
}

impl WebSocketServerOptions {
    pub fn max_buffered_amount(mut self, bytes: usize) -> Self {
        self.max_buffered_amount = bytes;
        self
    }
    pub fn close_timeout(mut self, timeout: Duration) -> Self {
        self.close_timeout = timeout;
        self
    }
    pub fn max_unread_bytes(mut self, bytes: usize) -> Self {
        self.max_unread_bytes = bytes;
        self
    }
    pub fn max_payload(mut self, bytes: usize) -> Self {
        self.max_payload = bytes;
        self
    }
    pub fn client_tracking(mut self, v: bool) -> Self {
        self.client_tracking = v;
        self
    }
    pub fn handle_protocols(mut self, f: impl Fn(&[String], &Request) -> Option<String> + Send + Sync + 'static) -> Self {
        self.handle_protocols = Some(Arc::new(f));
        self
    }
}

struct WssInner {
    options: WebSocketServerOptions,
    clients: Mutex<HashMap<u64, WebSocket>>,
    closed: AtomicBool,
}

/// `new WebSocketServer({ noServer: true })`: performs WebSocket handshakes for upgrade
/// requests received through `HttpServer::on_upgrade`. Cheap to clone.
#[derive(Clone)]
pub struct WebSocketServer {
    inner: Arc<WssInner>,
}

impl Default for WebSocketServer {
    fn default() -> Self {
        Self::new(WebSocketServerOptions::default())
    }
}

fn abort_handshake(req: &UpgradeRequest, status: u16, message: Option<&str>, extra: &[(&str, &str)]) {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
    let message = message.unwrap_or(status.canonical_reason().unwrap_or("")).to_string();
    let mut resp = Response::bytes(message, "text/html").with_status(status);
    for (k, v) in extra {
        resp.set_header(k, v);
    }
    let _ = req.respond(resp);
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// `subprotocol.parse()` of `ws`.
fn parse_subprotocols(header: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for part in header.split(',') {
        let p = part.trim_matches([' ', '\t']);
        if p.is_empty() || !p.bytes().all(is_token_char) || out.iter().any(|x| x == p) {
            return None;
        }
        out.push(p.to_string());
    }
    Some(out)
}

impl WebSocketServer {
    pub fn new(options: WebSocketServerOptions) -> Self {
        WebSocketServer { inner: Arc::new(WssInner { options, clients: Mutex::new(HashMap::new()), closed: AtomicBool::new(false) }) }
    }

    /// `wss.clients`: the open connections (requires `client_tracking`).
    pub fn clients(&self) -> Vec<WebSocket> {
        self.inner.clients.lock().unwrap().values().cloned().collect()
    }

    /// `wss.close()`: new handshakes are rejected with `503`; existing connections are
    /// left open (as in `ws` 8; terminate them explicitly if needed).
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
    }

    /// `wss.handleUpgrade(req, socket, head, cb)`: validates the handshake like `ws`
    /// (`405 Invalid HTTP method`, `400 Missing or invalid Sec-WebSocket-Key header`, ...),
    /// answers `101 Switching Protocols` and calls `cb` with the connected socket.
    /// Does nothing (except logging) if the request was already handled.
    pub fn handle_upgrade(&self, req: &UpgradeRequest, cb: impl FnOnce(WebSocket) + Send + 'static) {
        if req.is_handled() {
            Logger::new("WebSocketServer").warn("handleUpgrade() was called for an upgrade request that was already handled");
            return;
        }
        if req.method() != http::Method::GET {
            return abort_handshake(req, 405, Some("Invalid HTTP method"), &[]);
        }
        if !req.header("upgrade").map(|u| u.eq_ignore_ascii_case("websocket")).unwrap_or(false) {
            return abort_handshake(req, 400, Some("Invalid Upgrade header"), &[]);
        }
        let key = req.header("sec-websocket-key").unwrap_or("");
        let key_ok = key.len() == 24
            && key.ends_with("==")
            && key.as_bytes()[..22].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/');
        if !key_ok {
            return abort_handshake(req, 400, Some("Missing or invalid Sec-WebSocket-Key header"), &[]);
        }
        let version = req.header("sec-websocket-version").and_then(|v| v.trim().parse::<u32>().ok());
        if version != Some(13) && version != Some(8) {
            return abort_handshake(
                req,
                400,
                Some("Missing or invalid Sec-WebSocket-Version header"),
                &[("Sec-WebSocket-Version", "13, 8")],
            );
        }
        let protocols = match req.header("sec-websocket-protocol") {
            Some(h) => match parse_subprotocols(h) {
                Some(p) => p,
                None => return abort_handshake(req, 400, Some("Invalid Sec-WebSocket-Protocol header"), &[]),
            },
            None => Vec::new(),
        };
        if self.inner.closed.load(Ordering::SeqCst) {
            return abort_handshake(req, 503, None, &[]);
        }
        let protocol = if protocols.is_empty() {
            None
        } else {
            match &self.inner.options.handle_protocols {
                Some(f) => f(&protocols, req.request()),
                None => protocols.first().cloned(),
            }
        };
        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(http::header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(http::header::CONNECTION, HeaderValue::from_static("Upgrade"));
        if let Ok(v) = HeaderValue::from_str(&accept) {
            headers.insert("sec-websocket-accept", v);
        }
        if let Some(p) = &protocol
            && let Ok(v) = HeaderValue::from_str(p)
        {
            headers.insert("sec-websocket-protocol", v);
        }
        let Ok(upgraded) = req.switch_protocols(headers) else {
            Logger::new("WebSocketServer").warn("handleUpgrade() was called for an upgrade request that was already handled");
            return;
        };
        let request = req.request().clone();
        let server = self.clone();
        tokio::spawn(async move {
            let Ok(io) = upgraded.await else { return };
            let options = &server.inner.options;
            let config = WebSocketConfig {
                max_message_size: Some(options.max_payload),
                max_frame_size: Some(options.max_payload),
                ..WebSocketConfig::default()
            };
            let stream = WebSocketStream::from_raw_socket(io, Role::Server, Some(config)).await;
            let (control, rx) = mpsc::unbounded_channel();
            let ws = WebSocket {
                inner: Arc::new(WsInner {
                    id: SOCKET_IDS.fetch_add(1, Ordering::SeqCst),
                    state: AtomicU8::new(ReadyState::Open as u8),
                    protocol: protocol.unwrap_or_default(),
                    request,
                    queue: Arc::new(OutQueue::new(options.max_buffered_amount)),
                    control,
                    unread: Arc::new(UnreadGauge::gateway(options.max_unread_bytes)),
                    listeners: Mutex::new(Listeners::default()),
                    closed: watch::channel(None).0,
                }),
            };
            if options.client_tracking {
                server.inner.clients.lock().unwrap_or_else(|e| e.into_inner()).insert(ws.id(), ws.clone());
            }
            // Removes the socket from `clients()` and reports it closed even if something
            // below panics.
            let _cleanup = Cleanup { server: server.clone(), ws: ws.clone() };
            if !guarded("the handleUpgrade callback", || cb(ws.clone())) {
                ws.terminate();
            }
            run(ws.clone(), stream, rx, options.close_timeout).await;
        });
    }
}

struct Cleanup {
    server: WebSocketServer,
    ws: WebSocket,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.server.inner.clients.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.ws.id());
        if self.ws.ready_state() != ReadyState::Closed {
            self.ws.inner.state.store(ReadyState::Closed as u8, Ordering::SeqCst);
            self.ws.inner.queue.shutdown();
            if std::thread::panicking() {
                // Listeners are not called while unwinding (a second panic would abort).
                *self.ws.listeners() = Listeners::default();
                self.ws.inner.closed.send_replace(Some((1006, String::new())));
            } else {
                self.ws.emit_close(1006, "");
            }
        }
    }
}

pub(crate) fn protocol_error(e: &tokio_tungstenite::tungstenite::Error) -> (WsError, u16) {
    use tokio_tungstenite::tungstenite::Error as E;
    match e {
        E::Capacity(_) => (WsError::new("Max payload size exceeded", Some("WS_ERR_UNSUPPORTED_MESSAGE_LENGTH")), 1009),
        E::Utf8 => (WsError::new("Invalid UTF-8 sequence", Some("WS_ERR_INVALID_UTF8")), 1007),
        other => (WsError::new(other.to_string(), Some("WS_ERR_PROTOCOL")), 1002),
    }
}

async fn run(ws: WebSocket, stream: WebSocketStream<UpgradedIo>, mut control: mpsc::UnboundedReceiver<Control>, close_timeout: Duration) {
    use tokio_tungstenite::tungstenite::Error as E;
    let (sink, mut stream) = stream.split();
    let mut writer = tokio::spawn(write_loop(sink, ws.inner.queue.clone()));
    let mut writer_done = false;
    let mut received: Option<(u16, String)> = None;
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        let current_deadline = deadline;
        let timeout = async move {
            match current_deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => futures::future::pending::<()>().await,
            }
        };
        let paused = ws.inner.unread.over_limit();
        tokio::select! {
            incoming = stream.next(), if !paused => match incoming {
                None => break,
                Some(Ok(msg)) => match msg {
                    Message::Text(t) => ws.emit_message(WsMessage::Text(t)),
                    Message::Binary(b) => ws.emit_message(WsMessage::Binary(Bytes::from(b))),
                    // tungstenite answers pings by itself.
                    Message::Ping(p) => ws.emit_bytes(true, Bytes::from(p)),
                    Message::Pong(p) => ws.emit_bytes(false, Bytes::from(p)),
                    Message::Close(frame) => {
                        received = Some(match frame {
                            Some(f) => (u16::from(f.code), f.reason.into_owned()),
                            None => (1005, String::new()),
                        });
                        ws.inner.state.store(ReadyState::Closing as u8, Ordering::SeqCst);
                        deadline.get_or_insert_with(|| tokio::time::Instant::now() + close_timeout);
                    }
                    Message::Frame(_) => {}
                },
                Some(Err(E::ConnectionClosed | E::AlreadyClosed | E::Io(_))) => break,
                Some(Err(e)) => {
                    let (err, code) = protocol_error(&e);
                    ws.emit_error(&err);
                    let frame = CloseFrame { code: CloseCode::from(code), reason: Cow::Borrowed("") };
                    ws.inner.queue.close(Some(frame), true);
                    if !writer_done {
                        writer_done = tokio::time::timeout(Duration::from_secs(1), &mut writer).await.is_ok();
                    }
                    break;
                }
            },
            _ = ws.inner.unread.notify.notified(), if paused => {}
            cmd = control.recv() => match cmd {
                Some(Control::Close) => {
                    deadline.get_or_insert_with(|| tokio::time::Instant::now() + close_timeout);
                }
                Some(Control::Terminate) | None => break,
            },
            _ = timeout => break,
            // The writer stops after a write error (the peer accepts nothing: `send_timeout`) or after a close frame.
            end = &mut writer, if !writer_done => {
                writer_done = true;
                match end {
                    Ok(WriteEnd::Failed(kind)) => {
                        if kind == std::io::ErrorKind::TimedOut {
                            Logger::new("WebSocket").warn(format!(
                                "socket {}: accepted no data for the server's send_timeout, connection dropped",
                                ws.inner.id
                            ));
                        }
                        break;
                    }
                    // The socket is fine: a pending close reply still goes out through the read half, so keep
                    // reading until the peer closes or `close_timeout` passes.
                    Ok(WriteEnd::Closed | WriteEnd::Refused) if received.is_some() => break,
                    Ok(WriteEnd::Closed | WriteEnd::Refused) => {}
                    Err(_) => break,
                }
            }
        }
    }
    // Dropping both halves closes the TCP connection, even if the writer is blocked on a
    // peer that does not read.
    if !writer_done {
        writer.abort();
        let _ = writer.await;
    }
    drop(stream);
    ws.inner.state.store(ReadyState::Closed as u8, Ordering::SeqCst);
    // Fail pending `send_and_wait` calls.
    ws.inner.queue.shutdown();
    let (code, reason) = received.unwrap_or((1006, String::new()));
    ws.emit_close(code, &reason);
}

#[cfg(test)]
mod gauge_tests {
    use super::*;
    use futures::FutureExt;

    /// Wakeups after a rollback, with the other side run at each point where it could interleave. The refusing side
    /// counts FIRST and reads the level again; the rollback subtracts FIRST and reads the count.
    #[test]
    fn a_refusal_that_lands_around_a_rollback_is_never_left_waiting() {
        for stage in [Step::Decided, Step::Counted] {
            let level = UnreadGauge::new(300, None);
            let waits = level.changed();
            let seen = level.try_add_up_to(25, 300).expect("the asker's charge");
            let mut rolled_back = false;
            // The sibling decides on 25 held (it needs 290 of 300) and the whole rollback runs at `stage`.
            let got = level.try_add_up_to_with(290, 300, &mut |at| {
                if at == stage && !rolled_back {
                    rolled_back = true;
                    level.roll_back(25, seen);
                }
            });
            assert!(rolled_back);
            let woken = waits.now_or_never().is_some();
            // Either the sibling saw the lowered level and was admitted, or it was refused and the rollback woke it.
            assert!(got.is_some() || woken, "lost wakeup at {stage:?}");
            assert!(got.is_some(), "the re-read sees the level lowered, so it retries and is admitted ({stage:?})");
            assert_eq!(woken, stage == Step::Counted, "only a rollback that saw the count wakes ({stage:?})");
        }
    }

    /// The observer side: a caller told "paused" because a level is over its limit counts like a refusal.
    #[test]
    fn an_observer_that_saw_a_transient_peak_is_woken_by_the_rollback() {
        for stage in [None, Some(Step::Decided), Some(Step::Counted)] {
            let level = Arc::new(UnreadGauge::new(300, None));
            let session = Arc::new(UnreadGauge::new(10_000, Some(level.clone())));
            let held = UnreadTicket::new(&session, 5);
            let waits = session.changed();
            // The asker's charge takes the address to 350 while its limit is 300; the level above has not answered.
            let seen = level.try_add_up_to(350, 400).expect("the asker's charge");
            let mut rolled_back = false;
            let pause = match stage {
                None => {
                    let pause = session.pause();
                    level.roll_back(350, seen);
                    rolled_back = true;
                    pause
                }
                Some(at) => {
                    let (over, _) = session.observe_with(&mut |s| {
                        if s == at && !rolled_back {
                            rolled_back = true;
                            level.roll_back(350, seen);
                        }
                    });
                    if over { Pause::Own } else { Pause::No }
                }
            };
            assert!(rolled_back);
            let woken = waits.now_or_never().is_some();
            match stage {
                None => assert!(pause == Pause::Own && woken, "reported paused, so the rollback must wake it"),
                // The first read saw the peak, the rollback came before the count, so the re-read retries and finds
                // the level lowered: not paused, nothing to wake.
                Some(Step::Decided) => assert!(pause == Pause::No && !woken),
                // The count was in place when the rollback read it: woken, and the re-read retries to not paused.
                Some(Step::Counted) => assert!(pause == Pause::No && woken),
            }
            assert_eq!(session.pause(), Pause::No);
            drop(held);
        }
    }

    #[test]
    fn a_pause_is_classified_from_one_reading_and_idle_observers_wake_nobody() {
        let total = Arc::new(UnreadGauge::new(1000, None).pooled());
        let address = Arc::new(UnreadGauge::new(300, Some(total.clone())));
        let session = Arc::new(UnreadGauge::new(10_000, Some(address.clone())).with_fair_share(25));
        let held = UnreadTicket::new(&session, 20);
        assert_eq!(session.pause(), Pause::No);
        // Not over: nothing is counted, so a rollback has nobody to wake.
        let waits = address.changed();
        let seen = address.try_add_up_to(10, 400).expect("a charge");
        address.roll_back(10, seen);
        assert!(waits.now_or_never().is_none());
        // Over the address quota: the holder's own doing, from the same reading that found it over.
        let more = UnreadTicket::new(&session, 400);
        assert_eq!(session.pause(), Pause::Own);
        assert_eq!(session.observe(), (true, true));
        drop((held, more));
        // Over only the pool's limit: pressure.
        let total2 = Arc::new(UnreadGauge::new(100, None).pooled());
        let address2 = Arc::new(UnreadGauge::new(300, Some(total2.clone())));
        let session2 = Arc::new(UnreadGauge::new(10_000, Some(address2.clone())).with_fair_share(25));
        let _x = UnreadTicket::new(&session2, 150);
        assert_eq!(session2.pause(), Pause::Pressure);
    }

    /// Many askers and observers on two addresses sharing a pool that refuses above the addresses' caps: charges taken
    /// at the address level and rolled back by the pool. When all activity stops nobody may be left parked while room
    /// exists. The waiters never time out by themselves; a lost wakeup is a thread that never finishes.
    #[test]
    fn nobody_is_left_parked_while_room_exists_after_the_activity_stops() {
        use std::sync::mpsc;
        use std::time::Duration;

        let total = Arc::new(UnreadGauge::new(100, None).pooled().with_guarantee_cap(100));
        let mut threads = 0usize;
        let (done, finished) = mpsc::channel::<()>();
        for _ in 0..2 {
            let address = Arc::new(UnreadGauge::new(40, Some(total.clone())).with_guarantee_cap(80));
            for _ in 0..8 {
                let session = Arc::new(UnreadGauge::new(10_000, Some(address.clone())).with_fair_share(20));
                let done = done.clone();
                threads += 1;
                std::thread::spawn(move || {
                    for _ in 0..300 {
                        let ticket = loop {
                            let wait = session.changed();
                            if let Some(t) = UnreadTicket::try_new_within_share(&session, 20, 0) {
                                break t;
                            }
                            futures::executor::block_on(wait);
                        };
                        std::thread::yield_now();
                        drop(ticket);
                    }
                    let _ = done.send(());
                });
            }
            for _ in 0..4 {
                let address = address.clone();
                let done = done.clone();
                threads += 1;
                std::thread::spawn(move || {
                    for _ in 0..300 {
                        loop {
                            let wait = address.changed();
                            if !address.over_limit() {
                                break;
                            }
                            futures::executor::block_on(wait);
                        }
                        std::thread::yield_now();
                    }
                    let _ = done.send(());
                });
            }
        }
        drop(done);
        let mut ended = 0usize;
        while ended < threads {
            match finished.recv_timeout(Duration::from_secs(60)) {
                Ok(()) => ended += 1,
                Err(_) => panic!("{} of {threads} waiters are parked while room exists", threads - ended),
            }
        }
        assert_eq!(total.unread(), 0, "everything was given back");
    }
}
