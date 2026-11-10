#![cfg(feature = "socket-io")]
//! Runs the upstream `socket.io-protocol` test suite (fetched by `parity/socket-io/scripts/fetch-suites.mjs`) against
//! the Corvis Socket.IO server, and, as a control, against the Node reference server with the same options, so a
//! failure can be told apart from a defect of the suite.
//!
//! Opt in like `socket_io_diff.rs`: without `CORVIS_NODE_DIFF=1` the tests print a notice and pass. With it, a missing
//! Node, install or suite fails the test; nothing is installed or downloaded here (run `npm ci` and
//! `node scripts/fetch-suites.mjs` in `parity/socket-io/`). The server is the one the suite requires: ping
//! interval 300 ms, ping timeout 200 ms, connect timeout 1 s, CORS for any origin, `auth` emitted on connect, `message`
//! echoed as `message-back`, `message-with-ack` acknowledged, the namespace `/custom`, at `/socket.io/`. It binds to
//! 127.0.0.1.

mod engine_support;

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("parity").join("socket-io")
}

/// Kills the child when dropped, so a failed test never leaves a Node process behind.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn enabled() -> bool {
    if std::env::var("CORVIS_NODE_DIFF").as_deref() != Ok("1") {
        println!("skipping socket_io_conformance: set CORVIS_NODE_DIFF=1 (see CONTRIBUTING.md)");
        return false;
    }
    let dir = harness_dir();
    assert!(
        dir.join("node_modules").join("mocha").exists(),
        "the Socket.IO harness is not installed: run `npm ci` in parity/socket-io/ (cargo never installs packages)"
    );
    assert!(
        dir.join("suites").join("socket.io-protocol").join("test-suite.js").exists(),
        "the protocol suite is not fetched: run `node scripts/fetch-suites.mjs` in parity/socket-io/"
    );
    true
}

/// Runs the suite against `url`; returns whether it passed, and its output.
fn run_suite(url: &str) -> (bool, String) {
    let child = Command::new("node")
        .args(["node_modules/mocha/bin/mocha.js", "suites/socket.io-protocol/test-suite.js", "--exit"])
        .env("SUITE_URL", url)
        .current_dir(harness_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start node: {e}"));
    let mut guard = KillOnDrop(child);
    let mut stdout = guard.0.stdout.take().expect("piped");
    let mut stderr = guard.0.stderr.take().expect("piped");
    let out = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let err = thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let status = guard.0.wait().expect("wait for mocha");
    let mut output = out.join().unwrap_or_default();
    output.push_str(&err.join().unwrap_or_default());
    (status.success(), output)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_protocol_suite_passes_against_corvis() {
    if !enabled() {
        return;
    }
    let mut app = corvis::prelude::CorvisFactory::create::<engine_support::AppModule>().await.unwrap();
    let io = engine_support::io_app::suite_server();
    io.attach(&app.get_http_server()).unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let url = format!("http://{addr}");
    let (ok, output) = tokio::task::spawn_blocking(move || run_suite(&url)).await.unwrap();
    println!("{output}");
    assert!(ok, "the socket.io-protocol suite failed against Corvis:\n{output}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_protocol_suite_passes_against_the_node_reference() {
    if !enabled() {
        return;
    }
    let mut child = Command::new("node")
        .args(["reference/socket-server.mjs", "--suite"])
        .current_dir(harness_dir())
        // The reference server exits when its stdin closes: keep the pipe open for the length of the run.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start the reference server: {e}"));
    let stdout = child.stdout.take().expect("piped");
    let stdin = child.stdin.take();
    let _guard = KillOnDrop(child);
    let mut first = String::new();
    BufReader::new(stdout).read_line(&mut first).expect("the reference server prints its address");
    let info: serde_json::Value = serde_json::from_str(first.trim()).unwrap_or_else(|e| panic!("bad address line {first:?}: {e}"));
    let url = format!("http://{}:{}", info["address"].as_str().unwrap(), info["port"]);
    assert_eq!(info["address"], "127.0.0.1");
    let (ok, output) = tokio::task::spawn_blocking(move || run_suite(&url)).await.unwrap();
    drop(stdin);
    println!("{output}");
    assert!(ok, "the socket.io-protocol suite failed against the Node reference (a defect of the suite or its setup):\n{output}");
}
