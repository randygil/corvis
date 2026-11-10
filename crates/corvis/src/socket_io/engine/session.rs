//! One Engine.IO session: its state machine, outbox, heartbeat, upgrade state and close.
//!
//! All state transitions are synchronous functions under one mutex; the transports (`polling`, `websocket`) only
//! move bytes and call into them. One task per session owns the timers (ping interval, ping timeout, upgrade
//! timeout, the noop of a pending upgrade, the close timeout) and runs on `tokio::time`, so the unit tests below
//! drive it with a paused clock.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::{Notify, mpsc, watch};
use tokio::time::Instant;

use super::options::EngineOptions;
use super::packet::{EngineMessage, Packet};
use super::{ServerInner, TransportKind};
use crate::http::Request;
use crate::logger::Logger;
use crate::ws::{OutQueue, PushError, QueueItem, UnreadGauge, UnreadTicket};

/// Interval of the noop packets sent on polling while an upgrade is pending (Node's `check`).
pub(crate) const UPGRADE_NOOP_INTERVAL: Duration = Duration::from_millis(100);

/// Why a session ended, with the strings Node's `engine.io` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// The client closed the transport or sent a close packet.
    TransportClose,
    /// A transport error: protocol violation, oversized message, overlapping polls, a failed write, a send buffer
    /// over its limit.
    TransportError,
    /// The client did not answer a ping within `ping_timeout`.
    PingTimeout,
    /// A packet could not be parsed, or a polling `POST` carried too many.
    ParseError,
    /// The server closed the session (`EngineServer::close`, `EngineSocket::close`).
    ForcedClose,
}

impl CloseReason {
    /// The reason as Node's `engine.io` spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            CloseReason::TransportClose => "transport close",
            CloseReason::TransportError => "transport error",
            CloseReason::PingTimeout => "ping timeout",
            CloseReason::ParseError => "parse error",
            CloseReason::ForcedClose => "forced close",
        }
    }
}

impl std::fmt::Display for CloseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a message could not be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// The session is closing or closed.
    #[error("the session is closed")]
    Closed,
    /// The send buffer is over `max_buffered_amount`: the queued packets were discarded and the session closed with
    /// `transport error`.
    #[error("the send buffer is full")]
    Full,
}

impl QueueItem for Packet {
    fn size(&self) -> usize {
        self.polling_len() + 16
    }
}

/// The heartbeat of a v4 session: the server pings after `interval`, waits `timeout` for the pong, and a pong
/// schedules the next ping `interval` later. Pure: it only does arithmetic on the instants it is given.
#[derive(Debug)]
pub(crate) struct Heartbeat {
    interval: Duration,
    timeout: Duration,
    next_ping: Option<Instant>,
    pong_deadline: Option<Instant>,
    refreshed: bool,
}

/// What a heartbeat deadline means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Due {
    /// Send a ping now.
    Ping,
    /// The pong did not arrive: the session is dead.
    Timeout,
}

impl Heartbeat {
    pub(crate) fn new(now: Instant, interval: Duration, timeout: Duration) -> Self {
        Heartbeat { interval, timeout, next_ping: Some(now + interval), pong_deadline: None, refreshed: false }
    }

    /// The next instant something is due.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        match (self.next_ping, self.pong_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// What is due at `now`, if anything; sending the ping starts the wait for its pong.
    pub(crate) fn due(&mut self, now: Instant) -> Option<Due> {
        if self.pong_deadline.is_some_and(|d| d <= now) {
            return Some(Due::Timeout);
        }
        if self.next_ping.is_some_and(|d| d <= now) {
            self.next_ping = None;
            self.pong_deadline = Some(now + self.timeout);
            self.refreshed = false;
            return Some(Due::Ping);
        }
        None
    }

    /// A pong arrived: stop waiting and ping again `interval` from now.
    pub(crate) fn pong(&mut self, now: Instant) {
        self.pong_deadline = None;
        self.next_ping = Some(now + self.interval);
    }

    /// Any other packet arrived: while a pong is awaited, the wait is extended once (Node's
    /// `hasRefreshedPingTimeout`), since the client is evidently alive.
    pub(crate) fn activity(&mut self, now: Instant) {
        if self.pong_deadline.is_some() && !self.refreshed {
            self.pong_deadline = Some(now + self.timeout);
            self.refreshed = true;
        }
    }

    pub(crate) fn stop(&mut self) {
        self.next_ping = None;
        self.pong_deadline = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ready {
    Open,
    /// `EngineSocket::close`: queued packets are flushed, then the session closes with `forced close`.
    Closing,
    Closed,
}

/// What a pending poll is answered with when the session wants it back without data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PollCmd {
    Noop,
    Close,
}

enum Upgrade {
    None,
    Probing { id: u64, deadline: Instant, next_noop: Option<Instant>, cancel: watch::Sender<bool> },
    Upgraded,
}

struct State {
    ready: Ready,
    transport: TransportKind,
    upgrade: Upgrade,
    heartbeat: Heartbeat,
    get_pending: bool,
    post_active: bool,
    cmd: Option<PollCmd>,
    closing_deadline: Option<Instant>,
    next_candidate: u64,
}

/// A WebSocket that asked to take over a polling session.
pub(crate) struct Candidate {
    pub(crate) id: u64,
    pub(crate) cancel: watch::Receiver<bool>,
}

/// What a polling `GET` is answered with.
pub(crate) enum Polled {
    /// Packets to send (at least one), and whether the session is closing after them.
    Packets(Vec<Packet>),
    /// Answer a single noop (an upgrade, or the client closed).
    Noop,
    /// Answer a close packet.
    Close,
    /// A second `GET` while one was pending: the session was closed.
    Overlap,
}

pub(crate) struct Session {
    pub(crate) id: String,
    /// A per-process number for logs: the id is a credential and is never logged.
    pub(crate) log_id: u64,
    pub(crate) options: Arc<EngineOptions>,
    server: Weak<ServerInner>,
    pub(crate) handshake: Request,
    pub(crate) ip: String,
    pub(crate) outbox: Arc<OutQueue<Packet>>,
    pub(crate) unread: Arc<UnreadGauge>,
    inbox: Mutex<Option<mpsc::UnboundedSender<(EngineMessage, UnreadTicket)>>>,
    inbox_rx: Mutex<Option<mpsc::UnboundedReceiver<(EngineMessage, UnreadTicket)>>>,
    state: Mutex<State>,
    timer: Notify,
    poll_wake: Notify,
    closed: watch::Sender<Option<CloseReason>>,
}

static LOG_IDS: AtomicU64 = AtomicU64::new(1);

impl Session {
    /// Creates the session, queues its open packet and starts its timer task.
    pub(crate) fn open(
        server: Weak<ServerInner>,
        id: String,
        transport: TransportKind,
        handshake: Request,
        options: Arc<EngineOptions>,
    ) -> Arc<Session> {
        let (inbox, rx) = mpsc::unbounded_channel();
        let ip = handshake.ip();
        let now = Instant::now();
        let session = Arc::new(Session {
            id,
            log_id: LOG_IDS.fetch_add(1, Ordering::Relaxed),
            server,
            handshake,
            ip,
            outbox: Arc::new(OutQueue::new(options.max_buffered_amount)),
            unread: Arc::new(UnreadGauge::new(options.max_unread_bytes)),
            inbox: Mutex::new(Some(inbox)),
            inbox_rx: Mutex::new(Some(rx)),
            state: Mutex::new(State {
                ready: Ready::Open,
                transport,
                upgrade: if transport == TransportKind::WebSocket { Upgrade::Upgraded } else { Upgrade::None },
                heartbeat: Heartbeat::new(now, options.ping_interval, options.ping_timeout),
                get_pending: false,
                post_active: false,
                cmd: None,
                closing_deadline: None,
                next_candidate: 1,
            }),
            timer: Notify::new(),
            poll_wake: Notify::new(),
            closed: watch::channel(None).0,
            options,
        });
        let open = serde_json::json!({
            "sid": session.id,
            "upgrades": if transport == TransportKind::Polling { vec!["websocket"] } else { Vec::new() },
            "pingInterval": session.options.ping_interval.as_millis() as u64,
            "pingTimeout": session.options.ping_timeout.as_millis() as u64,
            "maxPayload": session.options.max_http_buffer_size,
        });
        // The queue is empty and nothing is pending: the open packet is always accepted.
        let _ = session.outbox.push(Packet::Open(open.to_string()), None);
        tokio::spawn(timer_task(session.clone()));
        session
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn transport(&self) -> TransportKind {
        self.lock().transport
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.lock().ready == Ready::Closed
    }

    pub(crate) fn ready(&self) -> Ready {
        self.lock().ready
    }

    pub(crate) fn closed_watch(&self) -> watch::Receiver<Option<CloseReason>> {
        self.closed.subscribe()
    }

    pub(crate) fn close_reason(&self) -> Option<CloseReason> {
        *self.closed.borrow()
    }

    pub(crate) fn take_inbox(&self) -> Option<mpsc::UnboundedReceiver<(EngineMessage, UnreadTicket)>> {
        self.inbox_rx.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    // ------------------------------------------------------------------ sending

    /// Queues a message. A send that would pass `max_buffered_amount` fails, discards the queue and closes the
    /// session with `transport error`.
    pub(crate) fn send(&self, message: EngineMessage) -> Result<(), SendError> {
        self.queue(Packet::Message(message))
    }

    fn queue(&self, packet: Packet) -> Result<(), SendError> {
        if self.lock().ready != Ready::Open {
            return Err(SendError::Closed);
        }
        match self.outbox.push(packet, None) {
            Ok(()) => Ok(()),
            Err(PushError::Closed) => Err(SendError::Closed),
            Err(PushError::Full) => {
                Logger::new("EngineIO").warn(format!("session {}: send buffer over its limit, session closed", self.log_id));
                self.close(CloseReason::TransportError);
                Err(SendError::Full)
            }
        }
    }

    // ----------------------------------------------------------------- receiving

    /// A packet arrived on the active transport.
    pub(crate) fn on_packet(&self, packet: Packet) {
        let now = Instant::now();
        {
            let mut s = self.lock();
            if s.ready != Ready::Open {
                return;
            }
            match &packet {
                Packet::Pong(_) => {
                    s.heartbeat.pong(now);
                    drop(s);
                    self.timer.notify_waiters();
                    return;
                }
                _ => s.heartbeat.activity(now),
            }
        }
        self.timer.notify_waiters();
        match packet {
            // v4: only the server pings.
            Packet::Ping(_) => self.close(CloseReason::TransportError),
            Packet::Message(m) => {
                let ticket = UnreadTicket::new(&self.unread, m.len());
                if let Some(tx) = self.inbox.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    let _ = tx.send((m, ticket));
                }
            }
            Packet::Open(_) | Packet::Close | Packet::Pong(_) | Packet::Upgrade | Packet::Noop => {}
        }
    }

    /// Something that is not a packet arrived on the active transport.
    pub(crate) fn on_invalid(&self) {
        self.close(CloseReason::ParseError);
    }

    // ------------------------------------------------------------------- polling

    /// Marks a `POST` as being read. `None` when another one is (the session is then closed, as Node does).
    pub(crate) fn begin_post(self: &Arc<Self>) -> Option<PostGuard> {
        let mut s = self.lock();
        if s.ready == Ready::Closed {
            return None;
        }
        if s.post_active {
            drop(s);
            self.close(CloseReason::TransportError);
            return None;
        }
        s.post_active = true;
        Some(PostGuard { session: self.clone(), done: false })
    }

    /// Waits for a pending `GET`'s turn: answered with queued packets, or the command the session left for it.
    pub(crate) async fn poll(self: &Arc<Self>) -> Polled {
        {
            let mut s = self.lock();
            if s.get_pending {
                drop(s);
                self.close(CloseReason::TransportError);
                return Polled::Overlap;
            }
            s.get_pending = true;
        }
        let mut guard = PollGuard { session: self.clone(), done: false };
        let mut closed = self.closed.subscribe();
        loop {
            let woken = self.poll_wake.notified();
            if let Some(answer) = self.try_poll() {
                guard.done = true;
                return answer;
            }
            tokio::select! {
                _ = self.outbox.wait_ready() => {}
                _ = woken => {}
                _ = closed.changed() => {}
            }
        }
    }

    fn try_poll(&self) -> Option<Polled> {
        let mut s = self.lock();
        if let Some(cmd) = s.cmd.take() {
            // A close answers once; later polls find the session gone.
            return Some(match cmd {
                PollCmd::Noop => Polled::Noop,
                PollCmd::Close => Polled::Close,
            });
        }
        if s.ready == Ready::Closed {
            return Some(Polled::Close);
        }
        if s.transport != TransportKind::Polling {
            return Some(Polled::Noop);
        }
        let max = self.options.max_http_buffer_size;
        let mut batch = self.outbox.take_batch(max, |p| p.polling_len() + 1);
        if batch.is_empty() {
            if self.outbox.is_drained() {
                // Closing and nothing left: the client is told to close.
                s.ready = Ready::Closed;
                drop(s);
                self.finish(CloseReason::ForcedClose);
                return Some(Polled::Close);
            }
            return None;
        }
        if self.outbox.is_drained() {
            batch.push(Packet::Close);
            s.ready = Ready::Closed;
            drop(s);
            self.finish(CloseReason::ForcedClose);
        }
        Some(Polled::Packets(batch))
    }

    // ------------------------------------------------------------------- upgrade

    /// A WebSocket with this session's id arrived. `None` when the session is closed, or an upgrade is in
    /// progress or done: the new WebSocket is then closed.
    pub(crate) fn begin_candidate(&self) -> Option<Candidate> {
        let mut s = self.lock();
        if s.ready != Ready::Open || !matches!(s.upgrade, Upgrade::None) {
            return None;
        }
        let id = s.next_candidate;
        s.next_candidate += 1;
        let (cancel, rx) = watch::channel(false);
        s.upgrade = Upgrade::Probing { id, deadline: Instant::now() + self.options.upgrade_timeout, next_noop: None, cancel };
        drop(s);
        self.timer.notify_waiters();
        Some(Candidate { id, cancel: rx })
    }

    /// The candidate sent `2probe`: from now on a noop goes to a pending poll every 100 ms.
    pub(crate) fn candidate_probed(&self, id: u64) {
        let mut s = self.lock();
        if let Upgrade::Probing { id: current, next_noop, .. } = &mut s.upgrade
            && *current == id
        {
            *next_noop = Some(Instant::now() + UPGRADE_NOOP_INTERVAL);
        }
        drop(s);
        self.timer.notify_waiters();
    }

    /// The candidate sent `5`: it becomes the active transport. `false` when it is no longer the candidate.
    pub(crate) fn candidate_upgraded(&self, id: u64) -> bool {
        let mut s = self.lock();
        let ours = matches!(&s.upgrade, Upgrade::Probing { id: current, .. } if *current == id);
        if !ours || s.ready == Ready::Closed {
            return false;
        }
        s.upgrade = Upgrade::Upgraded;
        s.transport = TransportKind::WebSocket;
        // A poll that is still pending is released: polling stops draining the outbox.
        s.cmd = Some(PollCmd::Noop);
        drop(s);
        self.poll_wake.notify_waiters();
        self.timer.notify_waiters();
        true
    }

    /// The candidate ended without upgrading (any other packet, its socket closed, the timeout).
    pub(crate) fn candidate_failed(&self, id: u64) {
        let mut s = self.lock();
        if matches!(&s.upgrade, Upgrade::Probing { id: current, .. } if *current == id) {
            s.upgrade = Upgrade::None;
        }
        drop(s);
        self.timer.notify_waiters();
    }

    // --------------------------------------------------------------------- close

    /// `EngineSocket::close`: queued packets are written, then the session closes with `forced close`.
    pub(crate) fn close_graceful(&self) {
        let mut s = self.lock();
        if s.ready != Ready::Open {
            return;
        }
        s.ready = Ready::Closing;
        s.closing_deadline = Some(Instant::now() + self.options.close_timeout);
        drop(s);
        self.outbox.seal(false);
        self.timer.notify_waiters();
    }

    /// Ends the session now, discarding what was queued. Idempotent: the first reason wins.
    pub(crate) fn close(&self, reason: CloseReason) {
        {
            let mut s = self.lock();
            if s.ready == Ready::Closed {
                return;
            }
            s.ready = Ready::Closed;
        }
        self.finish(reason);
    }

    /// Everything after `ready` became `Closed`.
    fn finish(&self, reason: CloseReason) {
        {
            let mut s = self.lock();
            s.heartbeat.stop();
            s.closing_deadline = None;
            // A pending poll is answered with a noop when the client closed, else with a close packet.
            s.cmd = Some(if reason == CloseReason::TransportClose { PollCmd::Noop } else { PollCmd::Close });
            if let Upgrade::Probing { cancel, .. } = &s.upgrade {
                let _ = cancel.send(true);
            }
            s.upgrade = Upgrade::None;
        }
        self.outbox.seal(true);
        // What was received stays readable; the stream ends after it.
        self.inbox.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.closed.send_replace(Some(reason));
        self.poll_wake.notify_waiters();
        self.timer.notify_waiters();
        if let Some(server) = self.server.upgrade() {
            server.remove(&self.id);
        }
    }

    // --------------------------------------------------------------------- timer

    /// The next instant the timer task has to act.
    fn next_deadline(&self) -> Option<Instant> {
        let s = self.lock();
        if s.ready == Ready::Closed {
            return None;
        }
        let mut next = s.heartbeat.deadline();
        let mut consider = |t: Option<Instant>| {
            if let Some(t) = t {
                next = Some(next.map_or(t, |n| n.min(t)));
            }
        };
        if let Upgrade::Probing { deadline, next_noop, .. } = &s.upgrade {
            consider(Some(*deadline));
            consider(*next_noop);
        }
        consider(s.closing_deadline);
        next
    }

    /// Runs what is due at `now`.
    fn on_tick(&self, now: Instant) {
        enum Act {
            Ping,
            Timeout,
            Forced,
        }
        let mut acts = Vec::new();
        {
            let mut s = self.lock();
            if s.ready == Ready::Closed {
                return;
            }
            match s.heartbeat.due(now) {
                Some(Due::Ping) => acts.push(Act::Ping),
                Some(Due::Timeout) => acts.push(Act::Timeout),
                None => {}
            }
            let mut abandon = false;
            let mut noop = false;
            if let Upgrade::Probing { deadline, next_noop, .. } = &mut s.upgrade {
                if *deadline <= now {
                    abandon = true;
                } else if next_noop.is_some_and(|t| t <= now) {
                    *next_noop = Some(now + UPGRADE_NOOP_INTERVAL);
                    noop = true;
                }
            }
            if abandon {
                if let Upgrade::Probing { cancel, .. } = &s.upgrade {
                    let _ = cancel.send(true);
                }
                s.upgrade = Upgrade::None;
            }
            if noop && s.get_pending && s.cmd.is_none() {
                s.cmd = Some(PollCmd::Noop);
                self.poll_wake.notify_waiters();
            }
            if s.closing_deadline.is_some_and(|t| t <= now) {
                acts.push(Act::Forced);
            }
        }
        for act in acts {
            match act {
                Act::Ping => {
                    // A ping goes through the same bounded outbox as everything else.
                    let _ = self.queue_ping();
                }
                Act::Timeout => self.close(CloseReason::PingTimeout),
                Act::Forced => self.close(CloseReason::ForcedClose),
            }
        }
    }

    fn queue_ping(&self) -> Result<(), SendError> {
        match self.outbox.push(Packet::Ping(String::new()), None) {
            Ok(()) => Ok(()),
            Err(PushError::Closed) => Err(SendError::Closed),
            Err(PushError::Full) => {
                self.close(CloseReason::TransportError);
                Err(SendError::Full)
            }
        }
    }
}

/// Holds the "one `POST` at a time" slot. Dropped before `finish`, the request was cancelled: Node closes the
/// session with `transport error` then.
pub(crate) struct PostGuard {
    session: Arc<Session>,
    done: bool,
}

impl PostGuard {
    pub(crate) fn finish(mut self) {
        self.done = true;
    }
}

impl Drop for PostGuard {
    fn drop(&mut self) {
        self.session.lock().post_active = false;
        if !self.done {
            self.session.close(CloseReason::TransportError);
        }
    }
}

/// Holds the "one pending `GET`" slot; a `GET` cancelled while waiting closes the session, as in Node.
struct PollGuard {
    session: Arc<Session>,
    done: bool,
}

impl Drop for PollGuard {
    fn drop(&mut self) {
        self.session.lock().get_pending = false;
        if !self.done {
            self.session.close(CloseReason::TransportError);
        }
    }
}

async fn timer_task(session: Arc<Session>) {
    let mut closed = session.closed.subscribe();
    loop {
        let changed = session.timer.notified();
        let Some(deadline) = session.next_deadline() else {
            if session.is_closed() {
                return;
            }
            tokio::select! {
                _ = changed => continue,
                _ = closed.changed() => continue,
            }
        };
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => session.on_tick(Instant::now()),
            _ = changed => {}
            _ = closed.changed() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_timer_logic_with_the_default_intervals() {
        let start = t0();
        let mut hb = Heartbeat::new(start, Duration::from_secs(25), Duration::from_secs(20));
        // Nothing is due before the first interval.
        assert_eq!(hb.deadline(), Some(start + Duration::from_secs(25)));
        assert_eq!(hb.due(start + Duration::from_secs(24)), None);
        // The first ping is due at 25 s and starts the wait for its pong.
        assert_eq!(hb.due(start + Duration::from_secs(25)), Some(Due::Ping));
        assert_eq!(hb.deadline(), Some(start + Duration::from_secs(45)));
        // A pong at 30 s schedules the next ping at 55 s.
        hb.pong(start + Duration::from_secs(30));
        assert_eq!(hb.deadline(), Some(start + Duration::from_secs(55)));
        assert_eq!(hb.due(start + Duration::from_secs(54)), None);
        assert_eq!(hb.due(start + Duration::from_secs(55)), Some(Due::Ping));
        // No pong after the ping at 55 s closes the session at 75 s, not before.
        assert_eq!(hb.due(start + Duration::from_secs(74)), None);
        assert_eq!(hb.due(start + Duration::from_secs(75)), Some(Due::Timeout));
    }

    #[tokio::test(start_paused = true)]
    async fn another_packet_extends_the_pong_wait_once() {
        let start = t0();
        let mut hb = Heartbeat::new(start, Duration::from_secs(25), Duration::from_secs(20));
        assert_eq!(hb.due(start + Duration::from_secs(25)), Some(Due::Ping));
        hb.activity(start + Duration::from_secs(30));
        assert_eq!(hb.deadline(), Some(start + Duration::from_secs(50)));
        // A second packet does not extend it again.
        hb.activity(start + Duration::from_secs(40));
        assert_eq!(hb.deadline(), Some(start + Duration::from_secs(50)));
        assert_eq!(hb.due(start + Duration::from_secs(50)), Some(Due::Timeout));
    }

    #[tokio::test(start_paused = true)]
    async fn activity_without_a_pending_ping_changes_nothing() {
        let start = t0();
        let mut hb = Heartbeat::new(start, Duration::from_secs(25), Duration::from_secs(20));
        hb.activity(start + Duration::from_secs(10));
        assert_eq!(hb.deadline(), Some(start + Duration::from_secs(25)));
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    fn open(options: EngineOptions, transport: TransportKind) -> Arc<Session> {
        let (parts, _) = http::Request::builder().uri("/socket.io/?EIO=4&transport=polling").body(()).unwrap().into_parts();
        let remote = "127.0.0.1:1".parse().unwrap();
        let handshake = Request::for_upgrade(parts, remote, &Default::default()).unwrap();
        // No server: removing the session from the store is a no-op.
        Session::open(Weak::new(), "aaaaaaaaaaaaaaaaaaaa".to_string(), transport, handshake, Arc::new(options))
    }

    fn drain(session: &Session) -> Vec<Packet> {
        session.outbox.take_batch(usize::MAX, |p| p.polling_len() + 1)
    }

    async fn advance(ms: u64) {
        tokio::time::advance(Duration::from_millis(ms)).await;
        // Let the timer task run.
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_session_pings_and_closes_with_ping_timeout_at_exact_times() {
        let options = EngineOptions::new().ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_millis(200));
        let session = open(options, TransportKind::WebSocket);
        // The open packet only.
        assert_eq!(drain(&session).len(), 1);
        advance(299).await;
        assert!(drain(&session).is_empty(), "no ping before the interval");
        advance(1).await;
        assert_eq!(drain(&session), vec![Packet::Ping(String::new())]);
        advance(199).await;
        assert_eq!(session.close_reason(), None, "no timeout before the pong deadline");
        advance(1).await;
        assert_eq!(session.close_reason(), Some(CloseReason::PingTimeout));
    }

    #[tokio::test(start_paused = true)]
    async fn a_pong_keeps_the_session_alive_and_schedules_the_next_ping() {
        let options = EngineOptions::new().ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_millis(200));
        let session = open(options, TransportKind::WebSocket);
        let _ = drain(&session);
        advance(300).await;
        let _ = drain(&session);
        advance(100).await;
        session.on_packet(Packet::Pong(String::new()));
        advance(299).await;
        assert!(drain(&session).is_empty());
        advance(1).await;
        assert_eq!(drain(&session), vec![Packet::Ping(String::new())]);
        assert_eq!(session.close_reason(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn an_upgrade_that_never_completes_is_abandoned_at_upgrade_timeout() {
        let options = EngineOptions::new().upgrade_timeout(Duration::from_secs(10));
        let session = open(options, TransportKind::Polling);
        let candidate = session.begin_candidate().expect("a first candidate");
        advance(9_999).await;
        assert!(!*candidate.cancel.borrow());
        advance(1).await;
        assert!(*candidate.cancel.borrow(), "the candidate is cancelled at the deadline");
        assert_eq!(session.close_reason(), None, "the session is kept");
        // The slot is free again.
        assert!(session.begin_candidate().is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn one_upgrade_at_a_time_and_none_after_the_upgrade() {
        let session = open(EngineOptions::new(), TransportKind::Polling);
        let first = session.begin_candidate().expect("first");
        assert!(session.begin_candidate().is_none(), "second candidate while probing");
        assert!(session.candidate_upgraded(first.id));
        assert_eq!(session.transport(), TransportKind::WebSocket);
        assert!(session.begin_candidate().is_none(), "a candidate after the upgrade");
        // A stale candidate cannot upgrade again.
        assert!(!session.candidate_upgraded(first.id));
    }

    #[tokio::test(start_paused = true)]
    async fn a_pending_poll_gets_a_noop_every_100_ms_while_probing() {
        let session = open(EngineOptions::new(), TransportKind::Polling);
        // The first poll takes the open packet off the outbox.
        assert!(matches!(session.poll().await, Polled::Packets(_)));
        let candidate = session.begin_candidate().unwrap();
        session.candidate_probed(candidate.id);
        for _ in 0..2 {
            let poller = {
                let session = session.clone();
                tokio::spawn(async move { session.poll().await })
            };
            advance(99).await;
            assert!(!poller.is_finished(), "no noop before 100 ms");
            advance(1).await;
            assert!(matches!(poller.await.unwrap(), Polled::Noop));
        }
    }
}
