//! The accept step of the server. An error that concerns one connection (it died between the kernel's queue and
//! `accept`) is retried at once; any other error (out of file descriptors, out of memory) would fail again at once, so
//! the server waits before the next attempt, 5 ms after the first error and doubling up to 1 s, until a connection is
//! accepted. Those errors are logged at most once a minute, with how many there were since the last line.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// First wait after an error that is not about one connection.
const FIRST_WAIT: Duration = Duration::from_millis(5);
/// Longest wait between two attempts.
const MAX_WAIT: Duration = Duration::from_secs(1);
/// Shortest time between two log lines.
const LOG_EVERY: Duration = Duration::from_secs(60);

/// Where connections come from: a TCP listener, or a scripted one in the tests.
pub(super) trait Acceptor {
    type Conn;
    fn poll_accept(&self, cx: &mut Context<'_>) -> Poll<io::Result<(Self::Conn, SocketAddr)>>;
}

impl Acceptor for TcpListener {
    type Conn = TcpStream;
    fn poll_accept(&self, cx: &mut Context<'_>) -> Poll<io::Result<(TcpStream, SocketAddr)>> {
        TcpListener::poll_accept(self, cx)
    }
}

/// An error about the connection being accepted only, not about the listener or the process: retried at once (as
/// hyper's former `AddrIncoming` did).
fn is_connection_error(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionRefused | io::ErrorKind::Interrupted
    )
}

/// The backoff and the log of the accept errors of one listener.
pub(super) struct AcceptErrors<L> {
    /// Wait before the next attempt; `None` after a success or an error of one connection.
    wait: Option<Duration>,
    /// Errors since the last log line.
    unlogged: u64,
    last_line: Option<Instant>,
    log: L,
}

impl<L: FnMut(String)> AcceptErrors<L> {
    /// `log` receives each warning line.
    pub(super) fn new(log: L) -> Self {
        AcceptErrors { wait: None, unlogged: 0, last_line: None, log }
    }

    fn accepted(&mut self) {
        self.wait = None;
    }

    fn failed(&mut self, error: &io::Error) {
        if is_connection_error(error) {
            return;
        }
        self.wait = Some(self.wait.map_or(FIRST_WAIT, |w| (w * 2).min(MAX_WAIT)));
        self.unlogged += 1;
        let now = Instant::now();
        if self.last_line.is_none_or(|t| now >= t + LOG_EVERY) {
            let line = match std::mem::take(&mut self.unlogged) {
                1 => format!("accept failed: {error}; new connections wait until it succeeds again"),
                n => format!("accept failed: {error}; {n} errors since the last line"),
            };
            (self.log)(line);
            self.last_line = Some(now);
        }
    }

    /// The next connection, with its `max_connections` permit when `permits` is set; `None` once `shutdown` resolved.
    /// No permit is held while the server waits after an error.
    pub(super) async fn accept<A, S>(
        &mut self,
        listener: &A,
        permits: Option<&Arc<Semaphore>>,
        mut shutdown: Pin<&mut S>,
    ) -> Option<(A::Conn, SocketAddr, Option<OwnedSemaphorePermit>)>
    where
        A: Acceptor,
        S: Future<Output = ()>,
    {
        loop {
            if let Some(wait) = self.wait {
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = shutdown.as_mut() => return None,
                }
            }
            let permit = match permits {
                Some(p) => tokio::select! {
                    permit = p.clone().acquire_owned() => permit.ok(),
                    _ = shutdown.as_mut() => return None,
                },
                None => None,
            };
            let accepted = tokio::select! {
                accepted = std::future::poll_fn(|cx| listener.poll_accept(cx)) => accepted,
                _ = shutdown.as_mut() => return None,
            };
            match accepted {
                Ok((conn, remote)) => {
                    self.accepted();
                    return Some((conn, remote, permit));
                }
                Err(e) => self.failed(&e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// An acceptor that plays a script of results (`None` is a connection) and records when each attempt was made.
    /// Past the script it never accepts.
    struct Script {
        results: Mutex<VecDeque<Option<io::ErrorKind>>>,
        attempts: Mutex<Vec<Instant>>,
    }

    impl Script {
        fn new(results: impl IntoIterator<Item = Option<io::ErrorKind>>) -> Self {
            Script { results: Mutex::new(results.into_iter().collect()), attempts: Mutex::default() }
        }
        /// Milliseconds between consecutive attempts.
        fn gaps(&self) -> Vec<u128> {
            self.attempts.lock().unwrap().windows(2).map(|w| (w[1] - w[0]).as_millis()).collect()
        }
    }

    impl Acceptor for Script {
        type Conn = ();
        fn poll_accept(&self, _cx: &mut Context<'_>) -> Poll<io::Result<((), SocketAddr)>> {
            let Some(next) = self.results.lock().unwrap().pop_front() else { return Poll::Pending };
            self.attempts.lock().unwrap().push(Instant::now());
            Poll::Ready(match next {
                None => Ok(((), SocketAddr::from(([127, 0, 0, 1], 1)))),
                Some(io::ErrorKind::Other) => Err(io::Error::other("Too many open files (os error 24)")),
                Some(kind) => Err(io::Error::from(kind)),
            })
        }
    }

    const EMFILE: Option<io::ErrorKind> = Some(io::ErrorKind::Other);

    fn sink() -> (Arc<Mutex<Vec<String>>>, impl FnMut(String)) {
        let lines: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = lines.clone();
        (lines, move |line| sink.lock().unwrap().push(line))
    }

    #[tokio::test(start_paused = true)]
    async fn errors_out_of_descriptors_back_off_up_to_a_second_and_are_logged_once_per_burst() {
        // 20 failures, a connection, one more failure and a connection.
        let script = Script::new((0..20).map(|_| EMFILE).chain([None, EMFILE, None]));
        let (lines, log) = sink();
        let mut errors = AcceptErrors::new(log);
        let never = std::future::pending::<()>();
        tokio::pin!(never);
        assert!(errors.accept(&script, None, never.as_mut()).await.is_some());
        assert!(errors.accept(&script, None, never.as_mut()).await.is_some());
        let mut want: Vec<u128> = vec![5, 10, 20, 40, 80, 160, 320, 640];
        want.extend([1000; 12]);
        // After the connection the next error waits 5 ms again.
        want.extend([0, 5]);
        assert_eq!(script.gaps(), want);
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("Too many open files"), "{lines:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_line_a_minute_carries_the_errors_since_the_last_one() {
        let script = Script::new((0..80).map(|_| EMFILE).chain([None]));
        let (lines, log) = sink();
        let mut errors = AcceptErrors::new(log);
        let never = std::future::pending::<()>();
        tokio::pin!(never);
        assert!(errors.accept(&script, None, never.as_mut()).await.is_some());
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 2, "{lines:?}");
        // The first line at the first error. Error 9 comes at 1,275 ms (the doubling), then one a second: error 68, at
        // 60,275 ms, is the first a minute after the line, and it counts errors 2 to 68.
        assert!(lines[0].starts_with("accept failed: Too many open files"), "{lines:?}");
        assert!(lines[1].ends_with("; 67 errors since the last line"), "{lines:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn an_error_of_one_connection_is_retried_at_once_and_not_logged() {
        let kinds = [
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::Interrupted,
        ];
        let script = Script::new(kinds.into_iter().map(Some).chain([None]));
        let (lines, log) = sink();
        let mut errors = AcceptErrors::new(log);
        let never = std::future::pending::<()>();
        tokio::pin!(never);
        assert!(errors.accept(&script, None, never.as_mut()).await.is_some());
        assert_eq!(script.gaps(), vec![0, 0, 0, 0]);
        assert!(lines.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_shutdown_during_the_wait_returns_at_once_and_no_permit_is_held_meanwhile() {
        let script = Script::new([EMFILE, EMFILE]);
        let (_lines, log) = sink();
        let mut errors = AcceptErrors::new(log);
        let permits = Arc::new(Semaphore::new(1));
        let start = Instant::now();
        // The second attempt fails at 5 ms; the server then waits 10 ms, and the signal comes 3 ms into that wait.
        let shutdown = tokio::time::sleep(Duration::from_millis(8));
        tokio::pin!(shutdown);
        let accepting = errors.accept(&script, Some(&permits), shutdown.as_mut());
        tokio::pin!(accepting);
        tokio::select! {
            r = accepting.as_mut() => panic!("returned early: {:?}", r.map(|(_, a, _)| a)),
            _ = tokio::time::sleep(Duration::from_millis(7)) => {}
        }
        assert_eq!(permits.available_permits(), 1, "the wait holds no permit");
        assert!(accepting.await.is_none());
        assert_eq!(Instant::now() - start, Duration::from_millis(8));
        assert_eq!(script.gaps(), vec![5]);
    }
}
