//! HTTP server (hyper) integration.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Waker;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use tokio::net::TcpListener;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::router::AppShared;

mod accept;
mod budget;
mod guarded_body;
mod h1_framing;
mod options;
mod rapid_reset;
mod request;
mod response;
mod timed_io;
#[cfg(feature = "tls")]
pub mod tls;
mod transport;
#[cfg(all(test, feature = "tls"))]
mod transport_tests;
mod watchdog;

pub use options::ServerOptions;
#[cfg(feature = "socket-io")]
pub(crate) use request::drain;
pub(crate) use request::{
    PendingBody, PendingBodySlot, content_length, entity_too_large_error, request_aborted_error, request_timeout_error, respond,
};
pub(crate) use response::{empty_body, into_hyper};

use budget::{Budget, Verdict};
use guarded_body::{GuardedBody, H2_SEND_BUF, InflightGuard};
use rapid_reset::{CancelWatch, CancelWindow, Outcome};
use response::PlainLen;
use timed_io::{Proto, TimedIo, limit_unsent_bytes};
use transport::ServerIo;
use watchdog::{BodyClock, Epoch, finish_connection};

pub(crate) type ResBody = UnsyncBoxBody<Bytes, std::io::Error>;

async fn handle(
    shared: Arc<AppShared>,
    req: hyper::Request<Incoming>,
    remote: SocketAddr,
    options: &ServerOptions,
    tasks: &ServerTasks,
    client_aborted: &mut bool,
) -> Result<hyper::Response<ResBody>, crate::http_adapter::SocketDestroyed> {
    // An attached Engine.IO server takes every request under its path first, upgrades included.
    #[cfg(feature = "socket-io")]
    {
        if let Some(engine) = shared.http_server.engine_for(req.uri().path()) {
            return Ok(engine.handle(req, remote, &shared.http_settings.body_parser, options.request_timeout).await);
        }
    }
    #[cfg(feature = "websockets")]
    {
        if let Some(gw) = crate::websockets::match_upgrade(&shared, req.headers(), req.uri().path()) {
            return Ok(crate::websockets::upgrade(gw, req, remote, &shared.http_settings.body_parser));
        }
    }
    // `server.on('upgrade', ...)` listeners take every upgrade request (Node semantics).
    if crate::http_adapter::is_upgrade_request(&req) {
        if shared.http_server.upgrade_listener_count() > 0 {
            return crate::http_adapter::dispatch_upgrade(&shared.http_server, req, remote, &shared.http_settings.body_parser).await;
        }
        // Like the `ws` adapter, upgrades for paths without a gateway are destroyed.
        #[cfg(feature = "websockets")]
        {
            if !shared.gateways.is_empty() {
                return Err(crate::http_adapter::SocketDestroyed);
            }
        }
    }
    Ok(respond(&shared, req, Some(remote), options, tasks, client_aborted).await)
}

// ---------------------------------------------------------------------------
// Connection timeouts
// ---------------------------------------------------------------------------

/// Activity of one connection, shared between its IO and its in-flight requests.
#[derive(Default)]
struct ConnState {
    /// Requests whose response has not been fully written yet.
    inflight: AtomicUsize,
    /// Bumped when a request starts or ends.
    generation: AtomicU64,
    /// Requests whose response is done (fully written or dropped). The framing check compares it
    /// with the requests it forwarded to know when a connection has nothing in flight.
    finished: AtomicU64,
    /// The connection was upgraded (WebSocket, ...): no idle timeouts.
    upgraded: AtomicBool,
    /// An upgrade listener destroyed the socket (or the upgrade was refused): nothing to protect.
    destroyed: AtomicBool,
    /// The server closes the connection to punish the peer (idle timeouts, refused h2c): no drain.
    no_linger: AtomicBool,
    /// The socket and its permit, left here by `TimedIo::drop` for the connection task, which decides
    /// whether to drain before closing (hyper's own timeouts are only visible to that task).
    closing: Mutex<Option<(ServerIo, Option<OwnedSemaphorePermit>)>>,
    /// Task waiting on the IO, woken when the last in-flight request ends.
    waker: Mutex<Option<Waker>>,
    /// Requests cancelled and completed in the current and the previous window (rapid reset detection).
    window: Mutex<Option<CancelWindow>>,
    /// Signalled when too many requests were cancelled before their response (rapid reset): the
    /// connection task closes the connection.
    reset_flood: tokio::sync::Notify,
    /// Set when the flood was detected: no further request of the connection reaches a handler.
    flooded: AtomicBool,
    /// `http2_max_cancelled_streams`: cancellations over the window that close the connection whatever it completed.
    cancel_ceiling: u32,
    /// Origin of the times the response-body clocks record.
    epoch: Epoch,
    /// Clocks of the response bodies of the connection that are alive (send timeout watchdog).
    clocks: Mutex<Vec<Arc<BodyClock>>>,
    /// When hyper last took a frame from any response body of the connection (milliseconds since `epoch` plus
    /// one; `0` = never): the connection's progress.
    progress: AtomicU64,
    /// Signalled when a clock is registered, so a watchdog that had nothing to watch looks again.
    clocks_changed: tokio::sync::Notify,
    /// Response bytes the connection holds (`max_pending_response_bytes`).
    budget: Mutex<Budget>,
    /// Signalled whenever pending bytes or running handlers drop, so waiting requests look again.
    budget_freed: tokio::sync::Notify,
    /// HTTP/1: charges of bodies hyper already took into its write buffer, released when hyper flushes the socket
    /// (it only does so with its write buffer empty).
    deferred: AtomicUsize,
}

/// hyper's `ignore_invalid_headers`; it must stay off while the framing check is on.
const IGNORE_INVALID_HEADERS: bool = false;

/// Whether hyper can add `t` to an `Instant` (it panics otherwise): a longer timeout is the same as none.
fn representable(t: Duration) -> bool {
    std::time::Instant::now().checked_add(t).is_some()
}

/// The connection builder of every listener: HTTP/1 and HTTP/2 settings from the options. Any listener that
/// can speak HTTP/2 (h2c, and TLS with ALPN `h2`) must be built with it so the limits apply to all of them.
pub(crate) fn connection_builder(options: &ServerOptions, tasks: ServerTasks) -> auto::Builder<ServerTasks> {
    // HTTP/2 streams run in tasks of their own: spawned through the server's tracker, so the shutdown covers them.
    let mut builder = auto::Builder::new(tasks);
    // The framing check reads header lines the way httparse does by default: `ignore_invalid_headers` would let
    // httparse skip lines the check has read (design.md, decision 4, of `e0-reject-ambiguous-http1-framing`).
    debug_assert!(options.insecure_http_parser || !IGNORE_INVALID_HEADERS);
    builder
        .http1()
        .ignore_invalid_headers(IGNORE_INVALID_HEADERS)
        .timer(TokioTimer::new())
        .header_read_timeout(options.headers_timeout.filter(|t| representable(*t)))
        .max_buf_size(options.max_header_size.max(8 * 1024));
    builder
        .http2()
        .timer(TokioTimer::new())
        .max_header_list_size(u32::try_from(options.max_header_size.max(8 * 1024)).unwrap_or(u32::MAX))
        .max_concurrent_streams(options.http2_max_concurrent_streams.max(1))
        .max_send_buf_size(H2_SEND_BUF)
        .keep_alive_interval(options.http2_keep_alive_interval.filter(|t| representable(*t)))
        .keep_alive_timeout(if representable(options.http2_keep_alive_timeout) {
            options.http2_keep_alive_timeout
        } else {
            Duration::from_secs(100 * 365 * 24 * 3600)
        });
    builder
}

/// Why a request ends without a response: the socket is destroyed (upgrade listeners, flood, a wait for room that
/// took too long over HTTP/1), or the server resets the HTTP/2 stream (response above `max_pending_response_bytes`,
/// wait for room that took too long) and hyper uses the reason in the source.
#[derive(Debug)]
enum ServiceError {
    Destroyed,
    Reset(h2::Error),
}

impl From<crate::http_adapter::SocketDestroyed> for ServiceError {
    fn from(_: crate::http_adapter::SocketDestroyed) -> Self {
        ServiceError::Destroyed
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Destroyed => f.write_str("socket destroyed"),
            ServiceError::Reset(_) => f.write_str("stream reset: the connection holds too many pending response bytes"),
        }
    }
}

impl std::error::Error for ServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServiceError::Destroyed => None,
            ServiceError::Reset(e) => Some(e),
        }
    }
}

/// Where a listener's connections come from: plain TCP, or TCP with a TLS handshake first.
pub(crate) enum Transport {
    Plain,
    #[cfg(feature = "tls")]
    Tls(tls::TlsSettings),
}

impl Transport {
    pub(crate) fn is_secure(&self) -> bool {
        match self {
            Transport::Plain => false,
            #[cfg(feature = "tls")]
            Transport::Tls(_) => true,
        }
    }
}

/// What a connection task needs, gathered by the accept loop.
struct ConnCtx {
    shared: Arc<AppShared>,
    opts: Arc<ServerOptions>,
    builder: auto::Builder<ServerTasks>,
    tasks: ServerTasks,
    stream_cap: usize,
    remote: SocketAddr,
    state: Arc<ConnState>,
    stop: tokio::sync::watch::Receiver<()>,
}

/// Serves one connection to its end (see [`finish_connection`]).
async fn run_connection(ctx: ConnCtx, stream: ServerIo, permit: Option<OwnedSemaphorePermit>, proto: Proto) {
    let ConnCtx { shared, opts, builder, tasks, stream_cap, remote, state, stop } = ctx;
    let send_timeout = opts.send_timeout;
    let conn_state = state.clone();
    let io = TimedIo::new(stream, permit, state.clone(), &opts, proto);
    let handlers = Arc::new(tokio::sync::Semaphore::new(stream_cap));
    // Compressions in progress of this connection (one set for all its HTTP/2 streams).
    #[cfg(feature = "compression")]
    let slots = crate::http::compression::ConnectionSlots::default();
    let service = hyper::service::service_fn(move |req: hyper::Request<Incoming>| {
        #[cfg(feature = "compression")]
        let req = {
            let mut req = req;
            req.extensions_mut().insert(slots.clone());
            req
        };
        let http2 = req.version() == http::Version::HTTP_2;
        let guard = InflightGuard::new(state.clone());
        let watch = CancelWatch { state: state.clone(), outcome: Outcome::Pending };
        let handlers = handlers.clone();
        let shared = shared.clone();
        let opts = opts.clone();
        let tasks = tasks.clone();
        async move {
            // Streams reset and replaced faster than their handlers stop would otherwise run more
            // handlers than the stream cap; the permit lives until this future is dropped.
            let mut watch = watch;
            // After a rapid-reset verdict the connection only waits for its GOAWAY: new streams are
            // reset without reaching a handler (hyper resets the stream on a service error).
            if guard.0.flooded.load(Ordering::SeqCst) {
                watch.outcome = Outcome::Refused;
                return Err(ServiceError::Destroyed);
            }
            let _permit = handlers.acquire_owned().await;
            // Room in the connection's response budget before the handler runs (a client that resets
            // its stream meanwhile drops this future: a cancelled request that never reached a handler).
            let admission = match opts.max_pending_response_bytes {
                Some(cap) => {
                    let limit = if http2 { stream_cap } else { 1 };
                    let admit = guard.0.admit(cap / limit, cap);
                    // Over HTTP/1 the wait only delays the client's own pipelined requests and ends when it
                    // reads (or `send_timeout` closes the connection): closing it would cut a slow download.
                    let admitted = match opts.max_pending_response_wait.filter(|_| http2) {
                        Some(t) => tokio::time::timeout(t, admit).await.ok(),
                        None => Some(admit.await),
                    };
                    let Some(admission) = admitted else {
                        // The handler never ran: neither cancelled nor completed.
                        watch.outcome = Outcome::Refused;
                        guard.0.warn_wait_once(opts.max_pending_response_wait.unwrap_or_default());
                        return Err(ServiceError::Reset(h2::Error::from(h2::Reason::REFUSED_STREAM)));
                    };
                    Some((admission, cap))
                }
                None => None,
            };
            if guard.0.flooded.load(Ordering::SeqCst) {
                watch.outcome = Outcome::Refused;
                return Err(ServiceError::Destroyed);
            }
            let mut aborted = false;
            let resp = handle(shared, req, remote, &opts, &tasks, &mut aborted).await;
            watch.outcome = if aborted { Outcome::ClientAborted } else { Outcome::Completed };
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    guard.0.destroyed.store(true, Ordering::SeqCst);
                    return Err(e.into());
                }
            };
            if resp.status() == http::StatusCode::SWITCHING_PROTOCOLS {
                guard.0.upgraded.store(true, Ordering::SeqCst);
            }
            // What the response holds until its body ends: an in-memory body in full (also when
            // compression hides its size), a streamed one only the unsent part of a cut frame.
            let charge = match admission {
                Some((admission, cap)) => {
                    let size = hyper::body::Body::size_hint(resp.body())
                        .exact()
                        .or_else(|| resp.extensions().get::<PlainLen>().map(|p| p.0))
                        .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
                    if admission.finish(size, cap, http2) == Verdict::Reset {
                        // Dropping the response frees its allocation and its compression slot now.
                        drop(resp);
                        watch.outcome = Outcome::Dropped;
                        return Err(ServiceError::Reset(h2::Error::from(h2::Reason::ENHANCE_YOUR_CALM)));
                    }
                    Some(size)
                }
                None => None,
            };
            // From here the body decides how the request ends (completed or cancelled).
            let record = !aborted;
            if record {
                watch.outcome = Outcome::Body;
            }
            Ok::<_, ServiceError>(resp.map(|inner| GuardedBody::with_charge(inner, guard, http2, charge, record).boxed_unsync()))
        }
    });
    if proto == Proto::H2Only {
        // ALPN said `h2`: the HTTP/2-only path. `serve_connection_with_upgrades` sniffs the first bytes whatever the
        // builder's version says, which would let a client speak HTTP/1 here without the framing check.
        let conn = builder.serve_connection(TokioIo::new(io), service).into_owned();
        finish_connection(conn_state, conn, stop, |c| c.graceful_shutdown(), send_timeout).await;
        return;
    }
    let conn = builder.serve_connection_with_upgrades(TokioIo::new(io), service);
    let conn = conn.into_owned();
    finish_connection(conn_state, conn, stop, |c| c.graceful_shutdown(), send_timeout).await;
}

/// The TLS handshake of one connection, in the connection's own task: bounded by the handshake timeout, abandoned
/// when the server stops, and never answered with HTTP data when it fails. The permit stays with the task, so a
/// connection in its handshake counts against `max_connections`.
#[cfg(feature = "tls")]
async fn handshake(
    settings: tls::TlsSettings,
    tcp: tokio::net::TcpStream,
    state: Arc<ConnState>,
    stop: &mut tokio::sync::watch::Receiver<()>,
) -> Option<(ServerIo, Proto)> {
    use transport::tls_io::{Gate, TlsIo, Wire};
    let wire = Wire::new();
    let gate = Gate::new(tcp, wire.clone(), state);
    let accept = settings.acceptor.accept(gate);
    let stream = tokio::select! {
        r = tokio::time::timeout(settings.handshake_timeout, accept) => r.ok()?.ok()?,
        _ = stop.changed() => return None,
    };
    let tls = TlsIo { stream, wire };
    let proto = match tls.alpn() {
        Some(b"h2") => Proto::H2Only,
        Some(b"http/1.1") | None => Proto::H1Only,
        Some(_) => return None,
    };
    Some((ServerIo::Tls(Box::new(tls)), proto))
}

/// The tasks a server starts: connections (with their TLS handshake), the reads that discard unread request bodies,
/// and the HTTP/2 streams hyper spawns through it (it is the connection builder's executor). Tracked so the shutdown
/// waits for them, and cancelled together at its deadline: a cancelled task is dropped at its next poll.
#[derive(Clone, Default)]
pub(crate) struct ServerTasks {
    tracker: TaskTracker,
    cancel: CancellationToken,
    /// Connection tasks running, for the line that says how many the shutdown cut.
    connections: Arc<AtomicUsize>,
}

impl ServerTasks {
    pub(crate) fn spawn<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tracker.spawn(self.cancel.clone().run_until_cancelled_owned(task));
    }

    fn spawn_connection<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let live = LiveConnection::new(&self.connections);
        self.spawn(async move {
            let _live = live;
            task.await;
        });
    }

    /// Waits for every task, up to `timeout`; past it cancels them and waits until each one has stopped. Returns how
    /// many connections were cut.
    async fn shut_down(&self, timeout: Option<Duration>) -> usize {
        self.tracker.close();
        match timeout.and_then(|t| Instant::now().checked_add(t)) {
            Some(deadline) => {
                if tokio::time::timeout_at(deadline, self.tracker.wait()).await.is_ok() {
                    return 0;
                }
            }
            None => {
                self.tracker.wait().await;
                return 0;
            }
        }
        let cut = self.connections.load(Ordering::SeqCst);
        self.cancel.cancel();
        self.tracker.wait().await;
        cut
    }
}

impl<F> hyper::rt::Executor<F> for ServerTasks
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, fut: F) {
        self.spawn(fut);
    }
}

/// Counts a connection task while it runs.
struct LiveConnection(Arc<AtomicUsize>);

impl LiveConnection {
    fn new(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        LiveConnection(count.clone())
    }
}

impl Drop for LiveConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Accepts connections until `shutdown` resolves, then shuts down: idle connections are closed, the requests in flight
/// get [`ServerOptions::shutdown_timeout`] to finish, and whatever the server started is cancelled past it. When this
/// returns, no task of the server runs.
pub(crate) async fn serve(
    listener: TcpListener,
    shared: Arc<AppShared>,
    options: ServerOptions,
    transport: Transport,
    shutdown: impl std::future::Future<Output = ()> + Send,
) -> std::io::Result<()> {
    let options = Arc::new(options);
    let (stop_tx, _) = tokio::sync::watch::channel(());
    let tasks = ServerTasks::default();
    let builder = connection_builder(&options, tasks.clone());
    let stream_cap = options.http2_max_concurrent_streams.max(1) as usize;
    let permits = options.max_connections.map(|n| Arc::new(tokio::sync::Semaphore::new(n.max(1))));
    tokio::pin!(shutdown);
    let mut accept_errors = accept::AcceptErrors::new(|line| crate::logger::Logger::new("CorvisServer").warn(line));
    while let Some((stream, remote, permit)) = accept_errors.accept(&listener, permits.as_ref(), shutdown.as_mut()).await {
        let _ = stream.set_nodelay(true);
        if options.send_timeout.is_some() {
            limit_unsent_bytes(&stream);
        }
        let state = Arc::new(ConnState { cancel_ceiling: options.http2_max_cancelled_streams, ..ConnState::default() });
        let ctx = ConnCtx {
            shared: shared.clone(),
            opts: options.clone(),
            builder: builder.clone(),
            tasks: tasks.clone(),
            stream_cap,
            remote,
            state,
            stop: stop_tx.subscribe(),
        };
        match &transport {
            Transport::Plain => tasks.spawn_connection(run_connection(ctx, ServerIo::Plain(stream), permit, Proto::Auto)),
            #[cfg(feature = "tls")]
            Transport::Tls(settings) => {
                let settings = settings.clone();
                tasks.spawn_connection(async move {
                    let mut ctx = ctx;
                    let Some((io, proto)) = handshake(settings, stream, ctx.state.clone(), &mut ctx.stop).await else { return };
                    ctx.builder = match proto {
                        Proto::H2Only => ctx.builder.http2_only(),
                        _ => ctx.builder.http1_only(),
                    };
                    run_connection(ctx, io, permit, proto).await;
                });
            }
        }
    }
    drop(listener);
    // Pending long-polls would hold the shutdown: answer them now.
    #[cfg(feature = "socket-io")]
    shared.http_server.close_engines();
    // Idle connections close, the others finish their requests; handshakes are abandoned.
    let _ = stop_tx.send(());
    let cut = tasks.shut_down(options.shutdown_timeout).await;
    if cut > 0 {
        crate::logger::Logger::new("CorvisServer").warn(format!(
            "{cut} connection(s) still had requests in flight when ServerOptions::shutdown_timeout ran out: they were \
             closed and their handlers cancelled"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sets its flag when dropped.
    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn never_ends(flag: &Arc<AtomicBool>) -> impl Future<Output = ()> + Send + 'static {
        let flag = DropFlag(flag.clone());
        async move {
            let _flag = flag;
            std::future::pending::<()>().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_shutdown_cancels_every_kind_of_server_task_at_the_deadline_and_waits_for_them() {
        let tasks = ServerTasks::default();
        let flags: [Arc<AtomicBool>; 3] = Default::default();
        tasks.spawn_connection(never_ends(&flags[0]));
        // A body drain, and an HTTP/2 stream that hyper spawns through the connection builder's executor.
        tasks.spawn(never_ends(&flags[1]));
        hyper::rt::Executor::execute(&tasks, never_ends(&flags[2]));
        // A connection that ends before the deadline is not counted as cut.
        tasks.spawn_connection(tokio::time::sleep(Duration::from_millis(500)));
        let start = Instant::now();
        assert_eq!(tasks.shut_down(Some(Duration::from_secs(2))).await, 1);
        assert_eq!(start.elapsed(), Duration::from_secs(2));
        assert!(flags.iter().all(|f| f.load(Ordering::SeqCst)), "every task was dropped before the shutdown returned");
    }

    #[tokio::test(start_paused = true)]
    async fn the_shutdown_returns_when_the_tasks_end_and_without_a_usable_deadline_waits_for_them() {
        for (timeout, task, returns_after) in [
            (Some(Duration::from_secs(10)), Duration::from_secs(1), Duration::from_secs(1)),
            (None, Duration::from_secs(3600), Duration::from_secs(3600)),
            // Too long to add to an `Instant`: no deadline.
            (Some(Duration::MAX), Duration::from_secs(100), Duration::from_secs(100)),
        ] {
            let tasks = ServerTasks::default();
            tasks.spawn_connection(tokio::time::sleep(task));
            let start = Instant::now();
            assert_eq!(tasks.shut_down(timeout).await, 0);
            assert_eq!(start.elapsed(), returns_after, "{timeout:?}");
        }
    }

    /// A client that opens a TLS connection and never sends its ClientHello holds a connection task (the handshake has
    /// its own 10 s timeout): the shutdown does not wait for it. Driven through `serve` itself, since the public test
    /// entry point (`listen_until_in_background`) listens without TLS.
    #[cfg(feature = "tls")]
    #[tokio::test(start_paused = true)]
    async fn a_tls_client_that_sends_nothing_does_not_hold_the_shutdown() {
        use tokio::io::AsyncReadExt;
        use tokio::net::TcpStream;

        use crate::prelude::*;
        #[module()]
        struct M;
        let mut app = CorvisFactory::create::<M>().await.unwrap();
        app.init().await.unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap().self_signed(&key).unwrap();
        let tls = crate::tls::TlsOptions::from_pem(cert.pem().as_bytes(), key.serialize_pem().as_bytes()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let options = ServerOptions::new().headers_timeout(None).keep_alive_timeout(None).shutdown_timeout(Duration::from_secs(1));
        let server = tokio::spawn(serve(listener, app.shared().unwrap(), options, Transport::Tls(tls.settings()), async move {
            let _ = stopped.await;
        }));
        let mut client = TcpStream::connect(addr).await.unwrap();
        // Half the handshake timeout: the clock moves once the server waits for the ClientHello.
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!server.is_finished());
        stop.send(()).unwrap();
        let stopped_at = Instant::now();
        server.await.unwrap().unwrap();
        assert!(Instant::now() - stopped_at <= Duration::from_secs(1));
        // The socket was closed without a byte.
        let mut rest = Vec::new();
        let _ = client.read_to_end(&mut rest).await;
        assert!(rest.is_empty());
    }
}
