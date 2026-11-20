//! The socket of a connection as hyper sees it: idle timeouts, the send deadline, the HTTP/1 framing check, read
//! backpressure of upgraded connections and the closing drain.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::{Instant, Sleep};

use super::transport::ServerIo;
use super::{ConnState, ServerOptions, h1_framing};

#[derive(Clone, Copy)]
enum Phase {
    /// Waiting for (the rest of) a request head; counted from the connection start or from the
    /// first byte after an idle keep-alive period.
    Head(Instant),
    /// Idle between requests since the last response ended (or the last byte was written).
    KeepAlive(Instant),
}

/// Which protocol a connection speaks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Proto {
    /// A plain listener: HTTP/1, or HTTP/2 when `h2c` is on, told apart by the first bytes.
    Auto,
    /// TLS with ALPN `http/1.1` (or none): HTTP/1 only, the HTTP/2 preface is refused.
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    H1Only,
    /// TLS with ALPN `h2`: HTTP/2 only.
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    H2Only,
}

const H2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// Frame boundaries of one direction of an HTTP/2 connection: which kinds of frame the bytes that pass belong to.
#[derive(Default)]
struct H2Frames {
    head: [u8; 9],
    head_len: usize,
    payload_left: usize,
    kind: u8,
}

const H2_DATA: u16 = 1 << 0x0;
const H2_HEADERS: u16 = 1 << 0x1;
const H2_CONTINUATION: u16 = 1 << 0x9;

impl H2Frames {
    /// Walks `data`; the bit of every frame kind (`1 << type`) that has a byte in it. A frame header counts for its
    /// frame once complete.
    fn scan(&mut self, mut data: &[u8]) -> u16 {
        let mut kinds = 0u16;
        while !data.is_empty() {
            if self.payload_left > 0 {
                let n = data.len().min(self.payload_left);
                self.payload_left -= n;
                data = &data[n..];
                kinds |= 1 << self.kind.min(15);
                continue;
            }
            let n = data.len().min(9 - self.head_len);
            self.head[self.head_len..self.head_len + n].copy_from_slice(&data[..n]);
            self.head_len += n;
            data = &data[n..];
            if self.head_len == 9 {
                self.head_len = 0;
                self.payload_left = u32::from_be_bytes([0, self.head[0], self.head[1], self.head[2]]) as usize;
                self.kind = self.head[3];
                kinds |= 1 << self.kind.min(15);
            }
        }
        kinds
    }
}

/// The socket of a connection, with the `headersTimeout` / `keepAliveTimeout` of Node: while no
/// request is in flight, a read that waits past the deadline fails and hyper closes the
/// connection. Requests in flight are not limited here (the body read has `requestTimeout`).
///
/// When the connection is dropped by the server, it is closed gracefully (see [`linger_close`]) so
/// that an error response sent before the request was fully read is not lost to a TCP reset.
pub(super) struct TimedIo {
    /// `None` only while `Drop` hands the socket to the closing task.
    io: Option<ServerIo>,
    /// Held until the socket is really closed, so `max_connections` also counts closing sockets.
    permit: Option<OwnedSemaphorePermit>,
    state: Arc<ConnState>,
    headers_timeout: Option<Duration>,
    keep_alive_timeout: Option<Duration>,
    phase: Phase,
    seen_generation: u64,
    sleep: Option<Pin<Box<Sleep>>>,
    /// Bytes of the HTTP/2 preface matched so far (`None` = done: not a preface, or an h2c connection).
    preface: Option<usize>,
    /// h2c enabled: the preface was seen and the frames that follow are tracked, so only a HEADERS frame (a request)
    /// ends the idle keep-alive phase.
    h2: Option<H2Frames>,
    /// The frames the server writes, tracked once the connection is HTTP/2: only DATA, HEADERS and CONTINUATION
    /// refresh the keep-alive phase (a PONG or a SETTINGS acknowledgement does not make a connection busy).
    h2_out: Option<H2Frames>,
    h2c: bool,
    /// TLS with ALPN `h2`: the connection must start with the HTTP/2 preface.
    h2_only: bool,
    send_timeout: Option<Duration>,
    /// Since when `poll_write` or `poll_flush` have been pending without the socket accepting a byte.
    write_stalled_since: Option<Instant>,
    write_sleep: Option<Pin<Box<Sleep>>>,
    /// The last write was pending (a flush that completes does not mean the socket accepts bytes).
    write_blocked: bool,
    /// Upgraded connections stop reading while a write is blocked (see `read_paused`): the task parked in
    /// `poll_read`, and the task whose write is pending.
    read_waker: Option<Waker>,
    write_waker: Option<Waker>,
    /// How many bytes an upgraded connection may read while a write is blocked.
    read_credit: ReadCredit,
    /// The deadline of the parked reader, separate from `write_sleep` (a `Sleep` keeps one waker: sharing it
    /// would take the wake-up away from the task whose write is pending).
    read_sleep: Option<Pin<Box<Sleep>>>,
    /// The send timeout fired: an upgraded connection fails its reads too.
    send_failed: bool,
    /// Bytes TCP had accepted when the send deadline last restarted (TLS: the progress shows below rustls).
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    seen_accepted: u64,
    /// The HTTP/1 framing check; `None` with `insecure_http_parser(true)`.
    framing: Option<Box<h1_framing::Framing>>,
    /// Bytes the check released that did not fit in hyper's buffer yet, served before the socket.
    queue: Vec<u8>,
    queue_pos: usize,
    /// The socket reported end of input.
    sock_eof: bool,
}

/// Most credit an upgraded connection keeps: enough for a client that writes a moderately large message before it
/// reads, small enough to bound what a ping flood can make the protocol layer queue as replies.
pub(super) const READ_CREDIT_MAX: usize = 64 * 1024;

/// The read credit of an upgraded connection: the bytes it may read while a write is blocked. Every byte the socket
/// accepts earns a byte (up to [`READ_CREDIT_MAX`]) and every byte read spends one, so what a peer can make the
/// server read during blocked writes never exceeds what the socket took plus the maximum, however small the slices
/// the socket accepts are. That bounds the replies (pongs) queued behind a peer that does not read.
#[derive(Debug, Clone, Copy)]
pub(super) struct ReadCredit(usize);

impl ReadCredit {
    pub(super) fn new() -> Self {
        ReadCredit(READ_CREDIT_MAX)
    }
    pub(super) fn earn(&mut self, written: usize) {
        self.0 = self.0.saturating_add(written).min(READ_CREDIT_MAX);
    }
    pub(super) fn spend(&mut self, read: usize) {
        self.0 = self.0.saturating_sub(read);
    }
    pub(super) fn exhausted(&self) -> bool {
        self.0 == 0
    }
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    pub(super) fn available(&self) -> usize {
        self.0
    }
}

impl Default for ReadCredit {
    fn default() -> Self {
        Self::new()
    }
}

/// Most input the server reads and discards after an early error response before closing.
const LINGER_MAX_BYTES: usize = 1024 * 1024;
/// Longest the server waits for the peer to finish (or stop) sending after an early error response.
const LINGER_MAX_TIME: Duration = Duration::from_secs(2);

/// Closes a socket the way Node does after answering before the request was fully read: the
/// response is already flushed, so send FIN, then read and discard what the peer still sends until
/// it closes, `LINGER_MAX_BYTES` or `LINGER_MAX_TIME`. Dropping a socket that still has unread
/// input makes Windows (and Linux) send a reset, and the peer then discards the response it has
/// not read yet. Both bounds keep a peer from holding the connection (and its connection permit).
pub(super) async fn linger_close(mut io: ServerIo) {
    let drain = async {
        let _ = tokio::io::AsyncWriteExt::shutdown(&mut io).await;
        let mut buf = [0u8; 4 * 1024];
        let mut total = 0usize;
        while total < LINGER_MAX_BYTES {
            match io.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => total += n,
            }
        }
    };
    let _ = tokio::time::timeout(LINGER_MAX_TIME, drain).await;
}

impl Drop for TimedIo {
    fn drop(&mut self) {
        if let Some(io) = self.io.take() {
            let permit = self.permit.take();
            *self.state.end.closing.lock().unwrap_or_else(|e| e.into_inner()) = Some((io, permit));
        }
    }
}

impl TimedIo {
    pub(super) fn new(
        io: ServerIo,
        permit: Option<OwnedSemaphorePermit>,
        state: Arc<ConnState>,
        options: &ServerOptions,
        proto: Proto,
    ) -> Self {
        let framing = match proto {
            Proto::H2Only => None,
            _ => (!options.insecure_http_parser).then(|| Box::new(h1_framing::Framing::new(options.max_header_size.max(8 * 1024)))),
        };
        TimedIo {
            io: Some(io),
            permit,
            state,
            headers_timeout: options.headers_timeout,
            keep_alive_timeout: options.keep_alive_timeout,
            phase: Phase::Head(Instant::now()),
            seen_generation: 0,
            sleep: None,
            preface: Some(0),
            h2: None,
            h2_out: None,
            // HTTP/2 negotiated with ALPN starts with the preface too, and is always allowed; HTTP/1 over TLS never is.
            h2c: match proto {
                Proto::Auto => options.h2c,
                Proto::H1Only => false,
                Proto::H2Only => true,
            },
            h2_only: proto == Proto::H2Only,
            send_timeout: options.send_timeout,
            write_stalled_since: None,
            write_sleep: None,
            write_blocked: false,
            read_waker: None,
            write_waker: None,
            read_credit: ReadCredit::new(),
            read_sleep: None,
            send_failed: false,
            framing,
            seen_accepted: 0,
            queue: Vec::new(),
            queue_pos: 0,
            sock_eof: false,
        }
    }

    /// Notices requests that started or ended since the last poll.
    fn sync(&mut self) {
        let g = self.state.activity.generation.load(Ordering::SeqCst);
        if g != self.seen_generation {
            self.seen_generation = g;
            if self.state.activity.inflight.load(Ordering::SeqCst) == 0 {
                self.phase = Phase::KeepAlive(Instant::now());
            }
        }
    }

    fn sock(&mut self) -> Pin<&mut ServerIo> {
        Pin::new(self.io.as_mut().expect("socket is present until drop"))
    }

    fn on_read(&mut self, data: &[u8]) -> io::Result<()> {
        self.sync();
        // An HTTP/2 connection leaves the keep-alive phase only when a request starts; other frames after the last
        // response (WINDOW_UPDATE, PING, SETTINGS) must not swap `keep_alive_timeout` for `headers_timeout`.
        let mut request = self.h2.is_none();
        if let Some(matched) = self.preface {
            let n = data.len().min(H2_PREFACE.len() - matched);
            if data[..n] != H2_PREFACE[matched..matched + n] {
                if self.h2_only {
                    // ALPN negotiated `h2`: anything but the HTTP/2 preface is a protocol violation, and must not be
                    // parsed as HTTP/1 (the framing check is off on this path).
                    self.state.end.no_linger.store(true, Ordering::SeqCst);
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "HTTP/2 connection preface expected"));
                }
                self.preface = None;
            } else if matched + n == H2_PREFACE.len() {
                if !self.h2c {
                    self.state.end.no_linger.store(true, Ordering::SeqCst);
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "HTTP/2 (h2c) is disabled"));
                }
                self.preface = None;
                self.h2_out = Some(H2Frames::default());
                let mut frames = H2Frames::default();
                request = frames.scan(&data[n..]) & H2_HEADERS != 0;
                self.h2 = Some(frames);
            } else {
                self.preface = Some(matched + n);
            }
        } else if let Some(frames) = self.h2.as_mut() {
            request = frames.scan(data) & H2_HEADERS != 0;
        }
        if request && let Phase::KeepAlive(_) = self.phase {
            self.phase = Phase::Head(Instant::now());
        }
        Ok(())
    }

    /// A write or flush is pending: fails with `TimedOut` once the socket has accepted nothing for `send_timeout`
    /// (the client does not read), after which the connection is closed without the drain. Pending otherwise, with
    /// the task woken at the deadline.
    fn poll_write_deadline(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let Some(timeout) = self.send_timeout else { return Poll::Pending };
        self.sync_wire_progress();
        let since = *self.write_stalled_since.get_or_insert_with(Instant::now);
        let Some(deadline) = since.checked_add(timeout) else { return Poll::Pending };
        let sleep = self.write_sleep.get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if sleep.deadline() != deadline {
            sleep.as_mut().reset(deadline);
        }
        match sleep.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(self.send_timed_out())),
            Poll::Pending => Poll::Pending,
        }
    }

    /// Under TLS a flush can stay pending while TCP keeps taking bytes: whatever TCP accepted since the last look is
    /// progress, so the send deadline restarts from now.
    fn sync_wire_progress(&mut self) {
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
            let accepted = t.wire.accepted();
            if accepted != self.seen_accepted {
                self.seen_accepted = accepted;
                self.write_stalled_since = None;
            }
        }
    }

    fn is_tls(&self) -> bool {
        #[cfg(feature = "tls")]
        {
            matches!(self.io, Some(ServerIo::Tls(_)))
        }
        #[cfg(not(feature = "tls"))]
        {
            false
        }
    }

    /// Whether the last write is blocked. Under TLS that is the last TCP write, or ciphertext rustls still holds.
    fn write_is_blocked(&self) -> bool {
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
            return t.wire.blocked() || t.wants_write();
        }
        self.write_blocked
    }

    fn read_credit_exhausted(&self) -> bool {
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
            return t.wire.credit_exhausted();
        }
        self.read_credit.exhausted()
    }

    /// Pushes the ciphertext rustls holds to the socket. `true` when nothing is left pending (or the flush failed,
    /// which the next read reports); `false` when the socket does not take it yet. The task whose write is pending
    /// keeps the socket's write waker.
    fn drain_tls(&mut self, cx: &mut Context<'_>) -> bool {
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_mut()
            && t.wants_write()
        {
            let waker = self.write_waker.clone().unwrap_or_else(|| cx.waker().clone());
            let mut wcx = Context::from_waker(&waker);
            return !Pin::new(&mut t.stream).poll_flush(&mut wcx).is_pending();
        }
        let _ = cx;
        true
    }

    /// The send timeout fired: mark the socket for an abortive close and wake both halves of an upgraded
    /// connection, so the one that is not polling the deadline sees the failure too.
    fn send_timed_out(&mut self) -> io::Error {
        self.state.end.no_linger.store(true, Ordering::SeqCst);
        self.send_failed = true;
        for w in [self.read_waker.take(), self.write_waker.take()].into_iter().flatten() {
            w.wake();
        }
        io::Error::new(io::ErrorKind::TimedOut, "send timeout")
    }

    /// The socket accepted bytes: the client reads.
    fn write_progress(&mut self, written: &[io::IoSlice<'_>], n: usize) {
        self.write_stalled_since = None;
        self.write_blocked = false;
        // Under TLS the credit is earned by the bytes TCP accepts (the gate), not by plaintext rustls buffers.
        if !self.is_tls() {
            self.read_credit.earn(n);
        }
        if let Some(w) = self.read_waker.take() {
            w.wake();
        }
        self.sync();
        // A response still being flushed is progress, not idleness. Over HTTP/2 only the frames of a response count:
        // PONGs, SETTINGS acknowledgements and the like would keep an idle connection open for ever.
        let response = match self.h2_out.as_mut() {
            None => true,
            Some(frames) => {
                let mut left = n;
                let mut kinds = 0;
                for part in written {
                    if left == 0 {
                        break;
                    }
                    let take = left.min(part.len());
                    kinds |= frames.scan(&part[..take]);
                    left -= take;
                }
                kinds & (H2_DATA | H2_HEADERS | H2_CONTINUATION) != 0
            }
        };
        if response && let Phase::KeepAlive(_) = self.phase {
            self.phase = Phase::KeepAlive(Instant::now());
        }
    }

    fn poll_idle(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.sync();
        if self.state.end.upgraded.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        if self.state.activity.inflight.load(Ordering::SeqCst) > 0 {
            *self.state.activity.waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
            // The request may have ended in between: look again.
            if self.state.activity.inflight.load(Ordering::SeqCst) > 0 {
                return Poll::Pending;
            }
            self.sync();
        }
        let (since, timeout) = match self.phase {
            Phase::Head(t) => (t, self.headers_timeout),
            Phase::KeepAlive(t) => (t, self.keep_alive_timeout),
        };
        // A timeout too long to add to an `Instant` never fires.
        let Some(deadline) = timeout.and_then(|t| since.checked_add(t)) else { return Poll::Pending };
        let sleep = self.sleep.get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if sleep.deadline() != deadline {
            sleep.as_mut().reset(deadline);
        }
        match sleep.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.state.end.no_linger.store(true, Ordering::SeqCst);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "connection idle timeout")))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl TimedIo {
    /// Tells whether `done` holds; if not, registers the connection task to be woken when a request
    /// finishes and looks again, so that a request finishing in between is not missed.
    fn wait_for_requests(&mut self, cx: &mut Context<'_>, done: impl Fn(&ConnState) -> bool) -> bool {
        if done(&self.state) {
            return true;
        }
        *self.state.activity.waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
        done(&self.state)
    }

    /// `poll_read` with the framing check: bytes reach hyper only when the check let them through.
    fn poll_read_checked(&mut self, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        // A read into an empty buffer cannot tell end of input from "no room": do not read.
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if self.queue_pos < self.queue.len() {
                let n = buf.remaining().min(self.queue.len() - self.queue_pos);
                buf.put_slice(&self.queue[self.queue_pos..self.queue_pos + n]);
                self.queue_pos += n;
                if self.queue_pos == self.queue.len() {
                    self.queue.clear();
                    self.queue_pos = 0;
                    if self.queue.capacity() > 64 * 1024 {
                        self.queue = Vec::new();
                    }
                }
                return Poll::Ready(Ok(()));
            }
            let state = self.state.clone();
            let sock_eof = self.sock_eof;
            let fr = self.framing.as_mut().expect("the framing check is on");
            if fr.is_holding() {
                // After an `Upgrade` request the bytes may not be HTTP: wait for its answer.
                let need = fr.heads();
                // `finished` first, then `upgraded`: a 101 sets `upgraded` before its request finishes, so a request
                // seen as finished here has its `upgraded` flag visible below and cannot be mistaken for a non-101.
                let finished = state.activity.finished.load(Ordering::SeqCst) >= need;
                let released = state.end.upgraded.load(Ordering::SeqCst);
                if released || finished {
                    if released {
                        fr.release(&mut self.queue);
                    } else {
                        fr.resume(&mut self.queue);
                    }
                    if sock_eof {
                        fr.eof(&mut self.queue);
                    }
                    continue;
                }
                let full = fr.hold_full();
                if self
                    .wait_for_requests(cx, |s| s.end.upgraded.load(Ordering::SeqCst) || s.activity.finished.load(Ordering::SeqCst) >= need)
                {
                    continue;
                }
                if full || sock_eof {
                    return self.poll_idle(cx);
                }
            } else if let Some(t) = fr.pending_terminal() {
                if state.activity.finished.load(Ordering::SeqCst) >= t.wait {
                    fr.send_terminal(&mut self.queue);
                    continue;
                }
                if self.wait_for_requests(cx, |s| s.activity.finished.load(Ordering::SeqCst) >= t.wait) {
                    continue;
                }
                return self.poll_idle(cx);
            } else if fr.terminal_sent() || sock_eof {
                return Poll::Ready(Ok(()));
            }
            let before = buf.filled().len();
            match self.sock().poll_read(cx, buf) {
                Poll::Pending => return self.poll_idle(cx),
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => {
                    let n = buf.filled().len() - before;
                    if n == 0 {
                        self.sock_eof = true;
                        if let Some(fr) = self.framing.as_mut() {
                            fr.eof(&mut self.queue);
                        }
                        continue;
                    }
                    self.on_read(&buf.filled()[before..])?;
                    let fr = self.framing.as_mut().expect("the framing check is on");
                    let region = &buf.filled()[before..];
                    let k = fr.passthrough_prefix(region);
                    if k < n {
                        fr.feed(&region[k..], &mut self.queue);
                    }
                    buf.set_filled(before + k);
                    if k > 0 {
                        return Poll::Ready(Ok(()));
                    }
                }
            }
        }
    }
}

impl TimedIo {
    /// Read backpressure of upgraded connections: while a write is blocked the peer is not read once
    /// the read credit ([`ReadCredit`]) is spent, so frames that need a reply (WebSocket pings answered with
    /// pongs) cannot pile up in the protocol's write buffer behind a peer that does not read. `Some` is the result
    /// of the read poll while paused, `None` means read normally.
    ///
    /// Nothing here registers a waker in a place the writer also uses: the socket is probed with a zero-byte
    /// `try_write` (it reports the readiness the driver already knows, without a waker), and the parked reader has
    /// its own timer. The reader is woken by `write_progress`, by the timer, or by the writer's own wake-up when the
    /// socket becomes writable (a protocol layer that wakes both halves, like tokio-tungstenite, wakes it too).
    fn read_paused(&mut self, cx: &mut Context<'_>) -> Option<Poll<io::Result<()>>> {
        if !self.state.end.upgraded.load(Ordering::SeqCst) || !self.write_is_blocked() || !self.read_credit_exhausted() {
            return None;
        }
        // The blocked write may have been issued by the read path itself (a pong flushed while reading), in which
        // case nobody else retries it: if the socket is writable again, or reports an error, let the read go on
        // (which retries the write, or returns the error) and wake the task whose write is pending. The credit is
        // not refilled: only bytes the socket accepts earn it.
        //
        // Under TLS a writable socket is not enough: ciphertext rustls still holds is pushed first, and the pause
        // lifts only when none is left (or the socket took part of it, which earns credit).
        if self.io.as_ref().expect("socket is present until drop").socket_writable() && self.drain_tls(cx) {
            self.write_blocked = false;
            #[cfg(feature = "tls")]
            if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
                t.wire.unblock();
            }
            if let Some(w) = self.write_waker.take() {
                w.wake();
            }
            return None;
        }
        self.sync_wire_progress();
        self.read_waker = Some(cx.waker().clone());
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
            t.wire.park_reader(cx.waker());
        }
        let Some(timeout) = self.send_timeout else { return Some(Poll::Pending) };
        let since = *self.write_stalled_since.get_or_insert_with(Instant::now);
        let Some(deadline) = since.checked_add(timeout) else { return Some(Poll::Pending) };
        let sleep = self.read_sleep.get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if sleep.deadline() != deadline {
            sleep.as_mut().reset(deadline);
        }
        match sleep.as_mut().poll(cx) {
            Poll::Ready(()) => Some(Poll::Ready(Err(self.send_timed_out()))),
            Poll::Pending => Some(Poll::Pending),
        }
    }
}

impl AsyncRead for TimedIo {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.send_failed && this.state.end.upgraded.load(Ordering::SeqCst) {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "send timeout")));
        }
        if let Some(r) = this.read_paused(cx) {
            return r;
        }
        let upgraded = this.state.end.upgraded.load(Ordering::SeqCst);
        if !upgraded {
            return this.poll_read_unpaused(cx, buf);
        }
        if this.is_tls() {
            // The credit is spent below TLS, on the ciphertext the gate reads (see `transport::tls_io::Gate`). A read
            // the gate parks looks again at the pause, which starts the deadline of the parked reader.
            for _ in 0..2 {
                let r = this.poll_read_unpaused(cx, buf);
                if r.is_ready() {
                    return r;
                }
                this.read_waker = Some(cx.waker().clone());
                if let Some(paused) = this.read_paused(cx) {
                    return paused;
                }
            }
            return Poll::Pending;
        }
        // While a write is blocked, never read more than the credit left (it is not exhausted here, or the read
        // would have been paused): small slices accepted by the socket must not let whole chunks through.
        let limit = if this.write_blocked { this.read_credit.0.min(buf.remaining()) } else { buf.remaining() };
        let (r, read) = if limit < buf.remaining() {
            let mut limited = ReadBuf::new(buf.initialize_unfilled_to(limit));
            let r = this.poll_read_unpaused(cx, &mut limited);
            let n = limited.filled().len();
            buf.advance(n);
            (r, n)
        } else {
            let before = buf.filled().len();
            let r = this.poll_read_unpaused(cx, buf);
            (r, buf.filled().len() - before)
        };
        if matches!(r, Poll::Ready(Ok(()))) {
            this.read_credit.spend(read);
        }
        if r.is_pending() {
            // So that a send timeout noticed by the writer wakes this reader too.
            this.read_waker = Some(cx.waker().clone());
        }
        r
    }
}

impl TimedIo {
    fn poll_read_unpaused(&mut self, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self;
        if this.framing.is_some() {
            return this.poll_read_checked(cx, buf);
        }
        let before = buf.filled().len();
        match this.sock().poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                if buf.filled().len() > before {
                    this.on_read(&buf.filled()[before..])?;
                }
                Poll::Ready(Ok(()))
            }
            Poll::Pending => this.poll_idle(cx),
            other => other,
        }
    }
}

impl AsyncWrite for TimedIo {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        match this.sock().poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                if n > 0 {
                    this.write_progress(&[io::IoSlice::new(buf)], n);
                }
                Poll::Ready(Ok(n))
            }
            Poll::Pending => {
                this.write_blocked = true;
                this.write_waker = Some(cx.waker().clone());
                this.poll_write_deadline(cx).map(|r| r.map(|()| 0))
            }
            other => other,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match this.sock().poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                if !this.write_blocked {
                    this.write_stalled_since = None;
                }
                // Hyper flushes only with its write buffer empty: the HTTP/1 responses it took are out of memory.
                this.state.budget.flush_deferred();
                Poll::Ready(Ok(()))
            }
            Poll::Pending => {
                this.write_waker = Some(cx.waker().clone());
                this.poll_write_deadline(cx)
            }
            other => other,
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.sock().poll_shutdown(cx)
    }
    fn poll_write_vectored(mut self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[io::IoSlice<'_>]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        match this.sock().poll_write_vectored(cx, bufs) {
            Poll::Ready(Ok(n)) => {
                if n > 0 {
                    this.write_progress(bufs, n);
                }
                Poll::Ready(Ok(n))
            }
            Poll::Pending => {
                this.write_blocked = true;
                this.write_waker = Some(cx.waker().clone());
                this.poll_write_deadline(cx).map(|r| r.map(|()| 0))
            }
            other => other,
        }
    }
    fn is_write_vectored(&self) -> bool {
        self.io.as_ref().is_some_and(|io| io.is_write_vectored())
    }
}

/// Unsent bytes a connection may hold in the kernel before its socket stops being reported writable
/// (`TCP_NOTSENT_LOWAT`), when `send_timeout` is on.
///
/// Linux reports a TCP socket writable only when the free part of the send buffer reaches half of what is queued, and
/// autotuning grows that buffer to several MiB: a client that reads slowly but steadily frees space the server cannot
/// see for many seconds, and the send timeout would cut it. With a low-water mark the socket is writable as soon as
/// the unsent part falls under half of it (`tcp_poll` asks `tcp_stream_memory_free` with a wake-up margin of one
/// shift), whatever the size of the buffer, so progress shows in steps of about half the mark.
/// HTTP/2 uses the same socket: its flow control bounds what h2 writes, but a client with large windows can still
/// fill the kernel buffers, so it gets the same limit. Where the socket option is not available (see
/// [`limit_unsent_bytes`]) the granularity is whatever the platform's buffers give.
#[cfg(any(target_os = "linux", target_os = "android"))]
const NOTSENT_LOWAT: u32 = 64 * 1024;

/// Sets `TCP_NOTSENT_LOWAT` on an accepted socket (Linux and Android). Failures are ignored: the option only makes
/// the send timeout notice progress sooner. Not available through `socket2` on macOS and the BSDs, nor on Windows,
/// where the platform's own buffers decide how soon progress is seen.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) fn limit_unsent_bytes(stream: &TcpStream) {
    let _ = socket2::SockRef::from(stream).set_tcp_notsent_lowat(NOTSENT_LOWAT);
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub(super) fn limit_unsent_bytes(_stream: &TcpStream) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A peer that floods pings and reads slowly: the socket takes `slice` bytes per writable event. Whatever the
    /// slice, the bytes read while writes are blocked stay within what the socket took plus the maximum credit.
    #[test]
    fn the_read_credit_follows_the_bytes_the_socket_accepts() {
        for slice in [1usize, 512, 4 * 1024, 16 * 1024, 32 * 1024, 64 * 1024] {
            let mut credit = ReadCredit::new();
            let (mut written, mut read) = (0usize, 0usize);
            for _ in 0..10_000 {
                // The socket accepts a slice (a writable event), then the reader takes chunks until the credit is
                // spent (paused: a blocked write with no credit left).
                credit.earn(slice);
                written += slice;
                while !credit.exhausted() {
                    // A read never takes more than the credit left.
                    let chunk = credit.0.min(8 * 1024);
                    credit.spend(chunk);
                    read += chunk;
                }
            }
            assert!(read <= written + READ_CREDIT_MAX, "slice {slice}: read {read}, written {written}");
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[tokio::test]
    async fn accepted_sockets_get_a_low_water_mark_for_unsent_data() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        limit_unsent_bytes(&accepted);
        assert_eq!(socket2::SockRef::from(&accepted).tcp_notsent_lowat().unwrap(), NOTSENT_LOWAT);
        drop(client);
    }

    #[test]
    fn the_frame_scanner_tells_the_kinds_of_frame_that_pass() {
        fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
            let mut v = vec![0, (payload.len() >> 8) as u8, payload.len() as u8, kind, 0, 0, 0, 0, 1];
            v.extend_from_slice(payload);
            v
        }
        let mut f = H2Frames::default();
        assert_eq!(f.scan(&frame(0x8, &[0, 0, 0, 1])), 1 << 8, "WINDOW_UPDATE");
        assert_eq!(f.scan(&frame(0x6, &[0; 8])), 1 << 6, "PING");
        assert_eq!(f.scan(&frame(0x4, &[])), 1 << 4, "SETTINGS");
        assert_eq!(f.scan(&frame(0x0, &[1; 300])), H2_DATA, "DATA");
        assert_eq!(f.scan(&frame(0x1, &[0x82])), H2_HEADERS, "HEADERS");
        // A frame cut at every byte boundary: every byte of its payload still belongs to it.
        let mut stream = frame(0x8, &[0, 0, 0, 1]);
        stream.extend(frame(0x1, &[0x82, 0x86]));
        stream.extend(frame(0x4, &[]));
        let mut f = H2Frames::default();
        let kinds: Vec<u16> = stream.chunks(1).map(|c| f.scan(c)).collect();
        assert_eq!(kinds.iter().filter(|k| **k & H2_HEADERS != 0).count(), 3, "the byte that completes the header and the payload");
        assert_eq!(kinds.iter().filter(|k| **k & (H2_DATA | H2_HEADERS | H2_CONTINUATION) != 0).count(), 3);
        assert!(kinds[..13].iter().all(|k| *k & H2_HEADERS == 0), "bytes of the WINDOW_UPDATE are not a response");
    }
}
