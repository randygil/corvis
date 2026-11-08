//! `corvis posture` end to end against a fake `cargo`: the test binary itself plays the part of `cargo build` and
//! `cargo run` (selected with environment variables), so the output filtering, the nonce, the exit codes, the
//! timeout and the cleanup of the process tree are exercised without building a project.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SCENARIO: &str = "FAKE_CARGO_SCENARIO";
const SUB: &str = "FAKE_CARGO_SUB";
const PIDFILE: &str = "FAKE_CARGO_PIDFILE";

fn marker(kind: &str, nonce: Option<&str>) -> String {
    match nonce {
        Some(n) => format!("----- {kind} CORVIS ROUTE POSTURE REPORT {n} -----"),
        None => format!("----- {kind} CORVIS ROUTE POSTURE REPORT -----"),
    }
}

const CLEAN_ROW: &str = "GET  /  AppController::get_hello  public  [no guards]";
const OPEN_ROW: &str = "GET  /open  AppController::open  UNPROTECTED  [no guards]";

fn print_report(nonce: Option<&str>, rows: &[&str]) {
    println!("{}", marker("BEGIN", nonce));
    for r in rows {
        println!("{r}");
    }
    println!("{}", marker("END", nonce));
}

/// The fake `cargo`. Does nothing unless the parent test selected a scenario.
#[test]
fn fake_cargo() {
    let Ok(scenario) = std::env::var(SCENARIO) else { return };
    // libtest wrote "test fake_cargo ... " without a newline: start on a fresh line.
    println!();
    if scenario == "sleeper" {
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    if std::env::var(SUB).as_deref() != Ok("run") {
        std::process::exit(0);
    }
    let nonce = std::env::var("CORVIS_ROUTE_POSTURE_NONCE").ok();
    let nonce = nonce.as_deref();
    println!("INFO log line before the report");
    match scenario.as_str() {
        "clean" => {
            print_report(nonce, &[CLEAN_ROW]);
            std::process::exit(40);
        }
        "unprotected" => {
            print_report(nonce, &[CLEAN_ROW, OPEN_ROW]);
            std::process::exit(41);
        }
        "unprotected-wrong-code" => {
            print_report(nonce, &[OPEN_ROW]);
            std::process::exit(0);
        }
        "clean-wrong-code" => {
            print_report(nonce, &[CLEAN_ROW]);
            std::process::exit(0);
        }
        "inject" => {
            // Output of the application or a library: a fake clean report with the plain markers, then a lone
            // BEGIN, before the real report.
            print_report(None, &["(no routes)"]);
            println!("{}", marker("BEGIN", None));
            print_report(nonce, &[CLEAN_ROW, OPEN_ROW]);
            std::process::exit(41);
        }
        "helper" => {
            let mut sleeper = Command::new(std::env::current_exe().unwrap());
            sleeper.args(["--exact", "fake_cargo", "--nocapture"]).env(SCENARIO, "sleeper").stdin(Stdio::null()).stderr(Stdio::null());
            let sleeper = sleeper.spawn().unwrap();
            std::fs::write(std::env::var(PIDFILE).unwrap(), sleeper.id().to_string()).unwrap();
            print_report(nonce, &[CLEAN_ROW]);
            std::process::exit(40);
        }
        "hang" => std::thread::sleep(Duration::from_secs(60)),
        "none" => std::process::exit(1),
        other => panic!("unknown scenario {other}"),
    }
}

struct Fixture {
    dir: PathBuf,
    cargo: PathBuf,
}

#[cfg(windows)]
fn write_wrapper(dir: &Path, exe: &Path, args: &str) -> PathBuf {
    let path = dir.join("fake-cargo.cmd");
    std::fs::write(
        &path,
        format!(
            "@echo off
set {SUB}=%1
\"{}\" {args}
exit /b %ERRORLEVEL%
",
            exe.display()
        ),
    )
    .unwrap();
    path
}

#[cfg(unix)]
fn write_wrapper(dir: &Path, exe: &Path, args: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-cargo.sh");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh
export {SUB}=\"$1\"
exec \"{}\" {args}
",
            exe.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("corvis-cli-posture-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let args = "--exact fake_cargo --nocapture --test-threads=1";
    let cargo = write_wrapper(&dir, &exe, args);
    Fixture { dir, cargo }
}

struct Outcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

fn posture(f: &Fixture, scenario: &str, args: &[&str]) -> Outcome {
    let started = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_corvis"))
        .arg("posture")
        .args(args)
        .env("CORVIS_CARGO", &f.cargo)
        .env(SCENARIO, scenario)
        .env(PIDFILE, f.dir.join("sleeper.pid"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    Outcome {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        elapsed: started.elapsed(),
    }
}

fn process_alive(pid: u32) -> bool {
    if cfg!(windows) {
        let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    } else {
        Command::new("kill").args(["-0", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }
}

fn wait_until_gone(pid: u32) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if !process_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn cleanup(f: &Fixture) {
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn a_clean_report_exits_zero_and_stdout_has_only_the_report() {
    let f = fixture("clean");
    let o = posture(&f, "clean", &[]);
    assert_eq!(o.code, Some(0), "{}\n{}", o.stdout, o.stderr);
    assert_eq!(o.stdout, format!("{CLEAN_ROW}\n"));
    assert!(o.stderr.contains("INFO log line before the report"), "{}", o.stderr);
    cleanup(&f);
}

#[test]
fn an_unprotected_report_exits_one() {
    let f = fixture("unprotected");
    let o = posture(&f, "unprotected", &[]);
    assert_eq!(o.code, Some(1), "{}\n{}", o.stdout, o.stderr);
    assert_eq!(o.stdout, format!("{CLEAN_ROW}\n{OPEN_ROW}\n"));
    cleanup(&f);
}

#[test]
fn an_unprotected_report_is_one_even_if_the_application_exits_with_zero() {
    let f = fixture("wrong");
    let o = posture(&f, "unprotected-wrong-code", &[]);
    assert_eq!(o.code, Some(1), "{}\n{}", o.stdout, o.stderr);
    assert_eq!(o.stdout, format!("{OPEN_ROW}\n"));
    cleanup(&f);
}

#[test]
fn a_clean_report_with_an_ordinary_exit_code_is_not_trusted() {
    let f = fixture("clean-wrong");
    let o = posture(&f, "clean-wrong-code", &[]);
    assert_eq!(o.code, Some(2), "{}\n{}", o.stdout, o.stderr);
    assert!(o.stdout.is_empty(), "{}", o.stdout);
    assert!(o.stderr.contains("not trusted"), "{}", o.stderr);
    cleanup(&f);
}

#[test]
fn fake_markers_printed_by_the_application_cannot_replace_the_report() {
    let f = fixture("inject");
    let o = posture(&f, "inject", &[]);
    assert_eq!(o.code, Some(1), "{}\n{}", o.stdout, o.stderr);
    assert_eq!(o.stdout, format!("{CLEAN_ROW}\n{OPEN_ROW}\n"));
    assert!(o.stderr.contains("(no routes)"), "the fake report is ordinary output: {}", o.stderr);
    cleanup(&f);
}

#[test]
fn no_report_exits_two_with_the_exit_code() {
    let f = fixture("none");
    let o = posture(&f, "none", &[]);
    assert_eq!(o.code, Some(2), "{}\n{}", o.stdout, o.stderr);
    assert!(o.stdout.is_empty());
    assert!(o.stderr.contains("exit code 1") && o.stderr.contains("without producing a route posture report"), "{}", o.stderr);
    cleanup(&f);
}

#[test]
fn a_helper_that_keeps_stdout_open_neither_hangs_the_command_nor_outlives_it() {
    let f = fixture("helper");
    let o = posture(&f, "helper", &["--timeout", "60"]);
    assert_eq!(o.code, Some(0), "{}\n{}", o.stdout, o.stderr);
    assert_eq!(o.stdout, format!("{CLEAN_ROW}\n"));
    assert!(o.elapsed < Duration::from_secs(30), "took {:?}", o.elapsed);
    let pid: u32 = std::fs::read_to_string(f.dir.join("sleeper.pid")).unwrap().trim().parse().unwrap();
    assert!(wait_until_gone(pid), "the helper process {pid} is still running");
    cleanup(&f);
}

#[test]
fn the_timeout_bounds_the_total_time_and_exits_two() {
    let f = fixture("hang");
    let o = posture(&f, "hang", &["--timeout", "2"]);
    assert_eq!(o.code, Some(2), "{}\n{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("within 2 seconds"), "{}", o.stderr);
    assert!(o.elapsed < Duration::from_secs(30), "took {:?}", o.elapsed);
    cleanup(&f);
}
