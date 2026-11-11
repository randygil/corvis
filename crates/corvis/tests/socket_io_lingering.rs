#![cfg(feature = "socket-io")]
//! Handlers that outlive their connection, and packets that race a namespace leave, over real sockets.
//!
//! A client that reconnects in a loop used to leave every handler it had started running for `close_timeout`, without
//! charge and without a cap: eight churning connections from one address reached tens of thousands of futures and
//! tens of megabytes. The client's drop now cancels them at once; where the application asked for a grace
//! (`finish_handlers_after_client_close`), or the server closes the connection, the handlers that linger count against
//! the address and an address at its cap is refused new sessions. The live heap is measured with a counting allocator.

mod engine_support;
mod socket_io_raw;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::socket_io::{EngineOptions, IoOptions, IoServer, IoSocket};
use engine_support::eventually;
use engine_support::io_client::start;
use socket_io_raw::{Raw, ip};

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_add(new_size as isize - layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// One test at a time: the counter is the process's.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

/// Counts the handlers that run (`block` never returns): +1 when one starts, -1 when its future is dropped.
#[derive(Clone, Default)]
struct Counts {
    running: Arc<AtomicIsize>,
    started: Arc<AtomicUsize>,
}

struct Running(Arc<AtomicIsize>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn register(io: &IoServer, counts: &Counts) {
    let counts = counts.clone();
    io.on_connection(move |socket| {
        let counts = counts.clone();
        socket.on("block", move |_socket, _args, _ack| {
            let counts = counts.clone();
            async move {
                counts.running.fetch_add(1, Ordering::SeqCst);
                counts.started.fetch_add(1, Ordering::SeqCst);
                let _running = Running(counts.running.clone());
                futures::future::pending::<()>().await
            }
        });
        socket.on("echo", |_socket, _args, ack| async move {
            if let Some(ack) = ack {
                let _ = ack.send(());
            }
        });
    });
}

/// One connection that starts `events` blocked handlers and goes away, as a client in a reconnect loop does. Returns
/// `false` when the server refused the session.
async fn churn_once(addr: std::net::SocketAddr, source: std::net::Ipv4Addr, counts: &Counts, events: usize) -> bool {
    let Ok(mut client) = Raw::open(addr, source).await else { return false };
    if !client.connect("").await.is_some_and(|m| m.starts_with("40")) {
        return false;
    }
    let before = counts.started.load(Ordering::SeqCst);
    for _ in 0..events {
        let _ = client.msg_str(r#"2["block"]"#).await;
    }
    eventually("the handlers of the connection to start", || counts.started.load(Ordering::SeqCst) >= before + events).await;
    drop(client);
    true
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_churn_leaves_no_handlers_and_no_memory_behind() {
    let _guard = ONE_AT_A_TIME.lock().await;
    let counts = Counts::default();
    let fx = start(IoOptions::new(), |io| register(io, &counts)).await;
    let base = live();
    // The shape that reached ~86,000 lingering futures and ~50 MB: eight churners from one address, 32 handlers each.
    let churners: Vec<_> = (0..8)
        .map(|_| {
            let (addr, counts) = (fx.addr, counts.clone());
            tokio::spawn(async move {
                let mut opened = 0;
                for _ in 0..25 {
                    if churn_once(addr, ip(4), &counts, 32).await {
                        opened += 1;
                    }
                }
                opened
            })
        })
        .collect();
    let mut opened = 0;
    for c in churners {
        opened += c.await.unwrap();
    }
    assert!(opened >= 100, "only {opened} sessions were opened");
    let at_end = counts.running.load(Ordering::SeqCst);
    // The handlers are cancelled when their client is gone, not after `close_timeout` (30 s).
    eventually("every handler to be cancelled", || counts.running.load(Ordering::SeqCst) == 0).await;
    eventually("every session to close", || fx.io.engine().session_count() == 0).await;
    // Back to the baseline: what stays is the allocator's and the tasks' own slack, far from tens of megabytes. The
    // frees of the connections that just ended run on their own tasks: the heap is waited for, not sampled once.
    eventually("the heap to come back near its baseline", || live() - base < 4 * 1024 * 1024).await;
    println!(
        "{opened} sessions, {} handlers started, {at_end} running when the last client left, heap {:+} B against the baseline",
        counts.started.load(Ordering::SeqCst),
        live() - base
    );
    fx.io.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_a_grace_requested_lingering_handlers_stay_under_the_per_address_cap_and_new_sessions_are_refused_over_it() {
    let _guard = ONE_AT_A_TIME.lock().await;
    let counts = Counts::default();
    // The cap is `max_sessions_per_ip x max_inflight` = 8 x 4 = 32 lingering handlers per address.
    let engine = EngineOptions::new().max_sessions_per_ip(8).close_timeout(Duration::from_secs(4));
    let options = IoOptions::new().engine(engine).max_inflight(4).finish_handlers_after_client_close(true);
    let cap = options.max_lingering_per_ip();
    assert_eq!(cap, 32);
    let fx = start(options, |io| register(io, &counts)).await;
    let mut refused = 0;
    let mut peak = 0;
    for _ in 0..40 {
        if !churn_once(fx.addr, ip(5), &counts, 4).await {
            refused += 1;
        }
        peak = peak.max(counts.running.load(Ordering::SeqCst));
    }
    println!("peak lingering handlers {peak} (cap {cap}), {refused} of 40 sessions refused");
    assert!(refused > 0, "an address over its cap is refused new sessions");
    assert!(peak <= 2 * cap as isize, "{peak} handlers lingered, above twice the cap of {cap}");
    // An honest address is served meanwhile.
    let mut honest = Raw::open(fx.addr, ip(200)).await.unwrap();
    assert!(honest.connect("").await.unwrap().starts_with("40"));
    // The grace is over after `close_timeout` and the address is admitted again.
    // The grace is real time (the server runs on real sockets): the handlers end when `close_timeout` (4 s) has passed.
    eventually("the lingering handlers to end", || counts.running.load(Ordering::SeqCst) == 0).await;
    assert!(churn_once(fx.addr, ip(5), &counts, 1).await, "the address is served again");
    fx.io.close();
}

// ---------------------------------------------------------------------------------- a namespace the server just left

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn packets_that_race_the_servers_leave_of_a_namespace_are_dropped_and_the_connection_survives() {
    let _guard = ONE_AT_A_TIME.lock().await;
    let sockets: Arc<Mutex<Vec<IoSocket>>> = Arc::new(Mutex::new(Vec::new()));
    let delivered = Arc::new(AtomicUsize::new(0));
    let (s, d) = (sockets.clone(), delivered.clone());
    let fx = start(IoOptions::new(), move |io| {
        let (s, d) = (s.clone(), d.clone());
        io.of("/b").unwrap().on_connection(move |socket| {
            let d = d.clone();
            socket.on("say", move |_socket, _args, _ack| {
                let d = d.clone();
                async move {
                    d.fetch_add(1, Ordering::SeqCst);
                }
            });
            s.lock().unwrap().push(socket);
        });
        io.on_connection(|socket| {
            socket.on("echo", |_socket, _args, ack| async move {
                if let Some(ack) = ack {
                    let _ = ack.send(());
                }
            });
        });
    })
    .await;
    let mut client = Raw::open(fx.addr, ip(1)).await.unwrap();
    assert!(client.connect("").await.unwrap().starts_with("40"));
    assert!(client.connect("/b").await.unwrap().starts_with("40/b"));
    eventually("the socket of /b on the server", || !sockets.lock().unwrap().is_empty()).await;
    // The server leaves /b; the client's packets for it were already on their way: an ACK, an event, a binary event.
    sockets.lock().unwrap()[0].disconnect(false);
    let answer = client.recv_text(Duration::from_secs(5)).await.expect("the DISCONNECT packet");
    assert!(answer.starts_with("41/b"), "{answer}");
    let tx = client.tx();
    tx.msg(b"3/b,0[]").await.unwrap();
    tx.msg(br#"2/b,["say"]"#).await.unwrap();
    tx.msg(br#"51-/b,["say",{"_placeholder":true,"num":0}]"#).await.unwrap();
    tx.bin(&vec![7u8; 100_000]).await.unwrap();
    // The connection and the main namespace are alive, and nothing was delivered for the namespace that was left.
    client.msg_str(r#"27["echo"]"#).await.unwrap();
    let echo = client.recv_text(Duration::from_secs(5)).await;
    assert_eq!(echo.as_deref(), Some("437[]"), "the main namespace still answers");
    assert_eq!(delivered.load(Ordering::SeqCst), 0);
    assert!(!client.is_closed());
    // A namespace the connection never joined is still a violation: the connection is closed.
    client.msg_str(r#"2/never,["say"]"#).await.unwrap();
    assert!(client.wait_closed(Duration::from_secs(5)).await, "a packet for a namespace it never joined closes the connection");
    fx.io.close();
}
