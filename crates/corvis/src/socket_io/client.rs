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
use super::parser::{self, Body, Decoder, Encoded, Limits, Packet, Prepared};
use super::server::ServerInner;
use super::socket::{AckSender, DisconnectReason, EmitError, Handshake, IoSocket, Phase};
use super::value::{Args, IoValue, json_cost};
use super::warn::WarnLimiter;

enum Slot {
    /// The CONNECT was accepted and the middleware is running.
    Pending,
    Joined(IoSocket),
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

    /// Queues an encoded packet: its text message and its attachments together, so nothing is queued between them.
    pub(crate) fn send_encoded(&self, encoded: &Encoded) -> Result<(), EmitError> {
        let mut messages = Vec::with_capacity(1 + encoded.attachments.len());
        messages.push(EngineMessage::Text(encoded.text.clone()));
        messages.extend(encoded.attachments.iter().map(|a| EngineMessage::Binary(a.bytes())));
        self.engine.send_all(messages).map_err(|e| match e {
            SendError::Closed => EmitError::NotConnected,
            SendError::Full => EmitError::BufferFull,
        })
    }

    fn send_packet(&self, packet: &Packet) -> Result<(), EmitError> {
        let encoded = parser::encode(packet, self.options.max_attachments).map_err(|_| EmitError::TooDeep)?;
        self.send_encoded(&encoded)
    }

    fn joined(&self, name: &str) -> Option<IoSocket> {
        match self.nsps().get(name) {
            Some(Slot::Joined(socket)) => Some(socket.clone()),
            _ => None,
        }
    }

    /// Removes a socket that left its namespace, so the client may join it again.
    pub(crate) fn forget(&self, name: &str, socket: &IoSocket) {
        let mut nsps = self.nsps();
        if matches!(nsps.get(name), Some(Slot::Joined(s)) if s == socket) {
            nsps.remove(name);
        }
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
                        Slot::Pending => None,
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
struct Charge {
    parts: Vec<Reservation>,
}

impl Charge {
    fn new(first: Reservation) -> Charge {
        Charge { parts: vec![first] }
    }

    fn add(&mut self, part: Reservation) {
        self.parts.push(part);
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

type Closed<'a> = Pin<Box<dyn Future<Output = CloseReason> + Send + 'a>>;

async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => futures::future::pending::<()>().await,
    }
}

/// The task of one Engine.IO connection.
pub(crate) async fn run(
    server: Arc<ServerInner>,
    shared: Arc<ClientShared>,
    mut messages: Pin<Box<dyn Stream<Item = (EngineMessage, Held)> + Send + 'static>>,
) {
    let options = shared.options.clone();
    let engine = shared.engine.clone();
    let mut decoder = Decoder::new(Limits::from_options(&options));
    let mut inflight: FuturesUnordered<BoxFuture<'static, ()>> = FuturesUnordered::new();
    let mut connect_deadline = Some(after(Instant::now(), options.connect_timeout));
    let mut reassembly: Option<(Instant, Charge)> = None;
    let mut parked: Option<Parked> = None;
    // When the parked message gives up waiting: set when it parks, cleared when it is admitted.
    let mut admission_deadline: Option<Instant> = None;
    let mut stream_done = false;
    let mut closed: Closed<'_> = Box::pin(engine.closed());
    loop {
        // Registered before anything below looks at the gauges, so a release in between wakes the select.
        let unread_changed = engine.unread_changed();
        if let Some(waiting) = parked.take() {
            let flow = admit(&server, &shared, &mut decoder, &mut reassembly, &mut parked, &mut inflight, &mut closed, waiting).await;
            match flow {
                Flow::Continue => {}
                Flow::Close(reason) => {
                    shared.terminate(reason, false, true);
                    break;
                }
                Flow::Gone(reason) => {
                    shared.terminate(reason.into(), false, false);
                    break;
                }
            }
        }
        if parked.is_none() {
            admission_deadline = None;
        } else if admission_deadline.is_none() {
            admission_deadline = Some(after(Instant::now(), options.admission_timeout));
        }
        let reassembling = decoder.is_reassembling();
        let over_limit = !reassembling && engine.unread_over_limit();
        let can_read = !stream_done && parked.is_none() && (reassembling || (inflight.len() < options.max_inflight && !over_limit));
        let connect_wait = wait_until(connect_deadline);
        let admission_wait = wait_until(admission_deadline);
        let reassembly_wait = wait_until(reassembly.as_ref().map(|(deadline, _)| *deadline));
        tokio::select! {
            biased;
            reason = &mut closed => {
                shared.terminate(reason.into(), false, false);
                break;
            }
            Some(()) = inflight.next(), if !inflight.is_empty() => {}
            _ = connect_wait => {
                connect_deadline = None;
                if !shared.joined_once.load(Ordering::SeqCst) {
                    shared.terminate(DisconnectReason::ForcedServerClose, false, true);
                    break;
                }
            }
            _ = reassembly_wait => {
                shared.terminate(DisconnectReason::ForcedServerClose, false, true);
                break;
            }
            // A packet waited for room longer than the limit: the wait is bounded and visible.
            _ = admission_wait => {
                shared.terminate(DisconnectReason::AdmissionTimeout, false, true);
                break;
            }
            // A release of room somewhere: the parked message, or the reads that are paused, may go on.
            _ = unread_changed, if parked.is_some() || over_limit => {}
            next = messages.next(), if can_read => {
                let Some((message, held)) = next else {
                    stream_done = true;
                    continue;
                };
                let flow = handle_message(&server, &shared, &mut decoder, &mut reassembly, &mut parked, &mut inflight, &mut closed, message, held).await;
                match flow {
                    Flow::Continue => {}
                    Flow::Close(reason) => {
                        shared.terminate(reason, false, true);
                        break;
                    }
                    Flow::Gone(reason) => {
                        shared.terminate(reason.into(), false, false);
                        break;
                    }
                }
            }
        }
    }
    // Handlers that already started get a grace period to finish; nothing can wait for them forever.
    if !inflight.is_empty() {
        let grace = options.engine.close_timeout;
        tokio::spawn(async move {
            let _ = tokio::time::timeout(grace, async move { while inflight.next().await.is_some() {} }).await;
        });
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_message(
    server: &Arc<ServerInner>,
    shared: &Arc<ClientShared>,
    decoder: &mut Decoder,
    reassembly: &mut Option<(Instant, Charge)>,
    parked: &mut Option<Parked>,
    inflight: &mut FuturesUnordered<BoxFuture<'static, ()>>,
    closed: &mut Closed<'_>,
    message: EngineMessage,
    held: Held,
) -> Flow {
    match message {
        EngineMessage::Text(text) => match decoder.prepare(text) {
            Err(_) => Flow::Close(DisconnectReason::ForcedClose),
            Ok(prepared) => admit(server, shared, decoder, reassembly, parked, inflight, closed, Parked { prepared, wire: held }).await,
        },
        EngineMessage::Binary(data) => match decoder.add_binary(data) {
            Err(_) => Flow::Close(DisconnectReason::ForcedClose),
            Ok(None) => {
                if let Some((_, charge)) = reassembly.as_mut() {
                    charge.add(Reservation::from_held(held));
                }
                Flow::Continue
            }
            Ok(Some(packet)) => {
                let Some((_, mut charge)) = reassembly.take() else { return Flow::Close(DisconnectReason::ForcedClose) };
                charge.add(Reservation::from_held(held));
                handle_packet(server, shared, packet, charge, inflight, closed).await
            }
        },
    }
}

/// Reserves the decoded cost of a measured message and decodes it; when the budget has no room the message is parked
/// and the connection takes no other until room is released.
#[allow(clippy::too_many_arguments)]
async fn admit(
    server: &Arc<ServerInner>,
    shared: &Arc<ClientShared>,
    decoder: &mut Decoder,
    reassembly: &mut Option<(Instant, Charge)>,
    parked: &mut Option<Parked>,
    inflight: &mut FuturesUnordered<BoxFuture<'static, ()>>,
    closed: &mut Closed<'_>,
    waiting: Parked,
) -> Flow {
    let Some(mut reservation) = Reservation::try_new(&shared.engine, waiting.prepared.reservation(), waiting.wire.bytes()) else {
        *parked = Some(waiting);
        return Flow::Continue;
    };
    let reserved = reservation.bytes();
    let Parked { prepared, wire } = waiting;
    match decoder.decode(prepared) {
        Err(_) => Flow::Close(DisconnectReason::ForcedClose),
        Ok(Some(packet)) => {
            // The text is gone with the decode: what stays is the decoded packet, at its real cost. The cost model is
            // an upper bound and the reservation was made from it; a packet above it would be a defect, and is refused.
            let cost = packet.cost();
            if cost > reserved {
                return Flow::Close(DisconnectReason::ForcedClose);
            }
            reservation.shrink_to(cost);
            drop(wire);
            handle_packet(server, shared, packet, Charge::new(reservation), inflight, closed).await
        }
        Ok(None) => {
            let cost = decoder.pending_cost();
            if cost > reserved {
                return Flow::Close(DisconnectReason::ForcedClose);
            }
            reservation.shrink_to(cost);
            drop(wire);
            *reassembly = Some((after(Instant::now(), shared.options.attachment_timeout), Charge::new(reservation)));
            Flow::Continue
        }
    }
}

async fn handle_packet(
    server: &Arc<ServerInner>,
    shared: &Arc<ClientShared>,
    packet: Packet,
    charge: Charge,
    inflight: &mut FuturesUnordered<BoxFuture<'static, ()>>,
    closed: &mut Closed<'_>,
) -> Flow {
    let Packet { nsp, body } = packet;
    match body {
        Body::Connect(auth) => match begin_connect(server, shared, nsp, auth) {
            Ok(()) => Flow::Continue,
            Err(()) => Flow::Close(DisconnectReason::ForcedServerClose),
        },
        // A client does not send these, and a packet for a namespace it has not joined is a violation.
        Body::ConnectError(_) => Flow::Close(DisconnectReason::ForcedServerClose),
        body => {
            let Some(socket) = shared.joined(&nsp) else { return Flow::Close(DisconnectReason::ForcedServerClose) };
            tokio::select! {
                _ = socket.wait_ready() => {}
                reason = &mut *closed => return Flow::Gone(reason),
            }
            if !socket.is_phase(Phase::Connected) {
                return Flow::Continue;
            }
            match body {
                Body::Disconnect => socket.disconnect_with(DisconnectReason::ClientNamespaceDisconnect, false),
                Body::Ack { id, args } => socket.resolve_ack(id, args),
                Body::Event { id, name, args } => dispatch(shared, &socket, id, &name, args, charge, inflight),
                Body::Connect(_) | Body::ConnectError(_) => unreachable!("handled above"),
            }
            Flow::Continue
        }
    }
}

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
    let Some(handler) = socket.handler(name) else { return };
    let ack = id.map(|id| AckSender::new(socket.clone(), id));
    let namespace = socket.namespace().name().to_string();
    let args = Args::new(args);
    let handler_socket = socket.clone();
    let created = std::panic::catch_unwind(AssertUnwindSafe(|| handler(handler_socket, args, ack)));
    let future = match created {
        Ok(future) => future,
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

/// CONNECT: looks the namespace up and starts the connection attempt. `Err` for a namespace that is already joined or
/// pending.
fn begin_connect(server: &Arc<ServerInner>, shared: &Arc<ClientShared>, name: String, auth: Option<IoValue>) -> Result<(), ()> {
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
        if nsps.contains_key(&name) {
            return Err(());
        }
        nsps.insert(name, Slot::Pending);
    }
    // `auth` stays a `serde_json::Value` (it is the type of `Handshake::auth`), whose size is not the one the decoder
    // measured against `max_auth_bytes`: the real size of the stored value is charged to the session and its address
    // for as long as the handshake lives.
    let auth = auth.and_then(|v| v.to_json().ok()).unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let charge = shared.engine.hold(json_cost(&auth));
    let handshake = Handshake::new(auth, shared.engine.handshake().clone(), Some(charge));
    let socket = IoSocket::new(namespace, shared.clone(), handshake);
    tokio::spawn(connect(shared.clone(), socket));
    Ok(())
}

/// Runs the middleware of one connection attempt and, when it passes, joins the namespace.
async fn connect(shared: Arc<ClientShared>, socket: IoSocket) {
    let namespace = socket.namespace();
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
    {
        let mut nsps = shared.nsps();
        if shared.done.load(Ordering::SeqCst) || !matches!(nsps.get(&name), Some(Slot::Pending)) {
            return;
        }
        // CONNECT goes out before the socket can be used, so nothing the application emits precedes it.
        let sid = IoValue::object([("sid", IoValue::from(socket.id()))]);
        if shared.send_packet(&Packet { nsp: name.clone(), body: Body::Connect(Some(sid)) }).is_err() {
            return;
        }
        nsps.insert(name.clone(), Slot::Joined(socket.clone()));
        guard.armed = false;
        socket.mark_connected();
        namespace.add_socket(&socket);
    }
    shared.joined_once.store(true, Ordering::SeqCst);
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
            warnings.warn(
                "a connect middleware timeout",
                &name,
                &format!("the connect middleware of the namespace {name} took longer than {limit:?}: the connection was refused"),
            );
            Err(ConnectError::internal())
        }
    }
}
