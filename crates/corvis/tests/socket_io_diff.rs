//! Entry point of the Socket.IO differential harness (`parity/socket-io/`).
//!
//! The harness runs the same scenarios against a Node `socket.io` server and, once Corvis implements Engine.IO and
//! Socket.IO, against Corvis, and fails on any difference. It needs Node 22 or later, so it is opt in: without
//! `CORVIS_NODE_DIFF=1` this test prints a notice and passes. With it, a missing Node or a missing install fails the
//! test. This test never installs packages: run `npm ci` in `parity/socket-io/` first (see CONTRIBUTING.md).

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

#[test]
fn socket_io_differential_harness() {
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

    let selftests = run_node(&dir, &["--test", "test/*.test.mjs"]);
    println!("{}", selftests.output);
    assert!(selftests.success, "the harness self-tests failed (output above)");

    // From the Engine.IO change on, this is where the Corvis server is started in-process on 127.0.0.1:0 and
    // `--corvis <url> --corvis-capabilities <list>` is passed to the runner.
    let runner = run_node(&dir, &["run.mjs"]);
    println!("{}", runner.output);
    assert!(runner.success, "the differential runner reported a difference or an error:\n{}", runner.output);
}
