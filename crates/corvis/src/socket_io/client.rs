//! One Engine.IO connection as the Socket.IO layer sees it: the decoder, the namespaces it joined or is joining, the
//! connect timeout, the handlers in flight and what is held against the unread budget.
//!
//! One task per connection reads its messages. Packets of the connection are handled in arrival order; the handlers of
//! events run concurrently inside that task (so they start in order), at most `max_inflight` at a time. The task takes
//! the next message only while a handler slot is free and while the Engine.IO session is not over its unread limit
//! (its own, or the server-wide one with the fair share: the test the transports use to stop reading). Everything it
//! keeps is charged to the session's unread gauge, which is chained to the server-wide one, until the packet's handler
//! finishes. A packet's decoded cost is reserved before any value is built (`budget.rs`): when the budget has no room
//! the message waits, parked, and the connection takes no other, so no queue of the Socket.IO layer can grow without
//! bound and the decoding itself cannot overshoot.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, Stream, StreamExt};
use tokio::time::Instant;

use super::budget::Reservation;
use super::engine::session::{CloseReason, after};
use super::engine::{EngineMessage, EngineSocket, Held, SendError};
use super::namespace::{ConnectError, IoNamespace};
use super::options::IoOptions;
use super::parser::{self, Body, Decoder, Encoded, Limits, Packet, Prepared, SharedEncoded};
use super::server::ServerInner;
use super::socket::{AckSender, DisconnectReason, EmitError, Handshake, IoSocket, Phase};
use super::testing::ConnectStep;
use super::value::{Args, IoValue, json_cost};
use super::warn::WarnLimiter;

enum Slot {
    /// The CONNECT was accepted and the middleware is running.
    Pending,
    Joined(IoSocket),
    /// The connection joined the namespace and the socket has left it (the server disconnected it, or the client
    /// did): packets the client had in flight for it are dropped instead of closing the connection (the Node server
    /// closes it: a deliberate, more lenient difference, at the cost of a packet for a joined namespace). A
    /// new CONNECT replaces it. There is at most one entry per registered namespace, so the set is bounded by them.
    Left,
}

/// What sockets and the connection task share about one Engine.IO connection.
pub(crate) struct ClientShared {
    pub(crate) engine: EngineSocket,
    options: Arc<IoOptions>,
    /// The namespaces of this connection: at most one entry per registered namespace, pending ones included.
    nsps: Mutex<HashMap<String, Slot>>,
    /// Set by the first `terminate`: no socket joins afterwards.
    done: AtomicBool,
    joined_once: AtomicBool,
    /// Rate limit of the lines a client can provoke.
    pub(crate) warnings: Arc<WarnLimiter>,
}

impl ClientShared {
    pub(crate) fn new(engine: EngineSocket, options: Arc<IoOptions>, server: &ServerInner) -> Arc<ClientShared> {
        Arc::new(ClientShared {
            engine,
            options,
            nsps: Mutex::new(HashMap::new()),
            done: AtomicBool::new(false),
            joined_once: AtomicBool::new(false),
            warnings: server.warnings.clone(),
        })
    }

    fn nsps(&self) -> std::sync::MutexGuard<'_, HashMap<String, Slot>> {
        self.nsps.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queues an encoded packet, moving its text into the queue: its text message and its attachments together, so
    /// nothing is queued between them.
    pub(crate) fn send_encoded(&self, encoded: Encoded) -> Result<(), EmitError> {
        let Encoded { text, attachments } = encoded;
        let text = EngineMessage::Framed(text);
        if attachments.is_empty() {
            return self.engine.send(text).map_err(emit_error);
        }
        let mut messages = Vec::with_capacity(1 + attachments.len());
        messages.push(text);
        messages.extend(attachments.into_iter().map(|a| EngineMessage::Binary(a.into_bytes())));
        self.engine.send_all(messages).map_err(emit_error)
    }

    /// Queues an encoding shared with other recipients (a broadcast): the text is a reference to the one buffer, and
    /// so are the attachments.
    pub(crate) fn send_shared(&self, encoded: &SharedEncoded) -> Result<(), EmitError> {
        let text = EngineMessage::Shared(encoded.text.clone());
        if encoded.attachments.is_empty() {
            return self.engine.send(text).map_err(emit_error);
        }
        let mut messages = Vec::with_capacity(1 + encoded.attachments.len());
        messages.push(text);
        messages.extend(encoded.attachments.iter().cloned().map(EngineMessage::Binary));
        self.engine.send_all(messages).map_err(emit_error)
    }

    fn send_packet(&self, packet: &Packet) -> Result<(), EmitError> {
        let encoded = parser::encode(packet, self.options.max_attachments).map_err(|_| EmitError::TooDeep)?;
        self.send_encoded(encoded)
    }

    fn joined(&self, name: &str) -> Option<IoSocket> {
        match self.nsps().get(name) {
            Some(Slot::Joined(socket)) => Some(socket.clone()),
            _ => None,
        }
    }

    /// Frees the namespace of a socket that left it, so the client may join it again; packets that were already on
    /// their way for it are dropped (`Slot::Left`).
    pub(crate) fn forget(&self, name: &str, socket: &IoSocket) {
        let mut nsps = self.nsps();
        if matches!(nsps.get(name), Some(Slot::Joined(s)) if s == socket) {
            nsps.insert(name.to_string(), Slot::Left);
        }
    }

    /// Frees the slot of a socket that left before it was announced: its client never joined, so its packets for the
    /// namespace are those of a namespace it has not joined, and a new CONNECT is a first one.
    pub(crate) fn forget_unannounced(&self, name: &str, socket: &IoSocket) {
        let mut nsps = self.nsps();
        if matches!(nsps.get(name), Some(Slot::Joined(s)) if s == socket) {
            nsps.remove(name);
        }
    }

    /// Whether the connection joined the namespace and its socket has left it since.
    fn has_left(&self, name: &str) -> bool {
        matches!(self.nsps().get(name), Some(Slot::Left))
    }

    /// Ends the connection for every socket, once: each reports `reason` (with a DISCONNECT packet first when
    /// `send_packets`), and the Engine.IO session is closed when `close_engine`. A socket that finishes its
    /// middleware afterwards is dropped.
    pub(crate) fn terminate(&self, reason: DisconnectReason, send_packets: bool, close_engine: bool) {
        if !self.done.swap(true, Ordering::SeqCst) {
            let sockets: Vec<IoSocket> = {
                let mut nsps = self.nsps();
                nsps.drain()
                    .filter_map(|(_, slot)| match slot {
                        Slot::Joined(socket) => Some(socket),
                        Slot::Pending | Slot::Left => None,
                    })
                    .collect()
            };
            for socket in sockets {
                socket.disconnect_with(reason, send_packets);
            }
        }
        if close_engine {
            self.engine.close();
        }
    }
}

fn emit_error(error: SendError) -> EmitError {
    match error {
        SendError::Closed => EmitError::NotConnected,
        SendError::Full => EmitError::BufferFull,
    }
}

/// Frees the slot of a connection attempt that ends without joining (an error, a close, a dropped task).
struct PendingGuard {
    shared: Arc<ClientShared>,
    name: String,
    armed: bool,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if self.armed {
            let mut nsps = self.shared.nsps();
            if matches!(nsps.get(&self.name), Some(Slot::Pending)) {
                nsps.remove(&self.name);
            }
        }
    }
}

/// Room held in the budget while a packet is being reassembled or handled; released when the packet's handler ends
/// (or the connection task is dropped), and not before: a closed connection keeps its handlers' charges on the
/// server-wide gauge for as long as they run.
pub(crate) struct Charge {
    parts: Vec<Reservation>,
}

impl Charge {
    fn new(first: Reservation) -> Charge {
        Charge { parts: vec![first] }
    }

    fn add(&mut self, part: Reservation) {
        self.parts.push(part);
    }

    /// The bytes held.
    pub(crate) fn bytes(&self) -> usize {
        self.parts.iter().map(Reservation::bytes).fold(0, usize::saturating_add)
    }
}

/// A text message that was measured and waits for room in the budget, with the wire message it came from (which
/// stays charged meanwhile).
struct Parked {
    prepared: Prepared,
    wire: Held,
}

enum Flow {
    Continue,
    /// A protocol violation: close the connection, reporting this reason to the sockets.
    Close(DisconnectReason),
    /// The Engine.IO session ended meanwhile.
    Gone(CloseReason),
}

async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => futures::future::pending::<()>().await,
    }
}

type Messages = Pin<Box<dyn Stream<Item = (EngineMessage, Held)> + Send + 'static>>;

/// The task of one Engine.IO connection: what it reads, decodes, holds and runs.
pub(crate) struct Connection {
    server: Arc<ServerInner>,
    shared: Arc<ClientShared>,
    messages: Messages,
    decoder: Decoder,
    /// The handlers of the connection's events, run inside its task.
    inflight: FuturesUnordered<BoxFuture<'static, ()>>,
    /// Until the first socket joins: a connection that joins nothing by then is closed.
    connect_deadline: Option<Instant>,
    /// A binary packet whose attachments are arriving: when it gives up, and what it holds.
    reassembly: Option<(Instant, Charge)>,
    /// A measured message that waits for room in the budget.
    parked: Option<Parked>,
    /// When the parked message gives up waiting: set when it parks, cleared when it is admitted.
    admission_deadline: Option<Instant>,
    stream_done: bool,
    /// The end of the Engine.IO session, built once for the connection's life.
    closed: Pin<Box<dyn Future<Output = CloseReason> + Send>>,
}

impl Connection {
    pub(crate) fn new(server: Arc<ServerInner>, shared: Arc<ClientShared>, messages: Messages) -> Connection {
        let engine = shared.engine.clone();
        let options = &shared.options;
        Connection {
            decoder: Decoder::new(Limits::from_options(options)),
            connect_deadline: Some(after(Instant::now(), options.connect_timeout)),
            closed: Box::pin(async move { engine.closed().await }),
            server,
            shared,
            messages,
            inflight: FuturesUnordered::new(),
            reassembly: None,
            parked: None,
            admission_deadline: None,
            stream_done: false,
        }
    }

    pub(crate) async fn run(mut self) {
        let engine = self.shared.engine.clone();
        let options = self.shared.options.clone();
        // The last turn found the budget full (a parked message, or reads paused): this turn registers for a release
        // before it looks again, so a release in between is not lost. A connection under its limits never registers.
        let mut blocked = false;
        // Whether the end of the connection is the client's doing.
        let client_dropped = loop {
            let release = blocked.then(|| engine.unread_release());
            if let Some(waiting) = self.parked.take() {
                let flow = self.admit(waiting).await;
                if let Some(end) = self.end_of(flow) {
                    break end;
                }
            }
            if self.parked.is_none() {
                self.admission_deadline = None;
            } else if self.admission_deadline.is_none() {
                self.admission_deadline = Some(after(Instant::now(), options.admission_timeout));
            }
            let reassembling = self.decoder.is_reassembling();
            let over_limit = !reassembling && engine.unread_over_limit();
            blocked = self.parked.is_some() || over_limit;
            if blocked && release.is_none() {
                // Found full without a registration: register, and look again.
                continue;
            }
            let can_read =
                !self.stream_done && self.parked.is_none() && (reassembling || (self.inflight.len() < options.max_inflight && !over_limit));
            let connect_wait = wait_until(self.connect_deadline);
            let admission_wait = wait_until(self.admission_deadline);
            let reassembly_wait = wait_until(self.reassembly.as_ref().map(|(deadline, _)| *deadline));
            tokio::select! {
                biased;
                reason = &mut self.closed => break self.gone(reason),
                Some(()) = self.inflight.next(), if !self.inflight.is_empty() => {}
                _ = connect_wait => {
                    self.connect_deadline = None;
                    if !self.shared.joined_once.load(Ordering::SeqCst) {
                        break self.close(DisconnectReason::ForcedServerClose);
                    }
                }
                _ = reassembly_wait => break self.close(DisconnectReason::ForcedServerClose),
                // A packet waited for room longer than the limit: the wait is bounded and visible.
                _ = admission_wait => break self.close(DisconnectReason::AdmissionTimeout),
                // A release of room somewhere: the parked message, or the reads that are paused, may go on.
                () = crate::ws::released(release), if blocked => {}
                next = self.messages.next(), if can_read => {
                    let Some((message, held)) = next else {
                        self.stream_done = true;
                        continue;
                    };
                    let flow = self.handle_message(message, held).await;
                    if let Some(end) = self.end_of(flow) {
                        break end;
                    }
                }
            }
        };
        self.finish(client_dropped);
    }

    /// What a `Flow` means for the loop: `None` to go on, `Some(client_dropped)` once the connection was ended here.
    fn end_of(&self, flow: Flow) -> Option<bool> {
        match flow {
            Flow::Continue => None,
            Flow::Close(reason) => Some(self.close(reason)),
            Flow::Gone(reason) => Some(self.gone(reason)),
        }
    }

    /// The server closes the connection for a fault of the client: the client's doing.
    fn close(&self, reason: DisconnectReason) -> bool {
        self.shared.terminate(reason, false, true);
        true
    }

    /// The Engine.IO session ended: the client's doing unless the server closed it.
    fn gone(&self, reason: CloseReason) -> bool {
        self.shared.terminate(reason.into(), false, false);
        dropped_by_client(reason)
    }

    /// What happens to the handlers still running once the loop ended.
    fn finish(self, client_dropped: bool) {
        let Connection { shared, mut inflight, .. } = self;
        if inflight.is_empty() {
            return;
        }
        // The client dropped the connection or failed to make progress (a protocol violation, a packet that waited too
        // long for room, a silence the heartbeat ended): nobody is left to answer, and a client that reconnects in a loop
        // would leave a crowd of handlers behind, so they are cancelled (dropped, which releases their charges) unless the
        // application asked for them to finish.
        if client_dropped && !shared.options.finish_handlers_after_client_close {
            return;
        }
        // Otherwise (the server closed the connection, or is shutting down) handlers that already started get a grace
        // period to finish; nothing can wait for them forever, and they count against the address while they run.
        let grace = shared.options.engine.close_timeout;
        let mut lingering = shared.engine.linger(inflight.len());
        tokio::spawn(async move {
            let _ = tokio::time::timeout(grace, async {
                while inflight.next().await.is_some() {
                    if let Some(l) = lingering.as_mut() {
                        l.one_done();
                    }
                }
            })
            .await;
        });
    }

    async fn handle_message(&mut self, message: EngineMessage, held: Held) -> Flow {
        match message {
            EngineMessage::Text(text) => match self.decoder.prepare(text) {
                Err(_) => Flow::Close(DisconnectReason::ForcedClose),
                Ok(prepared) => self.admit(Parked { prepared, wire: held }).await,
            },
            // Only the server sends framed and shared text; a received message is never one.
            EngineMessage::Framed(_) | EngineMessage::Shared(_) => Flow::Close(DisconnectReason::ForcedClose),
            EngineMessage::Binary(data) => match self.decoder.add_binary(data) {
                Err(_) => Flow::Close(DisconnectReason::ForcedClose),
                Ok(None) => {
                    if let Some((_, charge)) = self.reassembly.as_mut() {
                        charge.add(Reservation::from_held(held));
                    }
                    Flow::Continue
                }
                Ok(Some(packet)) => {
                    let Some((_, mut charge)) = self.reassembly.take() else { return Flow::Close(DisconnectReason::ForcedClose) };
                    charge.add(Reservation::from_held(held));
                    self.handle_packet(packet, charge).await
                }
            },
        }
    }

    /// Reserves the decoded cost of a measured message and decodes it; when the budget has no room the message is
    /// parked and the connection takes no other until room is released.
    async fn admit(&mut self, waiting: Parked) -> Flow {
        let Some(mut reservation) = Reservation::try_new(&self.shared.engine, waiting.prepared.reservation(), waiting.wire.bytes()) else {
            self.parked = Some(waiting);
            return Flow::Continue;
        };
        let reserved = reservation.bytes();
        let Parked { prepared, wire } = waiting;
        match self.decoder.decode(prepared) {
            Err(_) => Flow::Close(DisconnectReason::ForcedClose),
            Ok(Some(packet)) => {
                // The text is gone with the decode: what stays is the decoded packet, at its real cost. The cost model
                // is an upper bound and the reservation was made from it; a packet above it would be a defect, and is
                // refused.
                let cost = packet.cost();
                if cost > reserved {
                    return Flow::Close(DisconnectReason::ForcedClose);
                }
                reservation.shrink_to(cost);
                drop(wire);
                self.handle_packet(packet, Charge::new(reservation)).await
            }
            Ok(None) => {
                let cost = self.decoder.pending_cost();
                if cost > reserved {
                    return Flow::Close(DisconnectReason::ForcedClose);
                }
                reservation.shrink_to(cost);
                drop(wire);
                self.reassembly = Some((after(Instant::now(), self.shared.options.attachment_timeout), Charge::new(reservation)));
                Flow::Continue
            }
        }
    }

    async fn handle_packet(&mut self, packet: Packet, charge: Charge) -> Flow {
        let Packet { nsp, body } = packet;
        match body {
            Body::Connect(auth) => match begin_connect(&self.server, &self.shared, nsp, auth) {
                Ok(()) => Flow::Continue,
                Err(ProtocolViolation) => Flow::Close(DisconnectReason::ForcedServerClose),
            },
            // A client does not send these, and a packet for a namespace it has not joined is a violation.
            Body::ConnectError(_) => Flow::Close(DisconnectReason::ForcedServerClose),
            body => {
                let Some(socket) = self.shared.joined(&nsp) else {
                    // A namespace the connection joined and whose socket has left since (the server just disconnected
                    // it): the client's packet was already on its way, and is dropped. The packet was read, measured and
                    // charged like any other, and its attachments consumed, before it got here. A namespace it never
                    // joined, or was refused, is a violation.
                    return if self.shared.has_left(&nsp) { Flow::Continue } else { Flow::Close(DisconnectReason::ForcedServerClose) };
                };
                // Still joining: its CONNECT is not even queued, so no client can have read it and sent this. The same
                // violation as a packet sent while the middleware runs.
                if socket.is_phase(Phase::Joining) {
                    return Flow::Close(DisconnectReason::ForcedServerClose);
                }
                tokio::select! {
                    _ = socket.wait_ready() => {}
                    reason = &mut self.closed => return Flow::Gone(reason),
                }
                if !socket.is_phase(Phase::Connected) {
                    return Flow::Continue;
                }
                match body {
                    Body::Disconnect => socket.disconnect_with(DisconnectReason::ClientNamespaceDisconnect, false),
                    Body::Ack { id, args } => socket.resolve_ack(id, args, charge),
                    Body::Event { id, name, args } => dispatch(&self.shared, &socket, id, &name, args, charge, &mut self.inflight),
                    Body::Connect(_) | Body::ConnectError(_) => unreachable!("handled above"),
                }
                Flow::Continue
            }
        }
    }
}

/// Whether the client ended the connection (the transport closed or failed, a packet could not be parsed, or the client
/// stopped answering pings), as opposed to the server closing it.
fn dropped_by_client(reason: CloseReason) -> bool {
    !matches!(reason, CloseReason::ForcedClose)
}

/// A client broke the protocol: the connection is closed.
struct ProtocolViolation;

/// Starts the handler of an event, if there is one. The handler's future joins `inflight`, so it starts in arrival
/// order, and carries the charge of its packet until it finishes.
fn dispatch(
    shared: &Arc<ClientShared>,
    socket: &IoSocket,
    id: Option<u64>,
    name: &str,
    args: Vec<IoValue>,
    charge: Charge,
    inflight: &mut FuturesUnordered<BoxFuture<'static, ()>>,
) {
    // A handler registered on the socket wins; otherwise the namespace's dispatcher (the gateways') may take the event.
    let own = socket.handler(name);
    let dispatcher = if own.is_none() { socket.namespace().dispatcher() } else { None };
    if own.is_none() && dispatcher.is_none() {
        return;
    }
    let ack = id.map(|id| AckSender::new(socket.clone(), id));
    let namespace = socket.namespace().name().to_string();
    let args = Args::new(args);
    let handler_socket = socket.clone();
    // A socket's handler keeps the packet's charge until it finishes; the dispatcher takes it with the arguments.
    let (created, charge) = match (own, dispatcher) {
        (Some(handler), _) => (std::panic::catch_unwind(AssertUnwindSafe(|| Some(handler(handler_socket, args, ack)))), Some(charge)),
        (None, Some(dispatcher)) => {
            (std::panic::catch_unwind(AssertUnwindSafe(move || dispatcher(handler_socket, name, args, ack, charge))), None)
        }
        (None, None) => return,
    };
    let future = match created {
        Ok(Some(future)) => future,
        Ok(None) => return,
        Err(_) => {
            shared.warnings.panicked("an event handler", &namespace);
            return;
        }
    };
    let warnings = shared.warnings.clone();
    inflight.push(Box::pin(async move {
        let _charge = charge;
        if AssertUnwindSafe(future).catch_unwind().await.is_err() {
            warnings.panicked("an event handler", &namespace);
        }
    }));
}

/// The `CONNECT_ERROR` message for a client address at its share of pending disconnect hooks.
pub(crate) const TOO_MANY_PENDING_DISCONNECTS: &str = "Too many pending disconnects";

/// CONNECT: looks the namespace up and starts the connection attempt. `Err` for a namespace that is already joined or
/// pending.
fn begin_connect(
    server: &Arc<ServerInner>,
    shared: &Arc<ClientShared>,
    name: String,
    auth: Option<IoValue>,
) -> Result<(), ProtocolViolation> {
    let Some(namespace) = server.namespace(&name) else {
        let error = IoValue::object([("message", IoValue::from("Invalid namespace"))]);
        let _ = shared.send_packet(&Packet { nsp: name, body: Body::ConnectError(error) });
        return Ok(());
    };
    {
        let mut nsps = shared.nsps();
        if shared.done.load(Ordering::SeqCst) {
            return Ok(());
        }
        if matches!(nsps.get(&name), Some(Slot::Pending | Slot::Joined(_))) {
            return Err(ProtocolViolation);
        }
        // A socket of this namespace leaves a disconnect hook pending when it goes. An address at its share of pending
        // hooks gets no new socket here, as it gets no new session at the handshake: an open session joining and leaving
        // in a loop cannot pile hooks up either. Refused before any socket exists, with a fixed message.
        if namespace.limits_pending_hooks() && shared.engine.pending_hook_share_reached() {
            drop(nsps);
            let error = IoValue::object([("message", IoValue::from(TOO_MANY_PENDING_DISCONNECTS))]);
            let _ = shared.send_packet(&Packet { nsp: name, body: Body::ConnectError(error) });
            return Ok(());
        }
        nsps.insert(name.clone(), Slot::Pending);
    }
    // `auth` stays a `serde_json::Value` (it is the type of `Handshake::auth`), whose size is not the one the decoder
    // measured against `max_auth_bytes`: the real size of the stored value is charged to the session and its address
    // for as long as the handshake lives.
    let auth = auth.and_then(|v| v.to_json().ok()).unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let charge = shared.engine.hold(json_cost(&auth));
    let handshake = Handshake::new(auth, shared.engine.handshake_arc(), shared.engine.ip_arc(), Some(charge));
    let Some(socket) = IoSocket::new(namespace, shared.clone(), handshake) else {
        // The adapter panicked while the id was checked: the attempt is refused, and the slot is free again.
        shared.nsps().remove(&name);
        let error = IoValue::object([("message", IoValue::from("Internal server error"))]);
        let _ = shared.send_packet(&Packet { nsp: name, body: Body::ConnectError(error) });
        return Ok(());
    };
    tokio::spawn(connect(shared.clone(), socket));
    Ok(())
}

/// Runs the middleware of one connection attempt and, when it passes, joins the namespace.
async fn connect(shared: Arc<ClientShared>, socket: IoSocket) {
    let namespace = socket.namespace();
    let _attempt = namespace.connect_attempt();
    let name = namespace.name().to_string();
    let mut guard = PendingGuard { shared: shared.clone(), name: name.clone(), armed: true };
    let outcome = tokio::select! {
        outcome = run_middleware(&namespace, &socket, shared.options.middleware_timeout, &shared.warnings) => outcome,
        // The connection closed while the middleware ran: no CONNECT_ERROR, no handlers (the guard frees the slot).
        _ = shared.engine.closed() => return,
    };
    if let Err(error) = outcome {
        // The slot first: a client that retries on CONNECT_ERROR must find it free.
        drop(guard);
        let mut payload = indexmap::IndexMap::new();
        payload.insert("message".to_string(), IoValue::String(error.message));
        if let Some(data) = error.data {
            payload.insert("data".to_string(), data);
        }
        let _ = shared.send_packet(&Packet { nsp: name, body: Body::ConnectError(IoValue::Object(Box::new(payload))) });
        return;
    }
    // Register, join, announce (Node's `_doConnect` then `_onconnect`), with no lock of the connection held across the
    // adapter, a handler or the transport. Until it is announced the socket is joining: a lookup by id finds it,
    // broadcasts and its own emits skip it, and a teardown removes it silently.
    //
    // 1. The slot, so `terminate` finds the socket and tears it down. The phase is set under the same lock, which only
    //    the socket's state lock is ever taken inside.
    {
        let mut nsps = shared.nsps();
        if shared.done.load(Ordering::SeqCst) || !matches!(nsps.get(&name), Some(Slot::Pending)) {
            return;
        }
        nsps.insert(name.clone(), Slot::Joined(socket.clone()));
        guard.armed = false;
        socket.begin_joining();
    }
    namespace.connect_step(ConnectStep::Slotted).await;
    // 2. The namespace's table: from here a lookup by id finds the socket, before its client can know the id.
    if !namespace.add_socket(&socket) {
        return;
    }
    namespace.connect_step(ConnectStep::Registered).await;
    // 3. The room of its own id (the adapter may call anything, even close this connection). A failure refuses the
    //    attempt as a panic of the id check does: the client never sees the socket connected.
    if !socket.join_own_room() {
        socket.disconnect_with(DisconnectReason::ServerNamespaceDisconnect, false);
        let error = IoValue::object([("message", IoValue::from("Internal server error"))]);
        let _ = shared.send_packet(&Packet { nsp: name, body: Body::ConnectError(error) });
        return;
    }
    namespace.connect_step(ConnectStep::JoinedOwnRoom).await;
    // 4. CONNECT, queued and the socket marked connected in one step of its state lock: an emit that sees it connected
    //    is queued after CONNECT. The packet is encoded first, outside the lock.
    let sid = IoValue::object([("sid", IoValue::from(socket.id()))]);
    let Ok(encoded) = parser::encode(&Packet { nsp: name.clone(), body: Body::Connect(Some(sid)) }, shared.options.max_attachments) else {
        socket.disconnect_with(DisconnectReason::ServerNamespaceDisconnect, false);
        return;
    };
    match socket.announce(|| shared.engine.send(EngineMessage::Framed(encoded.text))) {
        // Torn down meanwhile.
        None => return,
        Some(Ok(())) => {}
        // The session is closing (or closed for a full send buffer): nothing of the socket stays.
        Some(Err(_)) => {
            socket.disconnect_with(DisconnectReason::TransportClose, false);
            return;
        }
    }
    shared.joined_once.store(true, Ordering::SeqCst);
    namespace.connect_step(ConnectStep::Announced).await;
    // 5. The connection handlers, right after the announce (the reference server's `_onconnect` then `connection`): the
    //    socket receives broadcasts from now on, so the hook where an application authenticates it cannot wait for
    //    the client (over long-polling, CONNECT is only written when the client polls). Packets of the client wait
    //    for `ready` meanwhile.
    if socket.is_phase(Phase::Connected) {
        for handler in namespace.connection_handlers() {
            let s = socket.clone();
            shared.warnings.guard("a connection handler", &name, || handler(s));
        }
    }
    socket.mark_ready();
}

async fn run_middleware(namespace: &IoNamespace, socket: &IoSocket, limit: Duration, warnings: &WarnLimiter) -> Result<(), ConnectError> {
    let chain = namespace.middleware();
    let name = namespace.name().to_string();
    let work = async {
        for middleware in chain {
            let future = match std::panic::catch_unwind(AssertUnwindSafe(|| middleware(socket.clone()))) {
                Ok(future) => future,
                Err(_) => {
                    warnings.panicked("a connect middleware", &name);
                    return Err(ConnectError::internal());
                }
            };
            match AssertUnwindSafe(future).catch_unwind().await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    warnings.panicked("a connect middleware", &name);
                    return Err(ConnectError::internal());
                }
            }
        }
        Ok(())
    };
    match tokio::time::timeout(limit, work).await {
        Ok(result) => result,
        Err(_) => {
            warnings.warn("a connect middleware timeout", &name, || {
                format!("the connect middleware of the namespace {name} took longer than {limit:?}: the connection was refused")
            });
            Err(ConnectError::internal())
        }
    }
}
