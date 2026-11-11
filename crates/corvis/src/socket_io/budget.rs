//! Who may hold how much of the server-wide unread budget.
//!
//! There is one accounting path. Everything a connection receives from the client after its handshake and keeps is
//! charged to the unread gauge of its Engine.IO session, and a session's gauge is chained to the gauge of its client
//! address, which is chained to the server-wide one (`engine/mod.rs`, `ws.rs`). A charge anywhere in the chain counts
//! in every gauge above it, so the address gauge holds, in every state and by construction:
//!
//! - the wire messages the transport received and nobody has taken yet, the one a connection took and parked, and the
//!   attachments of a binary packet being reassembled or already delivered;
//! - the reservations of decoded packets, shrunk to their real cost, until their handlers finish, including handlers
//!   that outlive their session (their charges keep the whole chain alive);
//! - what the application's handshake `auth` occupies for the socket's life.
//!
//! What is not charged: the HTTP request that opened the session. The session keeps it once, shared by every socket of
//! the connection, and it is bounded by the HTTP server's header limit before any session exists (at most
//! `max_sessions x max_header_size` in all), like the session's other fixed state. A charge for the session's whole
//! life would occupy part of its fair share and of its address's quota for good.
//!
//! Two rules decide who may take room, both checked before a single value is built:
//!
//! - **The guaranteed share.** A session may always hold up to the fair share of the server-wide budget
//!   (`f = max(16 KiB, max_total_unread_bytes / max_sessions)`), whatever others hold, as long as its address stays
//!   within its cap (`A + N x f`: the quota plus the guaranteed shares of the `N = max_sessions_per_ip` sessions an
//!   address may have open). An honest session with small packets is never starved.
//! - **The shared budget.** Anything beyond the share needs the address to stay within its quota `A`
//!   (`max_address_share`) and the server-wide budget to stay within `T` (`max_total_unread_bytes`), both checked
//!   atomically (compare-and-swap at each level, so concurrent connections cannot overshoot together).
//!
//! Received messages are charged when they arrive and cannot be refused, so the same limits stop the reading instead
//! (`UnreadGauge::over_limit`): a session above its fair share is not read while its address is above `A` or the
//! server-wide budget is above `T`, and every session is read while it is at or below `f`. A packet that finds no room
//! waits, parked, for as long as `admission_timeout`: its connection takes no further message from the engine.

use super::engine::{EngineSocket, Held};

/// Room taken in the budget: released, in the session's gauge and so in its address's and the server's (which wakes
/// the connections that wait), when this is dropped.
pub(crate) struct Reservation {
    held: Held,
}

impl Reservation {
    /// Wraps bytes that are already charged (a received message kept while its packet is reassembled).
    pub(crate) fn from_held(held: Held) -> Reservation {
        Reservation { held }
    }

    /// Tries to reserve `bytes` for a connection of `engine`; `None` when there is no room now. `credit` is the wire
    /// message the reservation is for, already charged and released once the packet is decoded: the limits are checked
    /// against what the connection would hold without it, so a message is not counted twice.
    pub(crate) fn try_new(engine: &EngineSocket, bytes: usize, credit: usize) -> Option<Reservation> {
        // The guaranteed share first, then the shared budget (the address's quota and the server-wide one).
        let held = engine.hold_within_share(bytes, credit).or_else(|| engine.hold_within_total(bytes, credit))?;
        Some(Reservation { held })
    }

    /// Lowers the reservation to `bytes` (never raises it), giving the rest back to the budget.
    pub(crate) fn shrink_to(&mut self, bytes: usize) {
        self.held.shrink_to(bytes);
    }

    /// The bytes reserved.
    pub(crate) fn bytes(&self) -> usize {
        self.held.bytes()
    }
}
