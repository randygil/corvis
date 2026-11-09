//! The socket under `TimedIo`: plain TCP, or TLS over TCP (`tls` feature).
//!
//! Layering of a TLS connection: TCP, `Gate`, rustls, `TimedIo`, hyper. `TimedIo` sees plaintext (the framing check,
//! the h2 frame scanner and the preface check need it), while everything that has to measure the wire sits in
//! `Gate`, below rustls: the bytes TCP accepted (the progress the send timeout and the read credit are about), whether
//! the last TCP write was pending, and the read pause of upgraded connections.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// The socket of a connection.
pub(super) enum ServerIo {
    Plain(TcpStream),
    #[cfg(feature = "tls")]
    Tls(Box<tls_io::TlsIo>),
}

impl ServerIo {
    /// Whether the TCP socket reports room for a write right now. `try_write` with no bytes reports the readiness the
    /// driver already knows, registers no waker and sends nothing.
    #[allow(clippy::infallible_destructuring_match)]
    pub(super) fn socket_writable(&self) -> bool {
        let tcp = match self {
            ServerIo::Plain(s) => s,
            #[cfg(feature = "tls")]
            ServerIo::Tls(t) => t.stream.get_ref().0.tcp(),
        };
        !matches!(tcp.try_write(&[]), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
    }
}

impl AsyncRead for ServerIo {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ServerIo::Plain(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "tls")]
            ServerIo::Tls(t) => Pin::new(&mut t.stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ServerIo {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            ServerIo::Plain(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "tls")]
            ServerIo::Tls(t) => Pin::new(&mut t.stream).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ServerIo::Plain(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "tls")]
            ServerIo::Tls(t) => Pin::new(&mut t.stream).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ServerIo::Plain(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "tls")]
            ServerIo::Tls(t) => Pin::new(&mut t.stream).poll_shutdown(cx),
        }
    }
    fn poll_write_vectored(self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[io::IoSlice<'_>]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            ServerIo::Plain(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(feature = "tls")]
            ServerIo::Tls(t) => Pin::new(&mut t.stream).poll_write_vectored(cx, bufs),
        }
    }
    fn is_write_vectored(&self) -> bool {
        match self {
            ServerIo::Plain(s) => s.is_write_vectored(),
            #[cfg(feature = "tls")]
            ServerIo::Tls(t) => t.stream.is_write_vectored(),
        }
    }
}

#[cfg(feature = "tls")]
pub(super) mod tls_io {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::task::Waker;

    use tokio_rustls::server::TlsStream;

    use super::*;
    use crate::server::{ConnState, ReadCredit};

    /// A TLS connection: the rustls stream over the gated socket, and the gate's shared state.
    pub(in crate::server) struct TlsIo {
        pub(in crate::server) stream: TlsStream<Gate>,
        pub(in crate::server) wire: Arc<Wire>,
    }

    impl TlsIo {
        /// Ciphertext rustls still holds (replies it generated while reading, or data the socket did not take yet).
        pub(in crate::server) fn wants_write(&self) -> bool {
            self.stream.get_ref().1.wants_write()
        }
        pub(in crate::server) fn alpn(&self) -> Option<&[u8]> {
            self.stream.get_ref().1.alpn_protocol()
        }
    }

    #[derive(Default)]
    pub(in crate::server) struct WireState {
        /// Bytes TCP has accepted in total.
        accepted: u64,
        /// The last write to the socket was pending.
        blocked: bool,
        /// Ciphertext an upgraded connection may read while a write is blocked, earned by bytes TCP accepts.
        credit: ReadCredit,
        /// The reader parked by the gate (credit spent while blocked).
        read_waker: Option<Waker>,
    }

    /// What `TimedIo` and the gate share about the socket under TLS.
    pub(in crate::server) struct Wire {
        state: Mutex<WireState>,
    }

    impl Wire {
        pub(in crate::server) fn new() -> Arc<Self> {
            Arc::new(Wire { state: Mutex::new(WireState::default()) })
        }
        fn lock(&self) -> std::sync::MutexGuard<'_, WireState> {
            self.state.lock().unwrap_or_else(|e| e.into_inner())
        }
        /// Total bytes TCP accepted: it changes whenever the client reads.
        pub(in crate::server) fn accepted(&self) -> u64 {
            self.lock().accepted
        }
        pub(in crate::server) fn blocked(&self) -> bool {
            self.lock().blocked
        }
        pub(in crate::server) fn credit_exhausted(&self) -> bool {
            self.lock().credit.exhausted()
        }
        pub(in crate::server) fn unblock(&self) {
            self.lock().blocked = false;
        }
        /// Parks the reader of the gate: woken by the next byte TCP accepts.
        pub(in crate::server) fn park_reader(&self, waker: &Waker) {
            self.lock().read_waker = Some(waker.clone());
        }
        fn progress(&self, n: usize) {
            let waker = {
                let mut s = self.lock();
                s.accepted = s.accepted.wrapping_add(n as u64);
                s.blocked = false;
                s.credit.earn(n);
                s.read_waker.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
        }
        fn pending(&self) {
            self.lock().blocked = true;
        }
    }

    /// The TCP socket under rustls. It forwards everything, records what TCP accepts, and, on an upgraded connection
    /// whose write is blocked, stops reading ciphertext once the read credit is spent. That is the pause of
    /// `TimedIo::read_paused` applied below TLS: what rustls answers on its own (a `KeyUpdate` reply, an alert) comes
    /// from ciphertext it reads, so reads that stop here keep those replies from piling up behind a peer that does
    /// not read.
    pub(in crate::server) struct Gate {
        tcp: TcpStream,
        wire: Arc<Wire>,
        state: Arc<ConnState>,
    }

    impl Gate {
        pub(in crate::server) fn new(tcp: TcpStream, wire: Arc<Wire>, state: Arc<ConnState>) -> Self {
            Gate { tcp, wire, state }
        }
        pub(in crate::server) fn tcp(&self) -> &TcpStream {
            &self.tcp
        }
    }

    impl AsyncRead for Gate {
        fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
            let this = self.get_mut();
            if buf.remaining() == 0 || !this.state.upgraded.load(Ordering::SeqCst) {
                return Pin::new(&mut this.tcp).poll_read(cx, buf);
            }
            let limit = {
                let mut s = this.wire.lock();
                if !s.blocked {
                    return Pin::new(&mut this.tcp).poll_read(cx, buf);
                }
                if s.credit.exhausted() {
                    s.read_waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
                s.credit.available().min(buf.remaining())
            };
            let mut limited = ReadBuf::new(buf.initialize_unfilled_to(limit));
            let r = Pin::new(&mut this.tcp).poll_read(cx, &mut limited);
            let n = limited.filled().len();
            buf.advance(n);
            if n > 0 {
                this.wire.lock().credit.spend(n);
            }
            r
        }
    }

    impl AsyncWrite for Gate {
        fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
            let this = self.get_mut();
            let r = Pin::new(&mut this.tcp).poll_write(cx, buf);
            this.note(&r);
            r
        }
        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.get_mut().tcp).poll_flush(cx)
        }
        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.get_mut().tcp).poll_shutdown(cx)
        }
        fn poll_write_vectored(self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[io::IoSlice<'_>]) -> Poll<io::Result<usize>> {
            let this = self.get_mut();
            let r = Pin::new(&mut this.tcp).poll_write_vectored(cx, bufs);
            this.note(&r);
            r
        }
        fn is_write_vectored(&self) -> bool {
            self.tcp.is_write_vectored()
        }
    }

    impl Gate {
        fn note(&self, r: &Poll<io::Result<usize>>) {
            match r {
                Poll::Ready(Ok(n)) if *n > 0 => self.wire.progress(*n),
                Poll::Pending => self.wire.pending(),
                _ => {}
            }
        }
    }
}
