//! `corvis posture`: runs the project in report mode and prints the route posture report.
//!
//! The application prints the report between two marker lines (it shares stdout with its log lines) and ends
//! the process with exit code 0 when nothing is unprotected and 1 otherwise. This module separates the report
//! from the other output, applies a timeout and maps the outcome to the command's exit status.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

/// Opens the report. Equal to `corvis::posture::REPORT_BEGIN`; the CLI does not depend on the framework.
pub const REPORT_BEGIN: &str = "----- BEGIN CORVIS ROUTE POSTURE REPORT -----";
/// Closes the report. Equal to `corvis::posture::REPORT_END`.
pub const REPORT_END: &str = "----- END CORVIS ROUTE POSTURE REPORT -----";
/// The environment variable that turns the framework's report mode on.
pub const REPORT_ENV: &str = "CORVIS_ROUTE_POSTURE_REPORT";
/// Seconds the application gets to print the report once it is running, unless `--timeout` says otherwise.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

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
    /// The END marker was read: the complete report (marker lines excluded).
    Report(String),
}

/// Separates the first complete report from the rest of the output, line by line.
#[derive(Debug, Default)]
pub struct ReportFilter {
    state: State,
    held: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Before,
    Inside,
    Done,
}

impl ReportFilter {
    pub fn feed(&mut self, line: &str) -> Feed {
        let line = line.trim_end_matches(['\r', '\n']);
        match self.state {
            State::Before if line == REPORT_BEGIN => {
                self.state = State::Inside;
                Feed::Held
            }
            State::Inside if line == REPORT_END => {
                self.state = State::Done;
                let mut report = self.held.join("\n");
                if !self.held.is_empty() {
                    report.push('\n');
                }
                self.held.clear();
                Feed::Report(report)
            }
            State::Inside => {
                self.held.push(line.to_string());
                Feed::Held
            }
            _ => Feed::Other(line.to_string()),
        }
    }

    /// Lines held for a report that never ended; they were output of the application after all.
    pub fn finish(&mut self) -> Vec<String> {
        if self.state == State::Inside {
            self.state = State::Done;
            let mut lines = vec![REPORT_BEGIN.to_string()];
            lines.append(&mut self.held);
            return lines;
        }
        Vec::new()
    }
}

/// Splits `lines` into the first complete report and the other lines. A report without an END marker is not
/// a report: its lines come back in the other lines.
pub fn split_report(lines: &[&str]) -> (Option<String>, Vec<String>) {
    let mut filter = ReportFilter::default();
    let (mut report, mut other) = (None, Vec::new());
    for line in lines {
        match filter.feed(line) {
            Feed::Other(l) => other.push(l),
            Feed::Held => {}
            Feed::Report(r) => report = Some(r),
        }
    }
    other.extend(filter.finish());
    (report, other)
}

/// The command's exit status: the application's own code (0 or 1) when it printed a complete report, 2
/// otherwise (no report, or a report followed by any other code).
pub fn exit_status(report_found: bool, child_code: Option<i32>) -> u8 {
    match (report_found, child_code) {
        (true, Some(0)) => 0,
        (true, Some(1)) => 1,
        _ => 2,
    }
}

fn no_report_message(child_code: Option<i32>, timeout: Option<Duration>) -> String {
    if let Some(t) = timeout {
        return format!(
            "the application did not print the route posture report within {} seconds and was stopped: it may never reach init(); \
             use --timeout <seconds> to wait longer",
            t.as_secs()
        );
    }
    let code = child_code.map_or("none, ended by a signal".to_string(), |c| c.to_string());
    format!("the application ended (exit code {code}) without producing a route posture report: check the build and startup output above")
}

/// Runs `corvis posture`.
pub fn run(args: &[String]) -> ExitCode {
    let args = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("\x1b[31mError:\x1b[0m {e}");
            return ExitCode::from(2);
        }
    };

    // Build first: the timeout only counts once the application runs, and a build failure is its own message.
    let mut build = Command::new("cargo");
    build.arg("build");
    if args.release {
        build.arg("--release");
    }
    match build.stdout(Stdio::null()).status() {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("\x1b[31mError:\x1b[0m cargo build failed ({s}), so there is no route posture report");
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("\x1b[31mError:\x1b[0m failed to run cargo: {e}");
            return ExitCode::from(2);
        }
    }

    let mut run = Command::new("cargo");
    run.arg("run");
    if args.release {
        run.arg("--release");
    }
    if !args.program.is_empty() {
        run.arg("--").args(&args.program);
    }
    run.env(REPORT_ENV, if args.json { "json" } else { "text" }).stdin(Stdio::null()).stdout(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        run.process_group(0);
    }
    let mut child = match run.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\x1b[31mError:\x1b[0m failed to run cargo: {e}");
            return ExitCode::from(2);
        }
    };
    let stdout = child.stdout.take().expect("piped stdout");
    let child_id = child.id();

    // The watchdog owns the child while it waits, so the main thread only reads the pipe. It is told to stop
    // through the channel when the application has ended; if the timeout passes first, it kills the tree and
    // the pipe reaches EOF.
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let timed_out = timed_out.clone();
        let timeout = args.timeout;
        std::thread::spawn(move || {
            if stop_rx.recv_timeout(timeout).is_err() {
                timed_out.store(true, Ordering::SeqCst);
                kill_by_id(child_id);
            }
        })
    };

    let mut filter = ReportFilter::default();
    let mut report_found = false;
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&buf).into_owned();
        match filter.feed(&line) {
            Feed::Other(l) => eprintln!("{l}"),
            Feed::Held => {}
            Feed::Report(report) => {
                report_found = true;
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(report.as_bytes());
                let _ = out.flush();
            }
        }
    }
    for l in filter.finish() {
        eprintln!("{l}");
    }
    let status = child.wait();
    let _ = stop_tx.send(());
    let _ = watchdog.join();

    let code = status.ok().and_then(|s| s.code());
    let timeout = timed_out.load(Ordering::SeqCst).then_some(args.timeout);
    let report_found = report_found && timeout.is_none();
    let exit = exit_status(report_found, code);
    if exit == 2 {
        eprintln!("\x1b[31mError:\x1b[0m {}", no_report_message(code, timeout));
    }
    ExitCode::from(exit)
}

/// Ends the process `id` and the processes it started (`cargo run` starts the application as its own child).
fn kill_by_id(id: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill").args(["/PID", &id.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill").args(["-KILL", "--", &format!("-{id}")]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_markers_have_a_fixed_text() {
        assert_eq!(REPORT_BEGIN, "----- BEGIN CORVIS ROUTE POSTURE REPORT -----");
        assert_eq!(REPORT_END, "----- END CORVIS ROUTE POSTURE REPORT -----");
        assert_eq!(REPORT_ENV, "CORVIS_ROUTE_POSTURE_REPORT");
    }

    #[test]
    fn a_report_is_separated_from_the_log_lines_around_it() {
        let lines = ["INFO starting", REPORT_BEGIN, "GET /  C::h  public", REPORT_END, "INFO after"];
        let (report, other) = split_report(&lines);
        assert_eq!(report.as_deref(), Some("GET /  C::h  public\n"));
        assert_eq!(other, ["INFO starting", "INFO after"]);
    }

    #[test]
    fn a_report_without_end_is_not_a_report() {
        let (report, other) = split_report(&["a", REPORT_BEGIN, "x"]);
        assert_eq!(report, None);
        assert_eq!(other, ["a", REPORT_BEGIN, "x"]);
    }

    #[test]
    fn an_end_without_begin_is_just_a_line() {
        let (report, other) = split_report(&["a", REPORT_END, "b"]);
        assert_eq!(report, None);
        assert_eq!(other, ["a", REPORT_END, "b"]);
    }

    #[test]
    fn the_first_of_two_reports_wins() {
        let lines = [REPORT_BEGIN, "one", REPORT_END, REPORT_BEGIN, "two", REPORT_END];
        let (report, other) = split_report(&lines);
        assert_eq!(report.as_deref(), Some("one\n"));
        assert_eq!(other, [REPORT_BEGIN, "two", REPORT_END]);
    }

    #[test]
    fn windows_line_endings_are_accepted() {
        let lines = ["log\r\n", "----- BEGIN CORVIS ROUTE POSTURE REPORT -----\r\n", "row\r\n", "----- END CORVIS ROUTE POSTURE REPORT -----\r\n"];
        let (report, other) = split_report(&lines);
        assert_eq!(report.as_deref(), Some("row\n"));
        assert_eq!(other, ["log"]);
    }

    #[test]
    fn an_empty_report_is_still_a_report() {
        let (report, _) = split_report(&[REPORT_BEGIN, REPORT_END]);
        assert_eq!(report.as_deref(), Some(""));
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
    fn exit_status_forwards_zero_and_one_only_with_a_report() {
        assert_eq!(exit_status(true, Some(0)), 0);
        assert_eq!(exit_status(true, Some(1)), 1);
        assert_eq!(exit_status(true, Some(101)), 2);
        assert_eq!(exit_status(true, None), 2);
        assert_eq!(exit_status(false, Some(0)), 2);
        assert_eq!(exit_status(false, Some(1)), 2);
        assert_eq!(exit_status(false, Some(101)), 2);
    }

    #[test]
    fn the_no_report_message_has_the_exit_code_or_the_timeout() {
        let m = no_report_message(Some(101), None);
        assert!(m.contains("exit code 101") && m.contains("without producing a route posture report"), "{m}");
        let m = no_report_message(None, Some(Duration::from_secs(30)));
        assert!(m.contains("30 seconds") && m.contains("--timeout"), "{m}");
    }
}
