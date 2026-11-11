//! Socket.IO on the application's HTTP server (preview, feature `socket-io`).
//!
//! Two layers: the Engine.IO v4 transport (handshake, HTTP long-polling, upgrade to WebSocket, heartbeat), in the
//! unstable, hidden `engine` module, and on top of it the Socket.IO v5 protocol: namespaces multiplexed over one
//! session, the connection handshake with `auth`, connect middleware, events and acknowledgements in both
//! directions, binary attachments and disconnect reasons. The wire behavior follows Node's `socket.io` 4.8.4 for
//! the official client; every limit a client can make the server hit has a bounded default (see [`IoOptions`](crate::socket_io::IoOptions)).
//!
//! ```no_run
//! use corvis::prelude::*;
//! use corvis::socket_io::{ConnectError, IoOptions, IoServer};
//!
//! # async fn demo(app: &CorvisApplication) -> Result<(), Box<dyn std::error::Error>> {
//! let io = IoServer::new(IoOptions::new())?;
//! io.of("/admin")?.use_middleware(|socket| async move {
//!     match socket.handshake().auth().get("token").and_then(|t| t.as_str()) {
//!         Some("secret") => Ok(()),
//!         _ => Err(ConnectError::new("not authorized")),
//!     }
//! });
//! io.main().on_connection(|socket| {
//!     socket.on("sum", |_socket, args, ack| async move {
//!         let total: i64 = args.iter().filter_map(|a| a.as_i64()).sum();
//!         if let Some(ack) = ack {
//!             let _ = ack.send((total,));
//!         }
//!     });
//!     let _ = socket.emit("hello", ("world",));
//! });
//! io.attach(&app.get_http_server())?;
//! # Ok(()) }
//! ```
//!
//! Rooms and broadcast: `socket.join(room)`, `io.to(room).except(other).emit(...)`, `socket.broadcast().emit(...)`
//! (see [`BroadcastOperator`](crate::socket_io::BroadcastOperator)); the rooms of a namespace live in an adapter, in memory by default.
//!
//! Gateways: `use_websocket_adapter(IoAdapter::new())` serves the application's `#[websocket_gateway]` classes over Socket.IO (see
//! [`IoAdapter`]).
//!
//! Not supported (yet): a Redis adapter, dynamic namespaces,
//! connection state recovery, `volatile` and `compress` flags, catch-all listeners, custom parsers, Engine.IO v3
//! clients and serving the client bundle.

#[doc(hidden)]
pub mod adapter;
#[doc(hidden)]
pub mod engine;

mod broadcast;
mod budget;
mod client;
mod gateway;
mod namespace;
mod options;
mod parser;
mod server;
mod socket;
mod value;
mod warn;

#[cfg(test)]
mod fuzz_adapter;
#[cfg(test)]
mod fuzz_parser;

pub use adapter::{AdapterError, HandshakeInfo, RoomError, SocketInfo};
pub use broadcast::{BroadcastAckError, BroadcastAckErrorKind, BroadcastOperator, IntoRooms, RemoteSocket};
pub use engine::options::{Admission, EngineOptions, EngineOptionsError};
pub use gateway::{DEFAULT_HOOK_TIMEOUT, IoAdapter, IoGatewayBroadcast, IoGatewayServer};
pub(crate) use gateway::{start as gateway_start, validate as gateway_validate};
pub use namespace::{ConnectError, IoNamespace};
pub use options::{IoOptions, IoOptionsError};
pub use server::{IoServer, NamespaceNameError};
pub use socket::{AckEmitter, AckError, AckSender, DisconnectReason, EmitError, Handshake, IoSocket};
pub use value::{Args, Binary, IntoArgs, IoValue, IoValueError, MAX_DEPTH};

/// What decoding one packet cost, for the tests that measure the real heap against the model. Unstable and hidden.
#[doc(hidden)]
pub struct DecodeProbe {
    /// What the connection reserves before it decodes the packet.
    pub reservation: usize,
    /// What the decoded packet is charged once decoded.
    pub cost: usize,
    _keep: Box<dyn std::any::Any + Send>,
}

/// Decodes one text packet the way a connection does (measure, then decode) with the given `max_decoded_bytes`, and
/// keeps the result alive in the probe. Unstable and hidden: for the allocator-based tests of the cost model.
#[doc(hidden)]
pub fn probe_decode(text: &str, max_decoded_bytes: usize) -> Result<DecodeProbe, String> {
    let options = IoOptions::new().max_decoded_bytes(max_decoded_bytes);
    let mut decoder = parser::Decoder::new(parser::Limits::from_options(&options));
    let prepared = decoder.prepare(text.to_owned()).map_err(|e| e.to_string())?;
    let reservation = prepared.reservation();
    let packet = decoder.decode(prepared).map_err(|e| e.to_string())?;
    let cost = packet.as_ref().map_or_else(|| decoder.pending_cost(), parser::Packet::cost);
    // What is kept is the packet, or the decoder holding the header of a binary packet: the model charges both.
    let keep: Box<dyn std::any::Any + Send> = match packet {
        Some(packet) => Box::new(packet),
        None => Box::new(decoder),
    };
    Ok(DecodeProbe { reservation, cost, _keep: keep })
}

/// Decodes the CONNECT packet `text` (for example `0/chat,{"token":"x"}`) the way a connection does, converts its
/// `auth` into the `serde_json::Value` a socket keeps for its life, and keeps that value alive in the probe. `cost` is
/// what the socket's handshake is charged for it, `reservation` what the decoder reserved. Unstable and hidden: for the
/// allocator-based test of the charge of `auth`.
#[doc(hidden)]
pub fn probe_auth(text: &str, max_decoded_bytes: usize) -> Result<DecodeProbe, String> {
    let options = IoOptions::new().max_decoded_bytes(max_decoded_bytes);
    let mut decoder = parser::Decoder::new(parser::Limits::from_options(&options));
    let prepared = decoder.prepare(text.to_owned()).map_err(|e| e.to_string())?;
    let reservation = prepared.reservation();
    let packet = decoder.decode(prepared).map_err(|e| e.to_string())?.ok_or("not a CONNECT packet")?;
    let parser::Body::Connect(Some(auth)) = packet.body else { return Err("a CONNECT packet with auth is expected".to_string()) };
    let stored = auth.to_json().map_err(|e| e.to_string())?;
    drop(auth);
    Ok(DecodeProbe { reservation, cost: value::json_cost(&stored), _keep: Box::new(stored) })
}

#[cfg(test)]
mod tests;
