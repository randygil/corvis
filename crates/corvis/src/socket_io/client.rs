//! One Engine.IO connection as the Socket.IO layer sees it: the decoder, the namespaces it joined or is joining, the
//! connect timeout, the handlers in flight and what is held against the unread budget.
//!
//! One task per connection reads its messages. Packets of the connection are handled in arrival order; the handlers of
//! events run concurrently inside that task (so they start in order), at most `max_inflight` at a time. The task takes
//! the next message only while a handler slot is free and while what it holds (reassembling packets, packets being
//! handled) is under `max_unread_bytes`; everything it holds is charged to the Engine.IO session's unread gauge until
//! the packet's handler finishes, so the transport stops reading when the budget is spent and no queue of the
//! Socket.IO layer can grow without bound.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, Stream, StreamExt};
use tokio::time::Instant;

use super::engine::session::{CloseReason, UNREAD_MESSAGE_OVERHEAD, after};
use super::engine::{EngineMessage, EngineSocket, Held, SendError};
use super::namespace::{ConnectError, IoNamespace};
use super::options::IoOptions;
use super::parser::{self, Body, Decoder, Encoded, Limits, Packet};
use super::server::ServerInner;
use super::socket::{AckSender, DisconnectReason, EmitError, Handshake, IoSocket, Phase, log_panic};
use super::value::{Args, IoValue};
use crate::logger::Logger;

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
    /// Bytes of received data this connection holds (charged to the session's gauge too).
    held: Arc<AtomicUsize>,
}

impl ClientShared {
    pub(crate) fn new(engine: EngineSocket, options: Arc<IoOptions>) -> Arc<ClientShared> {
        Arc::new(ClientShared {
            engine,
            options,
            nsps: Mutex::new(HashMap::new()),
            done: AtomicBool::new(false),
            joined_once: AtomicBool::new(false),
            held: Arc::new(AtomicUsize::new(0)),
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

/// Bytes held against the budget while a packet is being reassembled or handled.
struct Charge {
    guards: Vec<Held>,
    bytes: usize,
    counter: Arc<AtomicUsize>,
}

impl Charge {
    fn new(counter: &Arc<AtomicUsize>) -> Charge {
        Charge { guards: Vec::new(), bytes: 0, counter: counter.clone() }
    }

    fn add(&mut self, held: Held, bytes: usize) {
        self.counter.fetch_add(bytes, Ordering::SeqCst);
        self.bytes += bytes;
        self.guards.push(held);
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.counter.fetch_sub(self.bytes, Ordering::SeqCst);
    }
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
    let mut stream_done = false;
    let mut closed: Closed<'_> = Box::pin(engine.closed());
    let max_unread = options.engine.max_unread_bytes;
    loop {
        let reassembling = decoder.is_reassembling();
        let can_read =
            !stream_done && (reassembling || (inflight.len() < options.max_inflight && shared.held.load(Ordering::SeqCst) < max_unread));
        let connect_wait = wait_until(connect_deadline);
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
            next = messages.next(), if can_read => {
                let Some((message, held)) = next else {
                    stream_done = true;
                    continue;
                };
                let flow = handle_message(&server, &shared, &mut decoder, &mut reassembly, &mut inflight, &mut closed, message, held).await;
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
    inflight: &mut FuturesUnordered<BoxFuture<'static, ()>>,
    closed: &mut Closed<'_>,
    message: EngineMessage,
    held: Held,
) -> Flow {
    match message {
        EngineMessage::Text(text) => match decoder.add_text(&text) {
            Err(_) => Flow::Close(DisconnectReason::ForcedClose),
            Ok(Some(packet)) => {
                // The text is dropped with this message: what stays is the decoded packet, at its real cost.
                let mut charge = Charge::new(&shared.held);
                let cost = packet.cost();
                charge.add(shared.engine.hold(cost), cost);
                drop(held);
                drop(text);
                handle_packet(server, shared, packet, charge, inflight, closed).await
            }
            Ok(None) => {
                let mut charge = Charge::new(&shared.held);
                let cost = decoder.pending_cost();
                charge.add(shared.engine.hold(cost), cost);
                drop(held);
                *reassembly = Some((after(Instant::now(), shared.options.attachment_timeout), charge));
                Flow::Continue
            }
        },
        EngineMessage::Binary(data) => {
            let bytes = data.len() + UNREAD_MESSAGE_OVERHEAD;
            match decoder.add_binary(data) {
                Err(_) => Flow::Close(DisconnectReason::ForcedClose),
                Ok(None) => {
                    if let Some((_, charge)) = reassembly.as_mut() {
                        charge.add(held, bytes);
                    }
                    Flow::Continue
                }
                Ok(Some(packet)) => {
                    let Some((_, mut charge)) = reassembly.take() else { return Flow::Close(DisconnectReason::ForcedClose) };
                    charge.add(held, bytes);
                    handle_packet(server, shared, packet, charge, inflight, closed).await
                }
            }
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
    let _ = shared;
    let Some(handler) = socket.handler(name) else { return };
    let ack = id.map(|id| AckSender::new(socket.clone(), id));
    let namespace = socket.namespace().name().to_string();
    let args = Args::new(args);
    let handler_socket = socket.clone();
    let created = std::panic::catch_unwind(AssertUnwindSafe(|| handler(handler_socket, args, ack)));
    let future = match created {
        Ok(future) => future,
        Err(_) => {
            log_panic("an event handler", &namespace);
            return;
        }
    };
    inflight.push(Box::pin(async move {
        let _charge = charge;
        if AssertUnwindSafe(future).catch_unwind().await.is_err() {
            log_panic("an event handler", &namespace);
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
    let auth = auth.and_then(|v| v.to_json().ok()).unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let handshake = Handshake::new(auth, shared.engine.handshake().clone());
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
        outcome = run_middleware(&namespace, &socket, shared.options.middleware_timeout) => outcome,
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
            if !crate::ws::guarded("a Socket.IO connection handler", || handler(s)) {
                log_panic("a connection handler", &name);
            }
        }
    }
    socket.mark_ready();
}

async fn run_middleware(namespace: &IoNamespace, socket: &IoSocket, limit: Duration) -> Result<(), ConnectError> {
    let chain = namespace.middleware();
    let name = namespace.name().to_string();
    let work = async {
        for middleware in chain {
            let future = match std::panic::catch_unwind(AssertUnwindSafe(|| middleware(socket.clone()))) {
                Ok(future) => future,
                Err(_) => {
                    log_panic("connect middleware", &name);
                    return Err(ConnectError::internal());
                }
            };
            match AssertUnwindSafe(future).catch_unwind().await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    log_panic("connect middleware", &name);
                    return Err(ConnectError::internal());
                }
            }
        }
        Ok(())
    };
    match tokio::time::timeout(limit, work).await {
        Ok(result) => result,
        Err(_) => {
            Logger::new("SocketIO")
                .warn(format!("the connect middleware of the namespace {name} took longer than {limit:?}: the connection was refused"));
            Err(ConnectError::internal())
        }
    }
}
