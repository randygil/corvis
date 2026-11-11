//! How the built-in server shuts down: what is still running when the application's shutdown hooks start.
//!
//! Every test runs on a current-thread runtime with a paused clock: it advances only when every task waits, which is
//! exactly when a shutdown deadline should fire, so no test sleeps to synchronize. The idle timeouts of the server are
//! off, so the only timers are the ones a test is about.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::ServerOptions;
use corvis::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// What happened, in order, across the handlers and the hooks of one application.
#[injectable]
pub struct Events {
    #[default]
    seen: Mutex<Vec<&'static str>>,
    #[default]
    changed: Notify,
    /// Lets the `slow` handler finish.
    #[default]
    release: Notify,
}

impl Events {
    fn push(&self, event: &'static str) {
        self.seen.lock().unwrap().push(event);
        self.changed.notify_waiters();
    }
    fn list(&self) -> Vec<&'static str> {
        self.seen.lock().unwrap().clone()
    }
    /// Waits until `event` happened.
    async fn wait_for(&self, event: &'static str) {
        loop {
            let changed = self.changed.notified();
            if self.list().contains(&event) {
                return;
            }
            changed.await;
        }
    }
}

#[async_trait]
impl OnApplicationShutdown for Events {
    async fn on_application_shutdown(&self, _signal: Option<String>) -> corvis::Result<()> {
        self.push("hook");
        Ok(())
    }
}

/// Records `dropped` when the handler's future is dropped.
struct DropGuard(Arc<Events>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.0.push("dropped");
    }
}

#[controller("life")]
pub struct LifeController {
    events: Arc<Events>,
}

#[routes]
impl LifeController {
    /// Never returns.
    #[get("hang")]
    async fn hang(&self) -> &'static str {
        self.events.push("started");
        let _guard = DropGuard(self.events.clone());
        std::future::pending::<()>().await;
        "never"
    }

    /// Returns when the test releases it.
    #[get("slow")]
    async fn slow(&self) -> &'static str {
        self.events.push("started");
        self.events.release.notified().await;
        self.events.push("finished");
        "done"
    }

    /// Answers without reading the body (a declared length over the body limit is refused before it is read).
    #[post("upload")]
    async fn upload(&self) -> &'static str {
        "ok"
    }
}

#[module(controllers = [LifeController], providers = [Events])]
pub struct LifeModule;

/// A listening application with the idle timeouts off and `options` applied on top, and the sender of its shutdown
/// signal.
struct Server {
    addr: SocketAddr,
    events: Arc<Events>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    /// The application's `listen`: it ends once the server returned and the shutdown hooks ran.
    serving: JoinHandle<corvis::Result<()>>,
}

impl Server {
    async fn start(options: impl FnOnce(ServerOptions) -> ServerOptions) -> Server {
        let mut app = CorvisFactory::create::<LifeModule>().await.unwrap();
        app.silence_route_logs();
        app.set_server_options(options(ServerOptions::new().headers_timeout(None).keep_alive_timeout(None).request_timeout(None)));
        let events = app.get::<Events>().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let (addr, serving) = app
            .listen_until_in_background("127.0.0.1:0", async move {
                let _ = stopped.await;
            })
            .await
            .unwrap();
        Server { addr, events, stop: Some(stop), serving }
    }

    async fn send(&self, request: &[u8]) -> TcpStream {
        let mut client = TcpStream::connect(self.addr).await.unwrap();
        client.write_all(request).await.unwrap();
        client
    }

    /// Sends the shutdown signal; the time it was sent.
    fn stop(&mut self) -> Instant {
        self.stop.take().expect("one shutdown").send(()).unwrap();
        Instant::now()
    }
}

async fn read_all(mut client: TcpStream) -> String {
    let mut response = Vec::new();
    let _ = client.read_to_end(&mut response).await;
    String::from_utf8_lossy(&response).into_owned()
}

#[tokio::test(start_paused = true)]
async fn a_handler_still_running_at_the_deadline_is_dropped_before_the_hooks_run() {
    let mut server = Server::start(|o| o.shutdown_timeout(Duration::from_secs(1))).await;
    let client = server.send(b"GET /life/hang HTTP/1.1\r\nHost: x\r\n\r\n").await;
    server.events.wait_for("started").await;
    let stopped = server.stop();
    server.serving.await.unwrap().unwrap();
    assert_eq!(Instant::now() - stopped, Duration::from_secs(1));
    assert_eq!(server.events.list(), ["started", "dropped", "hook"]);
    // The connection was cut without a response.
    assert_eq!(read_all(client).await, "");
}

#[tokio::test(start_paused = true)]
async fn a_request_that_finishes_before_the_deadline_gets_its_whole_response_before_the_hooks_run() {
    let mut server = Server::start(|o| o).await;
    let client = server.send(b"GET /life/slow HTTP/1.1\r\nHost: x\r\n\r\n").await;
    server.events.wait_for("started").await;
    let stopped = server.stop();
    // Half the default deadline later (the clock only moves once the server waits for the request), it still waits.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!server.serving.is_finished());
    server.events.release.notify_one();
    let response = read_all(client).await;
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response:?}");
    assert!(response.ends_with("\r\n\r\ndone"), "{response:?}");
    server.serving.await.unwrap().unwrap();
    assert!(Instant::now() - stopped < Duration::from_secs(10));
    assert_eq!(server.events.list(), ["started", "finished", "hook"]);
}

#[tokio::test(start_paused = true)]
async fn without_a_deadline_the_server_waits_for_its_requests() {
    let mut server = Server::start(|o| o.shutdown_timeout(None)).await;
    let client = server.send(b"GET /life/slow HTTP/1.1\r\nHost: x\r\n\r\n").await;
    server.events.wait_for("started").await;
    server.stop();
    tokio::time::sleep(Duration::from_secs(3600)).await;
    assert!(!server.serving.is_finished());
    assert_eq!(server.events.list(), ["started"]);
    server.events.release.notify_one();
    assert!(read_all(client).await.ends_with("done"));
    server.serving.await.unwrap().unwrap();
    assert_eq!(server.events.list(), ["started", "finished", "hook"]);
}

#[tokio::test(start_paused = true)]
async fn a_body_drain_in_progress_is_cancelled_at_the_deadline() {
    let mut server = Server::start(|o| o.shutdown_timeout(Duration::from_secs(1))).await;
    // A declared length over the body limit: answered `413` without reading the body, whose bytes are then discarded
    // (up to the body limit, for up to 30 s without `request_timeout`) while the client sends nothing more.
    let mut client = server.send(b"POST /life/upload HTTP/1.1\r\nHost: x\r\nContent-Length: 100000000\r\n\r\nsome").await;
    let mut head = [0u8; 12];
    client.read_exact(&mut head).await.unwrap();
    assert_eq!(&head, b"HTTP/1.1 413");
    let stopped = server.stop();
    server.serving.await.unwrap().unwrap();
    // The drain would have waited 30 s; the server returned at its deadline, having cancelled it.
    assert_eq!(Instant::now() - stopped, Duration::from_secs(1));
}

/// HTTP/2 handlers run in tasks of their own (one per stream), outside the connection's task: they are cut too.
#[tokio::test(start_paused = true)]
async fn an_http2_handler_still_running_at_the_deadline_is_dropped_before_the_hooks_run() {
    let mut server = Server::start(|o| o.h2c(true).shutdown_timeout(Duration::from_secs(1))).await;
    let tcp = TcpStream::connect(server.addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http2::handshake(hyper_util::rt::TokioExecutor::new(), hyper_util::rt::TokioIo::new(tcp)).await.unwrap();
    tokio::spawn(conn);
    let request = hyper::Request::get("http://localhost/life/hang").body(http_body_util::Empty::<bytes::Bytes>::new()).unwrap();
    let response = tokio::spawn(sender.send_request(request));
    server.events.wait_for("started").await;
    let stopped = server.stop();
    server.serving.await.unwrap().unwrap();
    assert_eq!(Instant::now() - stopped, Duration::from_secs(1));
    assert_eq!(server.events.list(), ["started", "dropped", "hook"]);
    assert!(response.await.unwrap().is_err(), "no response");
}
