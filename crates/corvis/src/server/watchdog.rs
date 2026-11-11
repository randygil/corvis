//! The send-timeout watchdog of a connection, and the end of a connection.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Notify, OwnedSemaphorePermit};
use tokio::time::Instant;

use super::ConnState;
use super::rapid_reset::FLOOD_GRACE;
use super::timed_io::linger_close;
use super::transport::ServerIo;

pub(super) struct Epoch(Instant);

impl Default for Epoch {
    fn default() -> Self {
        Epoch(Instant::now())
    }
}

/// When a response body started waiting for hyper to take its next frame, to tell a stalled client from an
/// application that has nothing to send. `0` means the body is not waiting for hyper (its last poll was pending,
/// or it ended); otherwise milliseconds since the connection's epoch plus one.
#[derive(Default)]
pub(super) struct BodyClock {
    waiting_since: AtomicU64,
}

/// How a connection ends.
#[derive(Default)]
pub(super) struct EndFlags {
    /// The connection was upgraded (WebSocket, ...): no idle timeouts.
    pub(super) upgraded: AtomicBool,
    /// An upgrade listener destroyed the socket (or the upgrade was refused): nothing to protect.
    pub(super) destroyed: AtomicBool,
    /// The server closes the connection to punish the peer (idle timeouts, refused h2c): no drain.
    pub(super) no_linger: AtomicBool,
    /// The socket and its permit, left here by `TimedIo::drop` for the connection task, which decides
    /// whether to drain before closing (hyper's own timeouts are only visible to that task).
    pub(super) closing: Mutex<Option<(ServerIo, Option<OwnedSemaphorePermit>)>>,
}

/// The clocks of a connection's response bodies, which the send-timeout watchdog reads.
#[derive(Default)]
pub(super) struct Clocks {
    /// Origin of the times the response-body clocks record.
    pub(super) epoch: Epoch,
    /// Clocks of the response bodies of the connection that are alive (send timeout watchdog).
    pub(super) bodies: Mutex<Vec<Arc<BodyClock>>>,
    /// When hyper last took a frame from any response body of the connection (milliseconds since `epoch` plus
    /// one; `0` = never): the connection's progress.
    progress: AtomicU64,
    /// Signalled when a clock is registered, so a watchdog that had nothing to watch looks again.
    pub(super) changed: Notify,
}

impl Clocks {
    pub(super) fn note_progress(&self) {
        let ms = Instant::now().saturating_duration_since(self.epoch.0).as_millis() as u64;
        self.progress.store(ms + 1, Ordering::Relaxed);
    }
    fn last_progress(&self) -> Option<Instant> {
        match self.progress.load(Ordering::Relaxed) {
            0 => None,
            v => Some(self.epoch.0 + Duration::from_millis(v - 1)),
        }
    }
}

impl BodyClock {
    pub(super) fn wait_for_hyper(&self, epoch: &Epoch) {
        let ms = Instant::now().saturating_duration_since(epoch.0).as_millis() as u64;
        self.waiting_since.store(ms + 1, Ordering::Relaxed);
    }
    pub(super) fn wait_for_application(&self) {
        self.waiting_since.store(0, Ordering::Relaxed);
    }
    pub(super) fn since(&self, epoch: &Epoch) -> Option<Instant> {
        match self.waiting_since.load(Ordering::Relaxed) {
            0 => None,
            v => Some(epoch.0 + Duration::from_millis(v - 1)),
        }
    }
}

/// `true` when hyper ended the connection because of its own header read timeout.
fn is_hyper_timeout(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur = Some(err);
    while let Some(e) = cur {
        if e.downcast_ref::<hyper::Error>().is_some_and(|h| h.is_timeout()) {
            return true;
        }
        cur = e.source();
    }
    false
}

/// Resolves when the connection has stalled: some response body waits for hyper to take its next frame and no body
/// of the connection has had a frame taken for `timeout` (the client does not make room: a socket that does not
/// read, HTTP/2 windows that stay closed). A stalled stream next to streams that move does not count, and bodies
/// that wait for the application never do. Pending forever without a timeout, with one too long to add to an
/// `Instant`, and on upgraded connections.
async fn watchdog(state: &ConnState, timeout: Option<Duration>) {
    let Some(timeout) = timeout.filter(|t| Instant::now().checked_add(*t).is_some()) else { return std::future::pending().await };
    loop {
        let notified = state.clocks.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let now = Instant::now();
        let (mut any, mut earliest) = (false, None::<Instant>);
        for clock in state.clocks.bodies.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            any = true;
            if let Some(t) = clock.since(&state.clocks.epoch) {
                earliest = Some(earliest.map_or(t, |e| e.min(t)));
            }
        }
        if state.end.upgraded.load(Ordering::SeqCst) {
            return std::future::pending().await;
        }
        // The stall counts from the later of the oldest wait and the last frame any body gave: a body that starts
        // waiting after a long quiet period (idle SSE) gets a full period of its own.
        let stalled_since = earliest.map(|t| state.clocks.last_progress().map_or(t, |p| p.max(t)));
        if let Some(t) = stalled_since
            && t + timeout <= now
        {
            state.end.no_linger.store(true, Ordering::SeqCst);
            return;
        }
        if any {
            // Progress and new waits only move the deadline later, so waking at the current one is never late.
            let wake = stalled_since.unwrap_or(now) + timeout;
            tokio::select! {
                _ = tokio::time::sleep_until(wake) => {}
                _ = notified.as_mut() => {}
            }
        } else {
            notified.await;
        }
    }
}

/// Runs a connection to its end, then closes the socket: gracefully (see [`linger_close`]) unless the
/// server is closing it to punish the peer (an idle timeout of this server, hyper's header read
/// timeout, refused h2c), the connection was upgraded, or an upgrade listener destroyed the socket.
pub(super) async fn finish_connection<C>(
    state: Arc<ConnState>,
    conn: C,
    mut stop: tokio::sync::watch::Receiver<()>,
    graceful_shutdown: fn(Pin<&mut C>),
    send_timeout: Option<Duration>,
) where
    C: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
{
    let mut conn = Box::pin(conn);
    let mut stopping = false;
    let mut flood = false;
    let mut flood_deadline: Option<Pin<Box<tokio::time::Sleep>>> = None;
    let mut watch = Box::pin(watchdog(&state, send_timeout));
    let result = loop {
        tokio::select! {
            r = conn.as_mut() => break r,
            // A response the client does not let move: drop the connection without a drain.
            _ = watch.as_mut() => break Ok(()),
            // The server is shutting down: finish the streams in flight and stop.
            _ = stop.changed(), if !stopping => {
                stopping = true;
                graceful_shutdown(conn.as_mut());
            }
            // Rapid reset: a graceful GOAWAY refuses new streams (hyper cannot send ENHANCE_YOUR_CALM), and the
            // connection is dropped after a short grace period whatever the streams are doing.
            _ = state.cancels.reset_flood.notified(), if !flood => {
                flood = true;
                state.end.no_linger.store(true, Ordering::SeqCst);
                if !stopping {
                    graceful_shutdown(conn.as_mut());
                }
                flood_deadline = Some(Box::pin(tokio::time::sleep(FLOOD_GRACE)));
            }
            _ = async { flood_deadline.as_mut().unwrap().await }, if flood_deadline.is_some() => break Ok(()),
        }
    };
    // Dropping the connection hands its socket to `state.closing` (see `TimedIo::drop`); the receiver goes with it
    // so the server's shutdown does not wait for the drain below.
    drop(watch);
    drop(conn);
    drop(stop);
    let punish = state.end.no_linger.load(Ordering::SeqCst)
        || state.end.destroyed.load(Ordering::SeqCst)
        || state.end.upgraded.load(Ordering::SeqCst)
        || result.as_ref().err().is_some_and(|e| is_hyper_timeout(e.as_ref()));
    let closing = state.end.closing.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((io, permit)) = closing {
        if !punish {
            linger_close(io).await;
        }
        drop(permit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::task::Poll;

    use bytes::Bytes;
    use http_body_util::BodyExt;
    use hyper::body::Frame;

    use crate::server::guarded_body::tests::{Script, body_of, many_frames, poll_once};
    use crate::server::guarded_body::{GuardedBody, InflightGuard};

    /// One stalled body next to one that keeps moving is not a stalled connection; when neither moves it is.
    #[tokio::test(start_paused = true)]
    async fn the_watchdog_closes_the_connection_only_when_no_body_moves() {
        let state = Arc::new(ConnState::default());
        let make = |parts| {
            GuardedBody::new(
                Script { parts: std::collections::VecDeque::from(parts), pending_first: false }.boxed_unsync(),
                InflightGuard::new(state.clone()),
                false,
            )
        };
        let (mut stalled, mut moving) = (make(many_frames(3)), make(many_frames(50)));
        let mut watch = Box::pin(watchdog(&state, Some(Duration::from_secs(1))));
        // Both took their first frame; hyper takes nothing more from `stalled`.
        assert!(matches!(poll_once(&mut stalled), Poll::Ready(Some(_))));
        assert!(matches!(poll_once(&mut moving), Poll::Ready(Some(_))));
        for _ in 0..8 {
            tokio::time::advance(Duration::from_millis(400)).await;
            assert!(matches!(poll_once(&mut moving), Poll::Ready(Some(_))));
            assert!(futures::poll!(watch.as_mut()).is_pending(), "a body moved: the connection is not stalled");
        }
        tokio::time::advance(Duration::from_millis(1100)).await;
        assert!(futures::poll!(watch.as_mut()).is_ready(), "nothing moved for a send timeout");
        assert!(state.end.no_linger.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn the_watchdog_ignores_bodies_that_wait_for_the_application_and_huge_timeouts() {
        let state = Arc::new(ConnState::default());
        let (mut idle, idle_state) = body_of(vec![Some(Ok(Frame::data(Bytes::from_static(b"a"))))], true, false);
        let _ = &state;
        assert!(poll_once(&mut idle).is_pending());
        let mut watch = Box::pin(watchdog(&idle_state, Some(Duration::from_secs(1))));
        tokio::time::advance(Duration::from_secs(30)).await;
        assert!(futures::poll!(watch.as_mut()).is_pending(), "waiting for the application is not a stall");
        let mut forever = Box::pin(watchdog(&state, Some(Duration::MAX)));
        assert!(futures::poll!(forever.as_mut()).is_pending());
    }
}
