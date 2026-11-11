//! How the built-in server shuts down: what is still running when the application's shutdown hooks start.
//!
//! Every test runs on a current-thread runtime with a paused clock: it advances only when every task waits, which is
//! exactly when a shutdown deadline should fire, so no test sleeps to synchronize. The idle timeouts of the server are
//! off, so the only timers are the ones a test is about.

use std::sync::{Arc, Mutex};

use corvis::ServerOptions;
use corvis::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;

/// What happened, in order, across the handlers and the hooks of one application.
#[injectable]
pub struct Events {
    #[default]
    seen: Mutex<Vec<&'static str>>,
    #[default]
    changed: Notify,
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
}

#[module(controllers = [LifeController], providers = [Events])]
pub struct LifeModule;

/// An application with the idle timeouts off and `options` applied on top.
async fn app(options: impl FnOnce(ServerOptions) -> ServerOptions) -> (CorvisApplication, Arc<Events>) {
    let mut app = CorvisFactory::create::<LifeModule>().await.unwrap();
    app.silence_route_logs();
    app.set_server_options(options(ServerOptions::new().headers_timeout(None).keep_alive_timeout(None).request_timeout(None)));
    let events = app.get::<Events>().unwrap();
    (app, events)
}

async fn send(addr: std::net::SocketAddr, head: &str) -> TcpStream {
    let mut client = TcpStream::connect(addr).await.unwrap();
    client.write_all(head.as_bytes()).await.unwrap();
    client
}

#[tokio::test(start_paused = true)]
async fn a_handler_still_running_at_the_deadline_is_dropped_before_the_hooks_run() {
    let (app, events) = app(|o| o).await;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (addr, serving) = app
        .listen_until_in_background("127.0.0.1:0", async move {
            let _ = stopped.await;
        })
        .await
        .unwrap();
    let mut client = send(addr, "GET /life/hang HTTP/1.1\r\nHost: x\r\n\r\n").await;
    events.wait_for("started").await;
    stop.send(()).unwrap();
    serving.await.unwrap().unwrap();
    assert_eq!(events.list(), ["started", "dropped", "hook"]);
    // The connection was cut without a response.
    let mut rest = Vec::new();
    let _ = client.read_to_end(&mut rest).await;
    assert!(rest.is_empty(), "{:?}", String::from_utf8_lossy(&rest));
}
