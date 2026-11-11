#![cfg(feature = "socket-io")]
//! What the Socket.IO layer writes to the log when a client provokes it: a handler that panics on the client's input,
//! a connect middleware that is slow. Each kind of line is written once per minute per namespace, and none carries a
//! panic message or any data of a packet. Its own test binary: the logger is process-wide, so the tests share one
//! capturing sink and run one at a time.

mod engine_support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::socket_io::IoOptions;
use corvis::{LogLevel, LoggerService};
use engine_support::io_client::{Raw, start};

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn capture() -> Captured {
    let captured: Captured = Arc::default();
    corvis::logger::set_logger(Capture(captured.clone()));
    corvis::logger::set_log_levels(&LogLevel::all());
    captured
}

/// The lines the Socket.IO layer wrote.
fn lines(captured: &Captured) -> Vec<String> {
    captured.lock().unwrap().iter().filter(|(_, _, ctx)| ctx.as_deref() == Some("SocketIO")).map(|(_, m, _)| m.clone()).collect()
}

async fn wait_for(what: &str, done: impl FnMut() -> bool) {
    engine_support::eventually(what, done).await;
}

#[tokio::test]
async fn a_handler_that_panics_on_client_data_writes_one_line_a_minute_without_the_data_or_the_message() {
    let _serial = SERIAL.lock().await;
    let captured = capture();
    let handled = Arc::new(AtomicUsize::new(0));
    let counter = handled.clone();
    let fx = start(IoOptions::new(), move |io| {
        io.on_connection(move |socket| {
            let counter = counter.clone();
            socket.on("boom", move |_socket, args, _ack| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    // What a handler that unwraps its argument conversions does with a client's bad input.
                    let number: i64 = args.get::<i64>(0).unwrap_or_else(|e| panic!("secret-panic-message {e}"));
                    let _ = number;
                }
            });
        });
    })
    .await;
    let mut client = Raw::connect(fx.addr).await;
    client.join("/", None).await;
    for _ in 0..50 {
        client.send(r#"2["boom","client-data-123"]"#).await;
    }
    wait_for("the 50 handlers", || handled.load(Ordering::SeqCst) == 50).await;
    // No more lines after the first: the absence of a line another task might still write, with nothing that marks its
    // end; a quiet period, not a condition.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let written = lines(&captured);
    assert_eq!(written.len(), 1, "50 panics, one line: {written:?}");
    assert_eq!(written[0], "an event handler of the namespace / panicked");
    assert!(captured.lock().unwrap().iter().all(|(_, m, _)| !m.contains("secret-panic-message") && !m.contains("client-data-123")));
    fx.io.close();
}

#[tokio::test]
async fn panics_in_connection_and_disconnect_handlers_are_limited_and_never_print_the_message() {
    let _serial = SERIAL.lock().await;
    let captured = capture();
    let connected = Arc::new(AtomicUsize::new(0));
    let counter = connected.clone();
    let fx = start(IoOptions::new(), move |io| {
        io.on_connection(move |socket| {
            socket.on_disconnect(|_socket, _reason| panic!("secret-disconnect-message"));
            counter.fetch_add(1, Ordering::SeqCst);
            panic!("secret-connection-message");
        });
    })
    .await;
    let mut clients = Vec::new();
    for _ in 0..6 {
        let mut client = Raw::connect(fx.addr).await;
        client.join("/", None).await;
        clients.push(client);
    }
    wait_for("the connections", || connected.load(Ordering::SeqCst) == 6).await;
    // Every client leaves, so every disconnect handler panics too.
    for client in clients {
        client.close().await;
    }
    wait_for("every socket to leave", || fx.io.main().socket_count() == 0).await;
    // The disconnect handlers run right after their socket left; no more lines than the limit allow: the absence of a
    // line, a quiet period, not a condition.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let written = lines(&captured);
    let connection: Vec<_> = written.iter().filter(|l| l.contains("a connection handler")).collect();
    let disconnect: Vec<_> = written.iter().filter(|l| l.contains("a disconnect handler")).collect();
    assert_eq!(connection.len(), 1, "{written:?}");
    assert!(disconnect.len() <= 1, "{written:?}");
    assert!(captured.lock().unwrap().iter().all(|(_, m, _)| !m.contains("secret-")), "no panic message reaches the log");
    fx.io.close();
}

#[tokio::test]
async fn a_slow_connect_middleware_warns_once_a_minute_per_namespace() {
    let _serial = SERIAL.lock().await;
    let captured = capture();
    let options = IoOptions::new().middleware_timeout(Duration::from_millis(100));
    let fx = start(options, |io| {
        // Middleware that never finishes.
        io.of("/slow").unwrap().use_middleware(|_socket| async {
            futures::future::pending::<()>().await;
            Ok(())
        });
        io.of("/slower").unwrap().use_middleware(|_socket| async {
            futures::future::pending::<()>().await;
            Ok(())
        });
    })
    .await;
    for _ in 0..8 {
        let mut client = Raw::connect(fx.addr).await;
        client.send("0/slow,").await;
        let answer = client.recv_text().await;
        assert!(answer.starts_with("4/slow,"), "{answer}");
        client.close().await;
    }
    let mut client = Raw::connect(fx.addr).await;
    client.send("0/slower,").await;
    let answer = client.recv_text().await;
    assert!(answer.starts_with("4/slower,"), "{answer}");
    let written = lines(&captured);
    assert_eq!(written.iter().filter(|l| l.contains("/slow ")).count(), 1, "{written:?}");
    assert_eq!(written.iter().filter(|l| l.contains("/slower ")).count(), 1, "another namespace has its own line: {written:?}");
    fx.io.close();
}
