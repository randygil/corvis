//! Socket.IO on the application's HTTP server (preview, feature `socket-io`).
//!
//! This change ships the Engine.IO v4 transport layer only: handshake, HTTP long-polling, upgrade to WebSocket and
//! heartbeat, with the wire behavior of Node's `engine.io` 6.6.x. Socket.IO packets, namespaces, events and
//! acknowledgements follow in later changes. The Engine.IO server handle lives in the unstable, hidden `engine`
//! module; `EngineOptions` is the documented configuration.

#[doc(hidden)]
pub mod engine;

pub use engine::options::{EngineOptions, EngineOptionsError};
