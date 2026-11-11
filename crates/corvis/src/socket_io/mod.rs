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
//! [`IoAdapter`](crate::socket_io::IoAdapter)).
//!
//! Not supported (yet): a Redis adapter, dynamic namespaces,
//! connection state recovery, `volatile` and `compress` flags, catch-all listeners, custom parsers, Engine.IO v3
//! clients and serving the client bundle.

#[doc(hidden)]
pub mod adapter;
#[doc(hidden)]
pub mod engine;

mod ack;
mod broadcast;
mod budget;
mod client;
pub(crate) mod gateway;
mod info;
mod namespace;
mod options;
mod parser;
mod server;
mod socket;
#[doc(hidden)]
pub mod testing;
mod value;
mod warn;

#[cfg(test)]
mod fuzz_adapter;
#[cfg(test)]
mod fuzz_parser;

pub use adapter::{AdapterError, HandshakeInfo, RoomError, SocketInfo};
pub use broadcast::{BroadcastAckError, BroadcastAckErrorKind, BroadcastOperator, IntoRooms, RemoteSocket};
pub use engine::options::{Admission, EngineOptions, EngineOptionsError};
pub use gateway::{
    DEFAULT_HOOK_TIMEOUT, DEFAULT_MAX_PENDING_DISCONNECT_HOOK_BYTES, DEFAULT_MAX_PENDING_DISCONNECT_HOOKS, IoAdapter, IoGatewayBroadcast,
    IoGatewayServer,
};
pub(crate) use gateway::{start as gateway_start, validate as gateway_validate};
pub use namespace::{ConnectError, IoNamespace};
pub use options::{IoOptions, IoOptionsError};
pub use server::{IoServer, NamespaceNameError};
pub use socket::{AckEmitter, AckError, AckSender, DisconnectReason, EmitError, Handshake, IoSocket};
pub use value::{Args, Binary, IntoArgs, IoValue, IoValueError, MAX_DEPTH};

#[cfg(test)]
mod tests;
