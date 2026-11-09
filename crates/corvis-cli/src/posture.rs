//! `corvis posture`: runs the project in report mode and prints the route posture report.
//!
//! The application prints the report between two marker lines (it shares stdout with its log lines) and ends
//! the process with exit code 40 when nothing is unprotected and 41 otherwise (the CLI trusts the report, not 41). This module generates a nonce
//! per run that the markers must contain, separates the report from the other output, bounds the total time,
//! makes sure no process of the run outlives the command, and maps the outcome to the command's exit status.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

mod tree;

/// Opens the report when there is no nonce. Equal to `corvis::posture::REPORT_BEGIN`; the CLI does not depend
/// on the framework.
pub const REPORT_BEGIN: &str = "----- BEGIN CORVIS ROUTE POSTURE REPORT -----";
/// Closes the report when there is no nonce. Equal to `corvis::posture::REPORT_END`.
pub const REPORT_END: &str = "----- END CORVIS ROUTE POSTURE REPORT -----";
/// The environment variable that turns the framework's report mode on.
pub const REPORT_ENV: &str = "CORVIS_ROUTE_POSTURE_REPORT";
/// The environment variable that carries the per-run nonce the markers must contain.
pub const NONCE_ENV: &str = "CORVIS_ROUTE_POSTURE_NONCE";
/// The exit code of a report-mode application with nothing unprotected.
pub const EXIT_CLEAN: i32 = 40;
/// Seconds the application gets to print the report once it is running, unless `--timeout` says otherwise.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// How long the command keeps reading the application's stdout after the application ended: a process the
/// application started may hold the pipe open forever.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// The marker lines of a run.
pub fn markers(nonce: Option<&str>) -> (String, String) {
    match nonce {
        Some(n) => {
            (format!("----- BEGIN CORVIS ROUTE POSTURE REPORT {n} -----"), format!("----- END CORVIS ROUTE POSTURE REPORT {n} -----"))
        }
        None => (REPORT_BEGIN.to_string(), REPORT_END.to_string()),
    }
}

/// 32 hexadecimal digits from the operating system's randomness (the keys of `RandomState`).
fn new_nonce() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::with_capacity(32);
    for round in 0..2u64 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u32(std::process::id());
        hasher.write_u64(round);
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

/// The options of `corvis posture`.
#[derive(Debug, PartialEq, Eq)]
pub struct PostureArgs {
    pub json: bool,
    pub release: bool,
    pub timeout: Duration,
    /// Everything after the first `--`, given to the application.
    pub program: Vec<String>,
}

/// Reads `[--json] [--release|--prod] [--timeout <seconds>] [-- <program arguments>]`.
pub fn parse_args(args: &[String]) -> Result<PostureArgs, String> {
    let (own, program) = match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], args[i + 1..].to_vec()),
        None => (args, Vec::new()),
    };
    let mut parsed = PostureArgs { json: false, release: false, timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS), program };
    let mut i = 0;
    while i < own.len() {
        let arg = own[i].as_str();
        match arg {
            "--json" => parsed.json = true,
            "--release" | "--prod" => parsed.release = true,
            "--watch" => return Err("corvis posture does not support --watch".into()),
            "--timeout" => {
                i += 1;
                let value = own.get(i).ok_or("--timeout needs a number of seconds")?;
                parsed.timeout = parse_timeout(value)?;
            }
            _ => match arg.strip_prefix("--timeout=") {
                Some(value) => parsed.timeout = parse_timeout(value)?,
                None => return Err(format!("unknown option \"{arg}\" for corvis posture (see corvis --help)")),
            },
        }
        i += 1;
    }
    Ok(parsed)
}

fn parse_timeout(value: &str) -> Result<Duration, String> {
    match value.parse::<u64>() {
        Ok(n) if n > 0 => Ok(Duration::from_secs(n)),
        _ => Err(format!("--timeout needs a whole number of seconds greater than 0 (got \"{value}\")")),
    }
}

/// What to do with one line of the application's stdout.
#[derive(Debug, PartialEq, Eq)]
pub enum Feed {
    /// Not part of the report: forward it to stderr.
    Other(String),
    /// Part of the report, kept inside the filter.
    Held,
    /// A new BEGIN marker arrived while a report was open: the abandoned lines (the old BEGIN included) are
    /// output of the application after all, and the new report starts.
    Restart(Vec<String>),
    /// The END marker was read: the complete report (marker lines excluded).
    Report(String),
}

/// Separates the reports from the rest of the output, line by line. The caller keeps the last one.
#[derive(Debug)]
pub struct ReportFilter {
    begin: String,
    end: String,
    inside: bool,
    held: Vec<String>,
}

impl ReportFilter {
    pub fn new(nonce: Option<&str>) -> Self {
        let (begin, end) = markers(nonce);
        ReportFilter { begin, end, inside: false, held: Vec::new() }
    }

    pub fn feed(&mut self, line: &str) -> Feed {
        let line = line.trim_end_matches(['\r', '\n']);
        if !self.inside {
            if line == self.begin {
                self.inside = true;
                return Feed::Held;
            }
            return Feed::Other(line.to_string());
        }
        if line == self.end {
            self.inside = false;
            let mut report = self.held.join("\n");
            if !self.held.is_empty() {
                report.push('\n');
            }
            self.held.clear();
            return Feed::Report(report);
        }
        if line == self.begin {
            let mut abandoned = vec![self.begin.clone()];
            abandoned.append(&mut self.held);
            return Feed::Restart(abandoned);
        }
        self.held.push(line.to_string());
        Feed::Held
    }

    /// Lines held for a report that never ended; they were output of the application after all.
    pub fn finish(&mut self) -> Vec<String> {
        if !self.inside {
            return Vec::new();
        }
        self.inside = false;
        let mut lines = vec![self.begin.clone()];
        lines.append(&mut self.held);
        lines
    }
}

/// Splits `lines` into the last complete report and the other lines. A report without an END marker is not a
/// report: its lines come back in the other lines.
#[cfg(test)]
pub fn split_report(lines: &[&str], nonce: Option<&str>) -> (Option<String>, Vec<String>) {
    let mut filter = ReportFilter::new(nonce);
    let (mut report, mut other) = (None, Vec::new());
    for line in lines {
        match filter.feed(line) {
            Feed::Other(l) => other.push(l),
            Feed::Held => {}
            Feed::Restart(mut lines) => other.append(&mut lines),
            Feed::Report(r) => report = Some(r),
        }
    }
    other.extend(filter.finish());
    (report, other)
}

/// Whether a report (marker lines excluded) lists an unprotected entry, read from the report itself and not
/// from the application's exit code.
pub fn report_has_unprotected(report: &str, json: bool) -> bool {
    if json { report.contains("\"state\": \"unprotected\"") } else { report.lines().any(|l| l.contains("  UNPROTECTED  [")) }
}

/// The command's exit status. `report` is `Some(has_unprotected)` when a complete report was selected. A report
/// with an unprotected entry is 1 whatever the application's exit code; a clean report counts only when the
/// application ended with the report-mode code for it; everything else is 2.
pub fn exit_status(report: Option<bool>, child_code: Option<i32>) -> u8 {
    match (report, child_code) {
        (Some(true), _) => 1,
        (Some(false), Some(EXIT_CLEAN)) => 0,
        _ => 2,
    }
}

fn no_report_message(report: Option<bool>, child_code: Option<i32>, timeout: Option<Duration>) -> String {
    if let Some(t) = timeout {
        return format!(
            "the application did not print the route posture report within {} seconds and was stopped: it may never reach init(); \
             use --timeout <seconds> to wait longer",
            t.as_secs()
        );
    }
    let code = child_code.map_or("none, ended by a signal".to_string(), |c| c.to_string());
    if report.is_some() {
        return format!(
            "the application printed a report but ended with exit code {code} instead of {EXIT_CLEAN}: the report is not trusted"
        );
    }
    format!("the application ended (exit code {code}) without producing a route posture report: check the build and startup output above")
}

enum Msg {
    Line(String),
    End,
}

/// Reads `stdout` line by line on its own thread, so that the caller can stop waiting for it.
fn spawn_reader(stdout: std::process::ChildStdout) -> mpsc::Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(Msg::Line(String::from_utf8_lossy(&buf).into_owned())).is_err() {
                        return;
                    }
                }
            }
        }
        let _ = tx.send(Msg::End);
    });
    rx
}

fn fail(message: &str) -> ExitCode {
    eprintln!("\x1b[31mError:\x1b[0m {message}");
    ExitCode::from(2)
}

pub(crate) fn cargo_program() -> String {
    std::env::var("CORVIS_CARGO").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "cargo".to_string())
}

/// Runs `corvis posture`.
pub fn run(args: &[String]) -> ExitCode {
    let args = match parse_args(args) {
        Ok(a) => a,
        Err(e) => return fail(&e),
    };
    let cargo = cargo_program();

    // Build first: the timeout only counts once the application runs, and a build failure is its own message.
    let mut build = Command::new(&cargo);
    build.arg("build");
    if args.release {
        build.arg("--release");
    }
    match build.stdout(Stdio::null()).status() {
        Ok(s) if s.success() => {}
        Ok(s) => return fail(&format!("cargo build failed ({s}), so there is no route posture report")),
        Err(e) => return fail(&format!("failed to run cargo: {e}")),
    }

    let nonce = new_nonce();
    let mut run = Command::new(&cargo);
    run.arg("run");
    if args.release {
        run.arg("--release");
    }
    if !args.program.is_empty() {
        run.arg("--").args(&args.program);
    }
    run.env(REPORT_ENV, if args.json { "json" } else { "text" }).env(NONCE_ENV, &nonce).stdin(Stdio::null()).stdout(Stdio::piped());
    tree::prepare(&mut run);
    tree::install_interrupt_handler();
    let started = Instant::now();
    let mut child = match run.spawn() {
        Ok(c) => c,
        Err(e) => return fail(&format!("failed to run cargo: {e}")),
    };
    // Dropping it ends every process of the run, the application's helpers included.
    let tree = tree::Tree::adopt(&child);
    let rx = spawn_reader(child.stdout.take().expect("piped stdout"));
    let deadline = started + args.timeout;

    let mut filter = ReportFilter::new(Some(&nonce));
    let mut last_report: Option<String> = None;
    let mut status = None;
    let mut exited_at: Option<Instant> = None;
    let (mut timed_out, mut interrupted) = (false, false);
    loop {
        if status.is_none()
            && let Ok(Some(s)) = child.try_wait()
        {
            status = Some(s);
            exited_at = Some(Instant::now());
        }
        let now = Instant::now();
        if tree::interrupted() {
            interrupted = true;
            break;
        }
        if now >= deadline {
            timed_out = true;
            break;
        }
        if exited_at.is_some_and(|t| now >= t + DRAIN_GRACE) {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Msg::Line(line)) => match filter.feed(&line) {
                Feed::Other(l) => eprintln!("{l}"),
                Feed::Held => {}
                Feed::Restart(lines) => lines.iter().for_each(|l| eprintln!("{l}")),
                Feed::Report(report) => last_report = Some(report),
            },
            Ok(Msg::End) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                // The pipe is closed: wait for the application itself, within the deadline.
                while status.is_none() && Instant::now() < deadline && !tree::interrupted() {
                    match child.try_wait() {
                        Ok(Some(s)) => status = Some(s),
                        Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                        Err(_) => break,
                    }
                }
                timed_out = status.is_none() && !tree::interrupted();
                interrupted = tree::interrupted();
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    for l in filter.finish() {
        eprintln!("{l}");
    }
    if status.is_none() {
        if let Some(t) = &tree {
            t.kill();
        }
        let _ = child.kill();
        status = child.wait().ok();
    }
    drop(tree);

    if interrupted {
        eprintln!("\x1b[31mError:\x1b[0m interrupted: the application was stopped before the route posture report was complete");
        return ExitCode::from(130);
    }
    let code = status.and_then(|s| s.code());
    let timeout = timed_out.then_some(args.timeout);
    let selected = if timeout.is_some() { None } else { last_report.as_deref().map(|r| report_has_unprotected(r, args.json)) };
    let exit = exit_status(selected, code);
    if exit == 2 {
        eprintln!("\x1b[31mError:\x1b[0m {}", no_report_message(selected, code, timeout));
    } else if let Some(report) = &last_report {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(report.as_bytes());
        let _ = out.flush();
    }
    ExitCode::from(exit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    const N: &str = "abc123";

    fn begin() -> String {
        markers(Some(N)).0
    }

    fn end() -> String {
        markers(Some(N)).1
    }

    #[test]
    fn the_markers_have_a_fixed_text() {
        assert_eq!(REPORT_BEGIN, "----- BEGIN CORVIS ROUTE POSTURE REPORT -----");
        assert_eq!(REPORT_END, "----- END CORVIS ROUTE POSTURE REPORT -----");
        assert_eq!(REPORT_ENV, "CORVIS_ROUTE_POSTURE_REPORT");
        assert_eq!(NONCE_ENV, "CORVIS_ROUTE_POSTURE_NONCE");
        assert_eq!(EXIT_CLEAN, 40);
        assert_eq!(markers(Some("n0nce")).0, "----- BEGIN CORVIS ROUTE POSTURE REPORT n0nce -----");
        assert_eq!(markers(Some("n0nce")).1, "----- END CORVIS ROUTE POSTURE REPORT n0nce -----");
        assert_eq!(markers(None), (REPORT_BEGIN.to_string(), REPORT_END.to_string()));
    }

    #[test]
    fn nonces_are_long_hex_and_differ_between_runs() {
        let (a, b) = (new_nonce(), new_nonce());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn a_report_is_separated_from_the_log_lines_around_it() {
        let (b, e) = (begin(), end());
        let lines = ["INFO starting", &b, "GET /  C::h  public", &e, "INFO after"];
        let (report, other) = split_report(&lines, Some(N));
        assert_eq!(report.as_deref(), Some("GET /  C::h  public\n"));
        assert_eq!(other, ["INFO starting", "INFO after"]);
    }

    #[test]
    fn gateway_rows_and_the_trailing_blocks_are_part_of_the_report() {
        let (b, e) = (begin(), end());
        let lines = [
            "INFO log",
            &b,
            "WS  /chat  event \"ping\"  ChatGateway::ping  UNPROTECTED  [no guards]",
            "",
            "Not checked:",
            "  raw upgrade listeners",
            "",
            "access guards counted: none",
            "stale exemption: GET /gone (old) matches no route",
            &e,
        ];
        let (report, other) = split_report(&lines, Some(N));
        let report = report.unwrap();
        assert!(report.starts_with("WS  /chat") && report.contains("\n\nNot checked:\n") && report.ends_with("matches no route\n"));
        assert_eq!(other, ["INFO log"]);
    }

    #[test]
    fn markers_without_the_run_nonce_are_ordinary_lines() {
        let (b, e) = (begin(), end());
        let lines = [REPORT_BEGIN, "(no routes)", REPORT_END, &b, "real", &e];
        let (report, other) = split_report(&lines, Some(N));
        assert_eq!(report.as_deref(), Some("real\n"));
        assert_eq!(other, [REPORT_BEGIN, "(no routes)", REPORT_END]);
    }

    #[test]
    fn a_report_without_end_is_not_a_report() {
        let b = begin();
        let (report, other) = split_report(&["a", &b, "x"], Some(N));
        assert_eq!(report, None);
        assert_eq!(other, ["a", &b, "x"]);
    }

    #[test]
    fn an_end_without_begin_is_just_a_line() {
        let e = end();
        let (report, other) = split_report(&["a", &e, "b"], Some(N));
        assert_eq!(report, None);
        assert_eq!(other, ["a", &e, "b"]);
    }

    #[test]
    fn the_last_of_two_reports_wins() {
        let (b, e) = (begin(), end());
        let lines = [&b, "one", &e, &b, "two", &e];
        let (report, other) = split_report(&lines.each_ref().map(|l| &l[..]), Some(N));
        assert_eq!(report.as_deref(), Some("two\n"));
        assert!(other.is_empty());
    }

    #[test]
    fn a_new_begin_inside_a_report_restarts_it() {
        let (b, e) = (begin(), end());
        let lines = [&b, "fake", &b, "real", &e];
        let (report, other) = split_report(&lines.each_ref().map(|l| &l[..]), Some(N));
        assert_eq!(report.as_deref(), Some("real\n"));
        assert_eq!(other, [b.as_str(), "fake"]);
    }

    #[test]
    fn windows_line_endings_are_accepted() {
        let (b, e) = (format!("{}\r\n", begin()), format!("{}\r\n", end()));
        let lines = ["log\r\n", &b, "row\r\n", &e];
        let (report, other) = split_report(&lines, Some(N));
        assert_eq!(report.as_deref(), Some("row\n"));
        assert_eq!(other, ["log"]);
    }

    #[test]
    fn an_empty_report_is_still_a_report() {
        let (b, e) = (begin(), end());
        let (report, _) = split_report(&[&b, &e], Some(N));
        assert_eq!(report.as_deref(), Some(""));
    }

    #[test]
    fn unprotected_entries_are_read_from_the_report() {
        let text = "GET  /a  C::a  public  [no guards]\nGET  /b  C::b  UNPROTECTED  [no guards]\n";
        assert!(report_has_unprotected(text, false));
        assert!(!report_has_unprotected("GET  /a  C::a  public  [no guards]\n\naccess guards counted: none\n", false));
        let json = "{\n  \"entries\": [\n    {\n      \"status\": {\n        \"state\": \"unprotected\"\n      }\n    }\n  ]\n}\n";
        assert!(report_has_unprotected(json, true));
        assert!(!report_has_unprotected(&json.replace("unprotected", "public"), true));
        // A route named like the marker text cannot hide an entry, and a string cannot fake one in JSON.
        assert!(!report_has_unprotected("{\"path\": \"\\\"state\\\": \\\"unprotected\\\"\"}", true));
    }

    #[test]
    fn arguments_default_and_flags() {
        let a = parse_args(&[]).unwrap();
        assert_eq!(a, PostureArgs { json: false, release: false, timeout: Duration::from_secs(120), program: vec![] });
        let a = parse_args(&s(&["--json", "--release", "--timeout", "5", "--", "--port", "1"])).unwrap();
        assert!(a.json && a.release);
        assert_eq!(a.timeout, Duration::from_secs(5));
        assert_eq!(a.program, ["--port", "1"]);
        assert_eq!(parse_args(&s(&["--timeout=7"])).unwrap().timeout, Duration::from_secs(7));
        assert!(parse_args(&s(&["--prod"])).unwrap().release);
    }

    #[test]
    fn flags_after_the_separator_belong_to_the_application() {
        let a = parse_args(&s(&["--", "--json", "--timeout", "1"])).unwrap();
        assert!(!a.json);
        assert_eq!(a.timeout, Duration::from_secs(120));
        assert_eq!(a.program, ["--json", "--timeout", "1"]);
    }

    #[test]
    fn bad_arguments_are_rejected() {
        assert!(parse_args(&s(&["--watch"])).unwrap_err().contains("--watch"));
        assert!(parse_args(&s(&["--timeout"])).unwrap_err().contains("--timeout"));
        for bad in ["0", "-1", "abc", "1.5", ""] {
            assert!(parse_args(&s(&["--timeout", bad])).unwrap_err().contains("greater than 0"), "{bad}");
        }
        assert!(parse_args(&s(&["--bogus"])).unwrap_err().contains("--bogus"));
    }

    #[test]
    fn an_unprotected_report_is_one_whatever_the_exit_code() {
        for code in [Some(0), Some(1), Some(40), Some(41), Some(101), None] {
            assert_eq!(exit_status(Some(true), code), 1, "{code:?}");
        }
    }

    #[test]
    fn a_clean_report_needs_the_clean_exit_code() {
        assert_eq!(exit_status(Some(false), Some(40)), 0);
        for code in [Some(0), Some(1), Some(41), Some(101), None] {
            assert_eq!(exit_status(Some(false), code), 2, "{code:?}");
        }
    }

    #[test]
    fn without_a_report_the_status_is_two() {
        for code in [Some(0), Some(1), Some(40), Some(41), Some(101), None] {
            assert_eq!(exit_status(None, code), 2, "{code:?}");
        }
    }

    #[test]
    fn the_no_report_message_has_the_exit_code_or_the_timeout() {
        let m = no_report_message(None, Some(101), None);
        assert!(m.contains("exit code 101") && m.contains("without producing a route posture report"), "{m}");
        let m = no_report_message(None, None, Some(Duration::from_secs(30)));
        assert!(m.contains("30 seconds") && m.contains("--timeout"), "{m}");
        let m = no_report_message(Some(false), Some(0), None);
        assert!(m.contains("instead of 40") && m.contains("not trusted"), "{m}");
    }
}
