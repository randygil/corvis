//! Hooks for the tests of this crate and of the layers built on it. **Not part of the API**: hidden, unstable, and any
//! release may change or remove them.

use std::any::Any;
use std::sync::Arc;

use tokio::sync::Semaphore;

use super::IoOptions;
use super::adapter::{Adapter, InMemoryAdapter, MembershipSize};
use super::namespace::IoNamespace;
use super::socket::IoSocket;

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

/// The adapter of `namespace`.
pub fn adapter(namespace: &IoNamespace) -> &Arc<dyn Adapter> {
    namespace.adapter()
}

/// The adapter of `namespace` when it is the in-memory one.
pub fn in_memory(namespace: &IoNamespace) -> Option<&InMemoryAdapter> {
    let adapter: &dyn Any = namespace.adapter().as_ref();
    adapter.downcast_ref::<InMemoryAdapter>()
}

const NOT_IN_MEMORY: &str = "the namespace's adapter is not the in-memory one";

/// What the in-memory adapter of `namespace` holds; an error for another adapter, or when its two maps disagree.
pub fn membership_size(namespace: &IoNamespace) -> Result<MembershipSize, String> {
    in_memory(namespace).ok_or(NOT_IN_MEMORY)?.size()
}

/// The invariants of the in-memory adapter of `namespace` (its maps mirror each other, no empty room or socket).
pub fn check_invariants(namespace: &IoNamespace) -> Result<(), String> {
    in_memory(namespace).ok_or(NOT_IN_MEMORY)?.check_invariants()
}

/// Whether the in-memory adapter of `namespace` holds anything for the socket `id`.
pub fn has_socket(namespace: &IoNamespace, id: &str) -> Result<bool, String> {
    Ok(in_memory(namespace).ok_or(NOT_IN_MEMORY)?.has_socket(id))
}

/// What an in-memory adapter (one a test adapter wraps, for instance) holds; an error when its two maps disagree.
pub fn adapter_size(adapter: &InMemoryAdapter) -> Result<MembershipSize, String> {
    adapter.size()
}

/// The invariants of an in-memory adapter.
pub fn adapter_invariants(adapter: &InMemoryAdapter) -> Result<(), String> {
    adapter.check_invariants()
}

/// Whether an in-memory adapter holds anything for the socket `id`.
pub fn adapter_has_socket(adapter: &InMemoryAdapter, id: &str) -> bool {
    adapter.has_socket(id)
}

/// The bytes queued for the socket's connection and not yet written.
pub fn buffered_amount(socket: &IoSocket) -> usize {
    socket.engine().map_or(0, |e| e.buffered_amount())
}

/// The bytes the socket's Engine.IO session holds against its unread budget (received and not yet released): a test
/// that needs a message to have been read by the server waits for it here.
pub fn unread_bytes(socket: &IoSocket) -> usize {
    socket.engine().map_or(0, |e| e.unread_bytes())
}

/// Whether a polling `GET` of the session is waiting for something to answer.
pub fn poll_pending(socket: &super::engine::EngineSocket) -> bool {
    socket.poll_pending()
}

/// Whether a polling `POST` of the session is being read.
pub fn post_active(socket: &super::engine::EngineSocket) -> bool {
    socket.post_active()
}

/// Whether two sockets share one handshake request (the one their Engine.IO session keeps).
pub fn same_request(a: &IoSocket, b: &IoSocket) -> bool {
    std::ptr::eq(a.handshake().request(), b.handshake().request())
}

/// What decoding one packet cost, for the tests that measure the real heap against the model.
pub struct DecodeProbe {
    /// What the connection reserves before it decodes the packet.
    pub reservation: usize,
    /// What the decoded packet is charged once decoded.
    pub cost: usize,
    _keep: Box<dyn std::any::Any + Send>,
}

/// Decodes one text packet the way a connection does (measure, then decode) with the given `max_decoded_bytes`, and
/// keeps the result alive in the probe. For the allocator-based tests of the cost model.
pub fn probe_decode(text: &str, max_decoded_bytes: usize) -> Result<DecodeProbe, String> {
    let options = IoOptions::new().max_decoded_bytes(max_decoded_bytes);
    let mut decoder = super::parser::Decoder::new(super::parser::Limits::from_options(&options));
    let prepared = decoder.prepare(text.to_owned()).map_err(|e| e.to_string())?;
    let reservation = prepared.reservation();
    let packet = decoder.decode(prepared).map_err(|e| e.to_string())?;
    let cost = packet.as_ref().map_or_else(|| decoder.pending_cost(), super::parser::Packet::cost);
    // What is kept is the packet, or the decoder holding the header of a binary packet: the model charges both.
    let keep: Box<dyn std::any::Any + Send> = match packet {
        Some(packet) => Box::new(packet),
        None => Box::new(decoder),
    };
    Ok(DecodeProbe { reservation, cost, _keep: keep })
}

/// Decodes the CONNECT packet `text` (for example `0/chat,{"token":"x"}`) the way a connection does, converts its
/// `auth` into the `serde_json::Value` a socket keeps for its life, and keeps that value alive in the probe. `cost` is
/// what the socket's handshake is charged for it, `reservation` what the decoder reserved. For the allocator-based test of
/// the charge of `auth`.
pub fn probe_auth(text: &str, max_decoded_bytes: usize) -> Result<DecodeProbe, String> {
    let options = IoOptions::new().max_decoded_bytes(max_decoded_bytes);
    let mut decoder = super::parser::Decoder::new(super::parser::Limits::from_options(&options));
    let prepared = decoder.prepare(text.to_owned()).map_err(|e| e.to_string())?;
    let reservation = prepared.reservation();
    let packet = decoder.decode(prepared).map_err(|e| e.to_string())?.ok_or("not a CONNECT packet")?;
    let super::parser::Body::Connect(Some(auth)) = packet.body else { return Err("a CONNECT packet with auth is expected".to_string()) };
    let stored = auth.to_json().map_err(|e| e.to_string())?;
    drop(auth);
    Ok(DecodeProbe { reservation, cost: super::value::json_cost(&stored), _keep: Box::new(stored) })
}

/// From now on every socket that connects to `namespace` stops at `step` until [`ConnectPause::release`] lets it go.
/// Once per namespace: `None` when a pause is already installed.
pub fn pause_connect(namespace: &IoNamespace, step: ConnectStep) -> Option<Arc<ConnectPause>> {
    let pause = Arc::new(ConnectPause { step, reached: Semaphore::new(0), released: Semaphore::new(0), finished: Semaphore::new(0) });
    namespace.inner.connect_pause.set(pause.clone()).ok().map(|()| pause)
}
