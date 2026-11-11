//! Entry point of the Socket.IO differential harness (`parity/socket-io/`).
//!
//! The harness runs the same scenarios against a Node `socket.io` / `engine.io` server and against Corvis, and fails on
//! any difference. With the `socket-io` feature the Corvis target is the Engine.IO server of the harness application
//! (`engine_support/harness_app.rs`), with the Socket.IO scenario app (`engine_support/io_app.rs`) beside it, started in-process on 127.0.0.1:0 with the capabilities `engine-io` and `socket-io`; scenarios that
//! need `redis` run on Node only. The gateway scenarios (capability `gateways`) run against a second in-process application,
//! the same gateways written with `#[websocket_gateway]` over the Socket.IO adapter (`engine_support/gateway_app.rs`),
//! and against the reference gateway application of `parity/socket-io/gateway-reference/`, which needs its own `npm ci`. The test needs Node 22 or later, so it is opt in: without `CORVIS_NODE_DIFF=1` it
//! prints a notice and passes. With it, a missing Node or a missing install fails the test. This test never installs
//! packages: run `npm ci` in `parity/socket-io/` first (see CONTRIBUTING.md).

#[cfg(feature = "socket-io")]
mod engine_support;

use std::io::Read;
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

struct Run {
    success: bool,
    output: String,
}

fn run_node(dir: &PathBuf, args: &[&str]) -> Run {
    let child = Command::new("node")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start `node {}`: {e}", args.join(" ")));
    let mut guard = KillOnDrop(child);
    let mut stdout = guard.0.stdout.take().expect("stdout is piped");
    let mut stderr = guard.0.stderr.take().expect("stderr is piped");
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
    let status = guard.0.wait().expect("wait for node");
    let mut output = out.join().unwrap_or_default();
    output.push_str(&err.join().unwrap_or_default());
    Run { success: status.success(), output }
}

fn node_major_version() -> u32 {
    let output = Command::new("node")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("CORVIS_NODE_DIFF=1 needs Node 22 or later on PATH, but `node` cannot be started: {e}"));
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse().ok())
        .unwrap_or_else(|| panic!("cannot read the Node version from `{}`", text.trim()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn socket_io_differential_harness() {
    if std::env::var("CORVIS_NODE_DIFF").as_deref() != Ok("1") {
        println!("skipping socket_io_diff: set CORVIS_NODE_DIFF=1 (see CONTRIBUTING.md)");
        return;
    }

    let major = node_major_version();
    assert!(major >= 22, "CORVIS_NODE_DIFF=1 needs Node 22 or later, found major version {major}");

    let dir = harness_dir();
    assert!(
        dir.join("node_modules").join("socket.io").join("package.json").is_file(),
        "the Socket.IO harness is not installed: run `npm ci` in parity/socket-io/ (cargo never installs packages)"
    );

    assert!(
        dir.join("gateway-reference").join("node_modules").join("@nestjs").join("core").join("package.json").is_file(),
        "the gateway reference is not installed: run `npm ci` in parity/socket-io/gateway-reference/ (cargo never installs packages)"
    );

    // Node blocks its caller: run it off the runtime so the in-process Corvis server keeps serving.
    let selftest_dir = dir.clone();
    let selftests = tokio::task::spawn_blocking(move || run_node(&selftest_dir, &["--test", "test/*.test.mjs"])).await.unwrap();
    println!("{}", selftests.output);
    assert!(selftests.success, "the harness self-tests failed (output above)");

    // The Corvis target is started in-process on 127.0.0.1:0.
    #[cfg(feature = "socket-io")]
    let (_app, _gateway_app, runner) = {
        let (app, addr, _engines) = engine_support::harness_app::start().await;
        let (gateway_app, gateway_addr) = engine_support::gateway_app::start().await;
        let url = format!("http://{addr}");
        let gateway_url = format!("http://{gateway_addr}");
        let runner_dir = dir.clone();
        let runner = tokio::task::spawn_blocking(move || {
            run_node(
                &runner_dir,
                &["run.mjs", "--corvis", &url, "--corvis-capabilities", "engine-io,socket-io", "--corvis-gateways", &gateway_url],
            )
        })
        .await
        .unwrap();
        (app, gateway_app, runner)
    };
    #[cfg(not(feature = "socket-io"))]
    let runner = {
        let runner_dir = dir.clone();
        tokio::task::spawn_blocking(move || run_node(&runner_dir, &["run.mjs"])).await.unwrap()
    };
    println!("{}", runner.output);
    assert!(runner.success, "the differential runner reported a difference or an error:\n{}", runner.output);
}
