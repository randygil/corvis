#![cfg(feature = "socket-io")]
//! Regression tests of the second security review of the gateway adapter: events that race the opening of the gate
//! start in arrival order by construction (B1); the `handle_disconnect` queue is fair between client addresses, refuses
//! new sessions to an address at its share until its hooks drain, and runs every hook exactly once (B2); an Engine.IO
//! path that would hide an HTTP route fails at startup. Nothing here waits a fixed time or compares timings: the tests
//! wait for conditions (with a deadline that only turns a hang into a failure). Servers bind to 127.0.0.1.

mod engine_support;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::Module;
use corvis::prelude::*;
use corvis::socket_io::{IoAdapter, IoSocket};
use corvis::{OnGatewayConnection, OnGatewayDisconnect, websocket_gateway};
use engine_support::io_client::Raw;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

/// What the gateways of these tests record, and a way to wait for it.
#[injectable]
pub struct Record {
    #[default]
    entries: Arc<Mutex<Vec<String>>>,
    /// socket id -> the numbers its `seq` events carried, in the order their handlers ran.
    #[default]
    seqs: Arc<Mutex<HashMap<String, Vec<i64>>>>,
    #[default]
    changed: Arc<Notify>,
    /// Disconnect hooks wait here until the test adds permits.
    #[default]
    gate: Arc<HookGate>,
}

pub struct HookGate(tokio::sync::Semaphore);

impl Default for HookGate {
    fn default() -> Self {
        HookGate(tokio::sync::Semaphore::new(0))
    }
}

impl Record {
    fn push(&self, entry: impl Into<String>) {
        self.entries.lock().unwrap().push(entry.into());
        self.changed.notify_waiters();
    }

    fn count(&self, prefix: &str) -> usize {
        self.entries.lock().unwrap().iter().filter(|e| e.starts_with(prefix)).count()
    }

    fn seq(&self, socket: &IoSocket, n: i64) {
        self.seqs.lock().unwrap().entry(socket.id().to_string()).or_default().push(n);
        self.changed.notify_waiters();
    }

    /// Waits until `done` holds; fails the test after 60 s (a hang, not a measurement).
    async fn wait_until(&self, what: &str, done: impl Fn(&Record) -> bool) {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if done(self) {
                    return;
                }
                notified.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(60), wait).await.is_err() {
            panic!("{what} did not happen within 60 s");
        }
    }
}

async fn start<M: Module>(adapter: IoAdapter) -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.use_websocket_adapter(adapter);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

fn local(last: u8) -> IpAddr {
    IpAddr::from([127, 0, 0, last])
}

// ------------------------------------------------------------------------------- B1: events racing the opening of the gate

#[websocket_gateway("/o")]
pub struct NoHookGateway {
    record: Arc<Record>,
}

#[handlers]
impl NoHookGateway {
    #[subscribe_message("seq")]
    fn seq(&self, #[message_body] n: i64, #[connected_socket] socket: IoSocket) {
        self.record.seq(&socket, n);
    }
}

#[websocket_gateway("/h")]
pub struct TinyHookGateway {
    record: Arc<Record>,
}

#[handlers]
impl TinyHookGateway {
    #[subscribe_message("seq")]
    fn seq(&self, #[message_body] n: i64, #[connected_socket] socket: IoSocket) {
        self.record.seq(&socket, n);
    }
}

static SPIN: AtomicU64 = AtomicU64::new(0);

#[async_trait]
impl OnGatewayConnection<IoSocket> for TinyHookGateway {
    /// A hook of 50 to 810 microseconds that never yields: the gate opens from the hook's task while the connection is
    /// dispatching the burst that arrived with the CONNECT.
    async fn handle_connection(&self, _socket: IoSocket) {
        let k = SPIN.fetch_add(1, Ordering::Relaxed);
        let until = std::time::Instant::now() + Duration::from_micros(50 + (k % 20) * 40);
        while std::time::Instant::now() < until {
            std::hint::spin_loop();
        }
    }
}

#[module(providers = [Record, NoHookGateway, TinyHookGateway])]
pub struct RaceModule;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn events_written_with_the_connect_start_in_arrival_order_while_the_gate_opens() {
    const ROUNDS: usize = 1000;
    const EVENTS: usize = 30;
    const BATCH: usize = 100;
    let (mut app, addr) = start::<RaceModule>(IoAdapter::new()).await;
    let record = app.get::<Record>().unwrap();
    let mut source = 20u8;
    for nsp in ["/o", "/h"] {
        let mut ids = Vec::new();
        while ids.len() < ROUNDS {
            // A fresh source address per batch: the sessions of a batch stay open until its events ran.
            source += 1;
            let mut clients = Vec::with_capacity(BATCH);
            for _ in 0..BATCH {
                let mut client = Raw::connect_from(addr, local(source)).await;
                // The 30 events in one write as soon as the CONNECT is answered: they arrive while the connection hook
                // runs or just as its task opens the gate.
                ids.push(client.join(nsp, None).await);
                let packets: Vec<String> = (0..EVENTS).map(|n| format!(r#"2{nsp},["seq",{n}]"#)).collect();
                client.send_batch(&packets).await;
                clients.push(client);
            }
            let batch: Vec<String> = ids[ids.len() - BATCH..].to_vec();
            record
                .wait_until("the events of a batch", |r| {
                    let seqs = r.seqs.lock().unwrap();
                    batch.iter().all(|id| seqs.get(id).is_some_and(|s| s.len() == EVENTS))
                })
                .await;
            for client in clients {
                client.close().await;
            }
        }
        let seqs = record.seqs.lock().unwrap();
        let out_of_order: Vec<&Vec<i64>> = ids.iter().filter_map(|id| seqs.get(id)).filter(|s| s.windows(2).any(|w| w[0] > w[1])).collect();
        assert_eq!(ids.len(), ROUNDS);
        assert!(
            out_of_order.is_empty(),
            "{nsp}: {} of {ROUNDS} sockets saw their events start out of order, e.g. {:?}",
            out_of_order.len(),
            out_of_order.first()
        );
    }
    app.close().await.unwrap();
}

// ------------------------------------------------------------------------------------ B2: the disconnect-hook queue

/// Presence kept by a slow store: the hook records who left and waits for the test's permit.
#[websocket_gateway("/d")]
pub struct PresenceGateway {
    record: Arc<Record>,
}

#[handlers]
impl PresenceGateway {}

#[async_trait]
impl OnGatewayDisconnect<IoSocket> for PresenceGateway {
    async fn handle_disconnect(&self, socket: IoSocket) {
        let who = socket.handshake().auth().get("who").and_then(Value::as_str).unwrap_or("?").to_string();
        self.record.push(format!("start:{who}"));
        self.record.gate.0.acquire().await.unwrap().forget();
        self.record.push(format!("end:{who}"));
    }
}

/// A namespace without hooks: the connection handles packets in arrival order, so when its handler logs, every packet
/// the client sent before was handled.
#[websocket_gateway("/m")]
pub struct MarkerGateway {
    record: Arc<Record>,
}

#[handlers]
impl MarkerGateway {
    #[subscribe_message("mark")]
    fn mark(&self, #[message_body] n: u64) {
        self.record.push(format!("mark:{n}"));
    }
}

#[module(providers = [Record, PresenceGateway, MarkerGateway])]
pub struct PresenceModule;

/// One socket on `/d` from `source`, as `who`, that leaves at once.
async fn visit(addr: SocketAddr, source: u8, who: &str) {
    let mut client = Raw::connect_from(addr, local(source)).await;
    client.join("/d", Some(&format!(r#"{{"who":"{who}"}}"#))).await;
    client.close().await;
}

/// The reviewer's scenario: attackers from several addresses fill the workers and the queue with slow hooks; a victim
/// from another address leaves. Its hook starts after at most one hook of each attacker address, however many each
/// queued (measured in hooks started before it, not in time), and runs exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_victims_disconnect_hook_starts_after_at_most_one_hook_per_other_address() {
    const ATTACKERS: u8 = 4;
    const CYCLES: usize = 30;
    let (mut app, addr) = start::<PresenceModule>(IoAdapter::new()).await;
    let record = app.get::<Record>().unwrap();
    let mut attackers = Vec::new();
    for a in 0..ATTACKERS {
        attackers.push(tokio::spawn(async move {
            for _ in 0..CYCLES {
                visit(addr, 101 + a, &format!("a{a}")).await;
            }
        }));
    }
    for attacker in attackers {
        attacker.await.unwrap();
    }
    // 32 workers, 8 per address: four addresses occupy all of them; the other 88 sockets wait.
    record.wait_until("the workers to fill", |r| r.count("start:") >= 32).await;
    // The victim leaves `/d` and then marks on `/m`: once the marker is handled, its hook is queued.
    let mut victim = Raw::connect_from(addr, local(150)).await;
    victim.join("/d", Some(r#"{"who":"victim"}"#)).await;
    victim.join("/m", None).await;
    victim.send("1/d,").await;
    victim.send(r#"2/m,["mark",1]"#).await;
    record.wait_until("the victim's marker", |r| r.count("mark:1") == 1).await;
    assert_eq!(record.count("start:victim"), 0);
    // Let one hook end at a time: each frees a worker, which starts the next hook.
    let mut before_victim = 0;
    loop {
        let started = record.count("start:");
        record.gate.0.add_permits(1);
        record.wait_until("the next hook to start", |r| r.count("start:") > started).await;
        if record.count("start:victim") == 1 {
            break;
        }
        before_victim += 1;
        assert!(before_victim <= 4 * CYCLES, "the victim's hook never started");
    }
    assert!(
        before_victim <= ATTACKERS as usize,
        "{before_victim} hooks of other addresses started before the victim's; at most one per address ({ATTACKERS}) may"
    );
    // Everything else runs too, each hook once.
    record.gate.0.add_permits(ATTACKERS as usize * CYCLES + 1);
    record.wait_until("every hook to end", |r| r.count("end:") == ATTACKERS as usize * CYCLES + 1).await;
    assert_eq!(record.count("start:victim"), 1);
    assert_eq!(record.count("end:victim"), 1);
    assert_eq!(record.count("start:"), ATTACKERS as usize * CYCLES + 1);
    drop(victim);
    app.close().await.unwrap();
}

/// The status line of a polling handshake from `source`.
async fn handshake_status(addr: SocketAddr, source: u8) -> u16 {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::new(local(source), 0)).unwrap();
    let mut stream = socket.connect(addr).await.unwrap();
    stream.write_all(b"GET /socket.io/?EIO=4&transport=polling HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut head = [0u8; 12];
    stream.read_exact(&mut head).await.unwrap();
    std::str::from_utf8(&head[9..12]).unwrap().parse().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_address_at_its_share_of_pending_hooks_gets_no_session_until_they_drain() {
    // 128 pending hooks in the application: each address's share is 2.
    let (mut app, addr) = start::<PresenceModule>(IoAdapter::new().max_pending_disconnect_hooks(128)).await;
    let record = app.get::<Record>().unwrap();
    visit(addr, 160, "first").await;
    visit(addr, 160, "second").await;
    record.wait_until("both hooks to start", |r| r.count("start:") == 2).await;
    assert_eq!(handshake_status(addr, 160).await, 503, "an address at its share opens no session");
    assert_eq!(handshake_status(addr, 161).await, 200, "another address does");
    // Within its share no hook was skipped; once they end the address is served again.
    record.gate.0.add_permits(2);
    record.wait_until("both hooks to end", |r| r.count("end:") == 2).await;
    let recovered = async {
        while handshake_status(addr, 160).await != 200 {
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), recovered).await.expect("the address is served again once its hooks drained");
    assert_eq!(record.count("start:first") + record.count("start:second"), 2, "each hook ran once");
    app.close().await.unwrap();
}

// ------------------------------------------------------------------------------- an Engine.IO path that hides a route

#[controller("status")]
pub struct StatusController;

#[routes]
impl StatusController {
    #[get("ping")]
    fn ping(&self) -> &'static str {
        "pong"
    }
}

#[websocket_gateway(path = "/status")]
pub struct StatusPathGateway;

#[module(controllers = [StatusController], providers = [StatusPathGateway])]
pub struct HiddenRouteModule;

#[websocket_gateway(path = "/realtime")]
pub struct RealtimeGateway;

#[module(controllers = [StatusController], providers = [RealtimeGateway])]
pub struct VisibleRouteModule;

#[tokio::test]
async fn an_engine_path_under_which_a_route_lives_fails_at_startup_naming_both() {
    let mut app = CorvisFactory::create::<HiddenRouteModule>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    let message = format!("{:#}", app.init().await.unwrap_err());
    assert!(message.contains("/status/") && message.contains("StatusPathGateway") && message.contains("GET /status/ping"), "{message}");
    // Under the default adapter the gateway's `path` is a WebSocket path and hides nothing.
    let mut app = CorvisFactory::create::<HiddenRouteModule>().await.unwrap();
    app.init().await.unwrap();
    let mut app = CorvisFactory::create::<VisibleRouteModule>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    app.init().await.unwrap();
}

#[tokio::test]
async fn zero_caps_of_pending_disconnect_hooks_fail_init() {
    for adapter in [IoAdapter::new().max_pending_disconnect_hooks(0), IoAdapter::new().max_pending_disconnect_hook_bytes(0)] {
        let mut app = CorvisFactory::create::<PresenceModule>().await.unwrap();
        app.use_websocket_adapter(adapter);
        let message = format!("{:#}", app.init().await.unwrap_err());
        assert!(message.contains("max_pending_disconnect_hooks") && message.contains("above zero"), "{message}");
    }
}
