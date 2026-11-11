//! Hooks for the tests of this crate and of the layers built on it. **Not part of the API**: hidden, unstable, and any
//! release may change or remove them.

use std::sync::Arc;

use tokio::sync::Semaphore;

use super::namespace::IoNamespace;

/// A point of the connect sequence of a socket (after its middleware passed), where a test can hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectStep {
    /// The connection gave the socket its slot and the socket is joining.
    Slotted,
    /// The socket is in the namespace's table.
    Registered,
    /// The socket joined the room of its own id.
    JoinedOwnRoom,
    /// CONNECT was queued and the socket is connected; its connection handlers have not run.
    Announced,
}

/// Holds the connect sequence of the next sockets of a namespace at one step (see [`pause_connect`]).
#[derive(Debug)]
pub struct ConnectPause {
    step: ConnectStep,
    reached: Semaphore,
    released: Semaphore,
    finished: Semaphore,
}

/// One connect attempt of a namespace with a pause: reports to [`ConnectPause::finished`] when the attempt ends, however
/// it ends.
pub(crate) struct Attempt(Arc<ConnectPause>);

impl Drop for Attempt {
    fn drop(&mut self) {
        self.0.finished.add_permits(1);
    }
}

impl ConnectPause {
    /// Waits until a socket reached the step and is held there.
    pub async fn reached(&self) {
        if let Ok(permit) = self.reached.acquire().await {
            permit.forget();
        }
    }

    /// Waits until a connect attempt ended (refused, torn down, or connected with its handlers run).
    pub async fn finished(&self) {
        if let Ok(permit) = self.finished.acquire().await {
            permit.forget();
        }
    }

    pub(crate) fn attempt(self: &Arc<Self>) -> Attempt {
        Attempt(self.clone())
    }

    /// Lets one held socket go on.
    pub fn release(&self) {
        self.released.add_permits(1);
    }

    /// Called by the connect sequence at every step.
    pub(crate) async fn at(&self, step: ConnectStep) {
        if step != self.step {
            return;
        }
        self.reached.add_permits(1);
        if let Ok(permit) = self.released.acquire().await {
            permit.forget();
        }
    }
}

/// What the in-memory adapter of `namespace` holds; `None` for another adapter.
pub fn membership_size(namespace: &IoNamespace) -> Option<super::adapter::MembershipSize> {
    namespace.adapter().membership_size()
}

/// From now on every socket that connects to `namespace` stops at `step` until [`ConnectPause::release`] lets it go.
/// Once per namespace: `None` when a pause is already installed.
pub fn pause_connect(namespace: &IoNamespace, step: ConnectStep) -> Option<Arc<ConnectPause>> {
    let pause = Arc::new(ConnectPause { step, reached: Semaphore::new(0), released: Semaphore::new(0), finished: Semaphore::new(0) });
    namespace.inner.connect_pause.set(pause.clone()).ok().map(|()| pause)
}
