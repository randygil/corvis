//! The `handle_disconnect` hooks of the gateways: one queue for every namespace and Engine.IO path of the application.
//!
//! A socket that leaves a namespace whose gateways have `handle_disconnect` hooks waits here for a worker, and is
//! *pending* until its hooks returned (or exceeded `hook_timeout`). What the queue guarantees:
//!
//! - **A share per client address.** The hooks an address has pending, and the bytes they pin, are counted against it
//!   in the Engine.IO server's lingering table (#33), under the session's address key. An address at its share
//!   ([`ADDRESS_SHARE_DIVISOR`]th of either cap) is answered `503` on new sessions until its hooks drain, so it cannot
//!   add pending hooks except from the sessions it already has open. Within its share a hook is never skipped.
//! - **Fair workers.** At most [`DISCONNECT_WORKERS`] hooks run at once, at most [`DISCONNECT_WORKERS_PER_ADDRESS`] of
//!   them for one address, and a free worker takes the next address in round-robin order: a socket's hook starts after
//!   at most one hook of each other address that had hooks waiting before it, however many each of them queued.
//! - **A global cap.** At most `max_pending_disconnect_hooks` hooks are pending in the application, pinning at most
//!   `max_pending_disconnect_hook_bytes`. The one case in which a hook is dropped is a socket that leaves while either
//!   cap is full; it is logged (rate limited). Since every address is held to its share, filling a cap takes at least
//!   `cap / (share + max_sessions_per_ip x namespaces with disconnect hooks)` addresses at once.
//! - **What a pending socket pins.** The socket holds its connection weakly, so the Engine.IO session and its buffers
//!   go with the connection; the session's handshake is a copy of its own (not a slice of the read buffer). When the
//!   socket is queued, the unread-budget charge of its handshake `auth` is released: the queue's own budget charges it
//!   from then on. [`pending_cost`] is what one pending socket pins, at most: the queue's memory is at most the
//!   pending hooks times that, bounded by `max_pending_disconnect_hook_bytes`.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::super::engine::HookLinger;
use super::super::socket::{IoSocket, LEFT_SOCKET_HEAP};
use super::super::value::{ALLOC_OVERHEAD, json_cost, json_map_cost, text_cost};
use crate::http::Request;
use crate::logger::Logger;
use crate::websockets::{GatewayRuntime, run_hook_with};

/// How many `handle_disconnect` hooks run at once in the application.
pub(crate) const DISCONNECT_WORKERS: usize = 32;
/// How many of them may be one client address's at once.
pub(crate) const DISCONNECT_WORKERS_PER_ADDRESS: usize = 8;
/// The most hooks pending (queued or running) in the application by default.
pub const DEFAULT_MAX_PENDING_DISCONNECT_HOOKS: usize = 10_000;
/// The most the pending hooks' sockets may pin by default: 64 MiB.
pub const DEFAULT_MAX_PENDING_DISCONNECT_HOOK_BYTES: usize = 64 * 1024 * 1024;
/// One address's share of either cap: a 64th.
pub(crate) const ADDRESS_SHARE_DIVISOR: usize = 64;

/// What a header of a request occupies besides its name and value bytes: its slot in the header map's entry vector and
/// its index, with the map's spare capacity (fewer than twice the entries).
const HEADER_SLOT: usize = 256;
/// What the gateway layer attaches to a socket (its gate, in the socket's extensions table) and what the queue adds per
/// pending socket (its entry, with the queue's spare capacity).
const GATEWAY_HEAP: usize = 768;

/// The share of each address in the Engine.IO servers' lingering table: `(hooks, bytes)`.
pub(crate) fn address_share(max_hooks: usize, max_bytes: usize) -> (usize, usize) {
    ((max_hooks / ADDRESS_SHARE_DIVISOR).max(1), (max_bytes / ADDRESS_SHARE_DIVISOR).max(1))
}

/// What a request kept by a socket that left occupies on the heap, at most: its header map (entry slots, names and
/// values in allocations of their own), its URI (a string grown by doubling), and its parsed query.
fn request_cost(request: &Request) -> usize {
    let headers = request.headers().iter().fold(2 * ALLOC_OVERHEAD, |total, (name, value)| {
        total.saturating_add(HEADER_SLOT).saturating_add(text_cost(name.as_str().len())).saturating_add(text_cost(value.len()))
    });
    let uri = request.uri();
    let uri_len = uri.scheme_str().map_or(0, str::len)
        + uri.authority().map_or(0, |a| a.as_str().len())
        + uri.path_and_query().map_or(0, |p| p.as_str().len())
        + 8;
    headers.saturating_add(2 * text_cost(uri_len)).saturating_add(json_map_cost(&request.query))
}

/// What one socket that waits for its `handle_disconnect` hooks pins, at most: the socket itself
/// ([`LEFT_SOCKET_HEAP`]), its id and its address key (twice: the socket's and the address count's), its handshake
/// `auth` and its `data` (`json_cost`), its handshake request, and the gateway layer's gate and queue entry. What the
/// application attached to the socket with `insert` is not counted: its size is the application's.
pub(crate) fn pending_cost(socket: &IoSocket) -> usize {
    LEFT_SOCKET_HEAP
        .saturating_add(GATEWAY_HEAP)
        .saturating_add(text_cost(socket.id().len()))
        .saturating_add(2 * text_cost(socket.address_key().len()))
        .saturating_add(json_cost(socket.handshake().auth()))
        .saturating_add(socket.data_cost())
        .saturating_add(request_cost(socket.handshake().request()))
}

/// The queue of one application: every namespace with `handle_disconnect` hooks pushes here.
pub(crate) struct HookQueue {
    state: Mutex<State>,
    max_hooks: usize,
    max_bytes: usize,
    hook_timeout: Duration,
}

#[derive(Default)]
struct State {
    /// The addresses with hooks pending, each with the sockets that wait and the hooks that run.
    lanes: HashMap<String, Lane>,
    /// The addresses whose lane has sockets waiting, in the order the workers serve them.
    ring: VecDeque<String>,
    /// Hooks pending (waiting or running) in the application, and the bytes their sockets pin.
    hooks: usize,
    bytes: usize,
    /// Workers running.
    workers: usize,
}

#[derive(Default)]
struct Lane {
    waiting: VecDeque<Pending>,
    running: usize,
}

/// A socket whose hooks wait or run, with the gateways whose hooks they are and its count against its address.
struct Pending {
    gateways: Arc<Vec<Arc<GatewayRuntime>>>,
    socket: IoSocket,
    bytes: usize,
    _linger: Option<HookLinger>,
}

impl State {
    fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
        state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The next socket to run, round-robin over the addresses that have sockets waiting and fewer than
    /// [`DISCONNECT_WORKERS_PER_ADDRESS`] hooks running. At most that many addresses can be at that limit, so the walk
    /// is short.
    fn next(&mut self) -> Option<Pending> {
        for _ in 0..self.ring.len() {
            let address = self.ring.pop_front()?;
            let Some(lane) = self.lanes.get_mut(&address) else { continue };
            if lane.running >= DISCONNECT_WORKERS_PER_ADDRESS {
                self.ring.push_back(address);
                continue;
            }
            let Some(next) = lane.waiting.pop_front() else { continue };
            lane.running += 1;
            if !lane.waiting.is_empty() {
                self.ring.push_back(address);
            }
            return Some(next);
        }
        None
    }

    /// A socket's hooks ended.
    fn finished(&mut self, pending: &Pending) {
        self.hooks = self.hooks.saturating_sub(1);
        self.bytes = self.bytes.saturating_sub(pending.bytes);
        let address = pending.socket.address_key();
        if let Some(lane) = self.lanes.get_mut(address) {
            lane.running = lane.running.saturating_sub(1);
            if lane.running == 0 && lane.waiting.is_empty() {
                self.lanes.remove(address);
            }
        }
    }
}

impl HookQueue {
    pub(crate) fn new(max_hooks: usize, max_bytes: usize, hook_timeout: Duration) -> HookQueue {
        HookQueue { state: Mutex::new(State::default()), max_hooks, max_bytes, hook_timeout }
    }

    /// Queues the `handle_disconnect` hooks of a socket that left (called once per socket). Dropped, and logged, only
    /// when the application's caps are full.
    pub(crate) fn push(self: &Arc<Self>, gateways: Arc<Vec<Arc<GatewayRuntime>>>, socket: IoSocket) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
        let bytes = pending_cost(&socket);
        let address = socket.address_key().to_string();
        let mut state = State::lock(&self.state);
        if state.hooks >= self.max_hooks || state.bytes.saturating_add(bytes) > self.max_bytes {
            let (hooks, held) = (state.hooks, state.bytes);
            drop(state);
            socket.warn(
                "a disconnect hook dropped",
                &format!(
                    "{hooks} handle_disconnect hooks are pending in the application, pinning {held} bytes (IoAdapter::max_pending_disconnect_hooks and max_pending_disconnect_hook_bytes): the hook of a socket that left was dropped"
                ),
            );
            return;
        }
        state.hooks += 1;
        state.bytes = state.bytes.saturating_add(bytes);
        // Counted against the address until the hooks end: at its share it opens no session meanwhile.
        let linger = socket.engine().and_then(|engine| engine.linger_hook(bytes));
        // From now on the queue's budget charges the handshake's `auth`, not the connection's.
        socket.handshake().release_charge();
        let lane = state.lanes.entry(address.clone()).or_default();
        let idle = lane.waiting.is_empty();
        let free = lane.running < DISCONNECT_WORKERS_PER_ADDRESS;
        lane.waiting.push_back(Pending { gateways, socket, bytes, _linger: linger });
        if idle {
            state.ring.push_back(address);
        }
        // A worker that finds nothing to run exits under this lock, so a socket queued here is never left without one.
        let start = free && state.workers < DISCONNECT_WORKERS;
        if start {
            state.workers += 1;
        }
        drop(state);
        if start {
            handle.spawn(self.clone().work());
        }
    }

    async fn work(self: Arc<Self>) {
        let mut worker = Worker { queue: self.clone(), counted: true };
        loop {
            let next = {
                let mut state = State::lock(&self.state);
                let next = state.next();
                if next.is_none() {
                    state.workers = state.workers.saturating_sub(1);
                    worker.counted = false;
                }
                next
            };
            let Some(pending) = next else { return };
            let running = Running { queue: &self, pending };
            for gateway in running.pending.gateways.iter() {
                let Some(hook) = &gateway.info.io_on_disconnect else { continue };
                let arg: Arc<dyn Any + Send + Sync> = Arc::new(running.pending.socket.clone());
                if tokio::time::timeout(self.hook_timeout, run_hook_with(&gateway.name, "handle_disconnect", hook, arg)).await.is_err() {
                    Logger::new("WsExceptionsHandler")
                        .error(format!("{}: handle_disconnect took longer than {:?} and was abandoned", gateway.name, self.hook_timeout));
                }
            }
            drop(running);
        }
    }
}

/// A worker's place among the [`DISCONNECT_WORKERS`]: given back when the task ends, however it ends.
struct Worker {
    queue: Arc<HookQueue>,
    counted: bool,
}

impl Drop for Worker {
    fn drop(&mut self) {
        if self.counted {
            let mut state = State::lock(&self.queue.state);
            state.workers = state.workers.saturating_sub(1);
        }
    }
}

/// A socket whose hooks run: its counts are given back when they end, also when the task is dropped meanwhile.
struct Running<'a> {
    queue: &'a HookQueue,
    pending: Pending,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        State::lock(&self.queue.state).finished(&self.pending);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_address_has_a_sixty_fourth_of_either_cap_and_at_least_one() {
        assert_eq!(address_share(DEFAULT_MAX_PENDING_DISCONNECT_HOOKS, DEFAULT_MAX_PENDING_DISCONNECT_HOOK_BYTES), (156, 1024 * 1024));
        assert_eq!(address_share(10, 100), (1, 1));
    }
}
