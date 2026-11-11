//! The answers of a broadcast with acknowledgements: the sink an adapter reports to, the shared state of one call and
//! the server-wide budget of the answers it holds.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use super::client::Charge;
use super::value::{Args, IoValue};

struct AckState {
    responses: Vec<Args>,
    /// What the held answers occupy in the server-wide answer budget, released when the call ends.
    holds: Vec<AckHold>,
    bytes: usize,
    expected_clients: usize,
    reported: usize,
    servers: Option<usize>,
    too_large: bool,
    finished: bool,
    cleanups: Vec<Box<dyn FnOnce() + Send>>,
}

pub(crate) struct AckShared {
    state: Mutex<AckState>,
    notify: Notify,
    max_bytes: usize,
    budget: Arc<AckBudget>,
}

/// The server-wide room for the answers that calls with acknowledgements hold until they end. It belongs to the
/// calls, not to the clients that answered: a client's answer is charged to its connection only while it is read and
/// handled, and is released at once when the collector takes it (`try_hold` never waits), so a call can never keep
/// another client's unread budget, its address's or the server's, occupied. When there is no room the call ends with
/// `ResponsesTooLarge`.
pub(crate) struct AckBudget {
    state: Mutex<AckBudgetState>,
    cap: usize,
    /// What one client address may hold of `cap` (1/16): one address cannot keep the whole budget full and make every
    /// other call fail.
    address_share: usize,
}

#[derive(Default)]
struct AckBudgetState {
    used: usize,
    /// Per client address, only while it holds something.
    addresses: HashMap<String, usize>,
}

/// The divisor of the budget that gives the share of one client address.
const ACK_ADDRESS_SHARE_DIVISOR: usize = 16;

impl AckBudget {
    pub(crate) fn new(cap: usize) -> Arc<AckBudget> {
        Arc::new(AckBudget { state: Mutex::new(AckBudgetState::default()), cap, address_share: cap / ACK_ADDRESS_SHARE_DIVISOR })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AckBudgetState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes `bytes` when they fit under the cap and, for an answer that came from a client address, under that
    /// address's share. A short critical section and never a wait; the map holds an address only while it holds bytes.
    fn try_hold(self: &Arc<Self>, bytes: usize, address: Option<String>) -> Option<AckHold> {
        let mut s = self.lock();
        let total = s.used.checked_add(bytes).filter(|total| *total <= self.cap)?;
        if let Some(address) = &address {
            let held = s.addresses.get(address).copied().unwrap_or(0);
            let held = held.checked_add(bytes).filter(|held| *held <= self.address_share)?;
            s.addresses.insert(address.clone(), held);
        }
        s.used = total;
        Some(AckHold { budget: self.clone(), bytes, address })
    }

    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.lock().used
    }

    #[cfg(test)]
    pub(crate) fn address_entries(&self) -> usize {
        self.lock().addresses.len()
    }

    #[cfg(test)]
    pub(crate) fn address_share(&self) -> usize {
        self.address_share
    }
}

pub(crate) struct AckHold {
    budget: Arc<AckBudget>,
    bytes: usize,
    address: Option<String>,
}

impl Drop for AckHold {
    fn drop(&mut self) {
        let mut s = self.budget.lock();
        s.used = s.used.saturating_sub(self.bytes);
        if let Some(address) = self.address.take()
            && let Some(held) = s.addresses.get_mut(&address)
        {
            *held = held.saturating_sub(self.bytes);
            if *held == 0 {
                s.addresses.remove(&address);
            }
        }
    }
}

/// How a broadcast with acknowledgements ended, from the collector's side.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AckProgress {
    Complete,
    TooLarge,
}

impl AckShared {
    pub(crate) fn new(max_bytes: usize, budget: Arc<AckBudget>) -> Arc<AckShared> {
        Arc::new(AckShared {
            state: Mutex::new(AckState {
                responses: Vec::new(),
                holds: Vec::new(),
                bytes: 0,
                expected_clients: 0,
                reported: 0,
                servers: None,
                too_large: false,
                finished: false,
                cleanups: Vec::new(),
            }),
            notify: Notify::new(),
            max_bytes,
            budget,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AckState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn set_servers(&self, servers: usize) {
        self.lock().servers = Some(servers);
        self.notify.notify_one();
    }

    /// Waits until every server reported its clients and every counted client answered, or the cap was passed.
    pub(crate) async fn wait(&self) -> AckProgress {
        loop {
            let notified = self.notify.notified();
            {
                let s = self.lock();
                if s.too_large {
                    return AckProgress::TooLarge;
                }
                if s.servers.is_some_and(|n| s.reported >= n) && s.responses.len() >= s.expected_clients {
                    return AckProgress::Complete;
                }
            }
            notified.await;
        }
    }

    /// Takes what was collected and ends the collection: later `expect` and `respond` calls do nothing, and the
    /// cleanups registered by the adapter run (outside the lock). Idempotent.
    pub(crate) fn finish(&self) -> Vec<Args> {
        let (responses, holds, cleanups) = {
            let mut s = self.lock();
            s.finished = true;
            s.bytes = 0;
            (std::mem::take(&mut s.responses), std::mem::take(&mut s.holds), std::mem::take(&mut s.cleanups))
        };
        drop(holds);
        for cleanup in cleanups {
            cleanup();
        }
        responses
    }
}

/// Where an adapter reports the answers of a broadcast with acknowledgements (Node's two callbacks). Cheap to clone;
/// every clone feeds the same collection. Calls after the broadcast ended do nothing.
#[derive(Clone)]
pub struct AckSink {
    shared: Arc<AckShared>,
}

impl std::fmt::Debug for AckSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AckSink").finish_non_exhaustive()
    }
}

impl AckSink {
    pub(crate) fn new(shared: Arc<AckShared>) -> AckSink {
        AckSink { shared }
    }

    /// One server reports how many clients it sent the packet to (the first callback of `broadcastWithAck`). Call
    /// it once per server, with the number of clients whose answer will be sent to [`respond`](Self::respond).
    pub fn expect(&self, clients: usize) {
        {
            let mut s = self.shared.lock();
            if s.finished {
                return;
            }
            s.expected_clients = s.expected_clients.saturating_add(clients);
            s.reported += 1;
        }
        self.shared.notify.notify_one();
    }

    /// A client's answer. The answers of one broadcast are bounded by `max_broadcast_ack_bytes`: the one that would
    /// pass it is dropped and ends the call with the answers that were collected.
    pub fn respond(&self, args: Args) {
        self.collect(args, None, None);
    }

    /// The answer of a local socket with the room it occupies in that connection's unread budget. The connection's
    /// room is released as soon as the answer is collected (or refused): what the call keeps is charged to the
    /// server-wide answer budget instead.
    /// `address` is the client address key of that connection: the answers one address keeps are bounded by its share
    /// of the answer budget.
    pub(crate) fn respond_charged(&self, args: Args, charge: Charge, address: String) {
        self.collect(args, Some(charge), Some(address));
    }

    fn collect(&self, args: Args, charge: Option<Charge>, address: Option<String>) {
        let cost = args.iter().map(IoValue::cost).fold(std::mem::size_of::<Args>(), usize::saturating_add);
        {
            let mut s = self.shared.lock();
            if s.finished || s.too_large {
                drop(s);
                drop(charge);
                return;
            }
            let hold = if s.bytes.saturating_add(cost) > self.shared.max_bytes { None } else { self.shared.budget.try_hold(cost, address) };
            match hold {
                Some(hold) => {
                    s.bytes += cost;
                    s.responses.push(args);
                    s.holds.push(hold);
                }
                None => s.too_large = true,
            }
        }
        // Released outside the lock: a release wakes the connections that wait for room.
        drop(charge);
        self.shared.notify.notify_one();
    }

    /// Registers what the adapter must undo when the broadcast ends, however it ends (a pending acknowledgement on
    /// every recipient, a subscription). Runs at once when the broadcast already ended.
    pub fn on_finish(&self, cleanup: impl FnOnce() + Send + 'static) {
        {
            let mut s = self.shared.lock();
            if !s.finished {
                s.cleanups.push(Box::new(cleanup));
                return;
            }
        }
        cleanup();
    }
}
