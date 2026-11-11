//! Options of the Socket.IO server: Node's `socket.io` defaults where there is a Node default, and a bounded default
//! for everything a client can make the server hold.

use std::sync::Arc;
use std::time::Duration;

use super::adapter::{Adapter, AdapterFactory, LocalNamespace};
use super::engine::options::{EngineOptions, EngineOptionsError, MAX_DURATION};

/// `connectTimeout` of Node's `socket.io`.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_millis(45_000);
/// Default deadline of an acknowledged emit. Node waits forever unless the call uses `timeout()`.
pub const DEFAULT_ACK_TIMEOUT: Duration = Duration::from_secs(30);
/// `maxAttachments` of `socket.io-parser` 4.2.7.
pub const DEFAULT_MAX_ATTACHMENTS: usize = 10;
/// Handlers that may run at the same time on one connection.
pub const DEFAULT_MAX_INFLIGHT: usize = 32;
/// Unanswered server-side acknowledgements per socket.
pub const DEFAULT_MAX_PENDING_ACKS: usize = 1_000;
/// What one packet may cost once decoded (see [`IoOptions::max_decoded_bytes`]): 32 MiB, which keeps a 1 MB text
/// packet of ordinary JSON working (it decodes to 11 to 20 times its size) and refuses the shapes that expand more
/// than 33 times (the worst, nested objects with empty keys, expands 58 times).
pub const DEFAULT_MAX_DECODED_BYTES: usize = 32 * 1024 * 1024;
/// What the `auth` payload of a CONNECT may cost once decoded.
pub const DEFAULT_MAX_AUTH_BYTES: usize = 16 * 1024;
/// Longest event name accepted from a client, in bytes.
pub const DEFAULT_MAX_EVENT_NAME_LEN: usize = 256;
/// Most arguments (and so array items) of one event or acknowledgement received from a client.
pub const DEFAULT_MAX_ARGS: usize = 128;
/// How long the attachments of one binary packet may take to arrive after its header.
pub const DEFAULT_ATTACHMENT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the connect middleware of one connection attempt may run.
pub const DEFAULT_MIDDLEWARE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a call of the adapter made by `emit`, `fetch_sockets` and the bulk operations, and its `init`, may take
/// (see [`IoOptions::adapter_timeout`]).
pub const DEFAULT_ADAPTER_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a packet may wait for room in the unread budget (see [`IoOptions::admission_timeout`]).
pub const DEFAULT_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);
/// The default share of the server-wide unread budget one client address may hold: one in this many (see
/// [`IoOptions::max_address_share`]).
pub const DEFAULT_ADDRESS_SHARE_DIVISOR: usize = 16;

/// Rooms one socket may be in, its own id's room included (Node has no bound).
pub const DEFAULT_MAX_ROOMS_PER_SOCKET: usize = 1_000;
/// Longest room name, in bytes (Node accepts any value).
pub const DEFAULT_MAX_ROOM_NAME_LENGTH: usize = 256;
/// What the answers collected by one broadcast with acknowledgements may cost (Node has no bound): 16 MiB.
pub const DEFAULT_MAX_BROADCAST_ACK_BYTES: usize = 16 * 1024 * 1024;
/// The shortest `max_room_name_length` accepted: a socket id (20 characters) must fit, because every socket is in the
/// room named after its id.
pub const MIN_ROOM_NAME_LENGTH: usize = 20;

/// Why a set of [`IoOptions`] was refused when creating the server.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IoOptionsError {
    /// A numeric option that must be positive was zero.
    #[error("Socket.IO option `{option}` must not be zero")]
    Zero { option: &'static str },
    /// A value outside its allowed range, or inconsistent with another option.
    #[error("Socket.IO option `{option}` {reason}")]
    Range { option: &'static str, reason: String },
    /// The Engine.IO layer refused its options.
    #[error(transparent)]
    Engine(#[from] EngineOptionsError),
}

/// Options of an [`IoServer`](super::IoServer).
///
/// Every limit has a bounded default, zero is refused, and `IoServer::new` names the option it refuses. Where Node's
/// `socket.io` 4.8.4 has no bound the table says so: those are the places where Corvis deliberately differs.
///
/// | Option | Default | Node `socket.io` 4.8.4 |
/// |---|---|---|
/// | [`engine`](Self::engine) | [`EngineOptions`] defaults | `engine.io` defaults |
/// | [`connect_timeout`](Self::connect_timeout) | 45 s | same (`connectTimeout`) |
/// | [`ack_timeout`](Self::ack_timeout) | 30 s | none: waits forever without `timeout()` |
/// | [`max_attachments`](Self::max_attachments) | 10 | 10 when decoding, none when encoding |
/// | [`max_inflight`](Self::max_inflight) | 32 per connection | none |
/// | [`max_pending_acks`](Self::max_pending_acks) | 1,000 per socket | none |
/// | [`max_decoded_bytes`](Self::max_decoded_bytes) | 32 MiB per packet | none |
/// | [`max_auth_bytes`](Self::max_auth_bytes) | 16 KiB | none |
/// | [`max_event_name_len`](Self::max_event_name_len) | 256 bytes | none |
/// | [`max_args`](Self::max_args) | 128 | none |
/// | [`attachment_timeout`](Self::attachment_timeout) | 30 s | none |
/// | [`middleware_timeout`](Self::middleware_timeout) | 10 s | none |
/// | [`max_address_share`](Self::max_address_share) | 1/16 of the engine's `max_total_unread_bytes`, at least one maximum packet | none |
/// | [`admission_timeout`](Self::admission_timeout) | 30 s | none |
/// | [`max_rooms_per_socket`](Self::max_rooms_per_socket) | 1,000 (own id's room included) | none |
/// | [`max_room_name_length`](Self::max_room_name_length) | 256 bytes | none |
/// | [`max_broadcast_ack_bytes`](Self::max_broadcast_ack_bytes) | 16 MiB per broadcast | none |
/// | [`adapter_timeout`](Self::adapter_timeout) | 5 s per adapter call | none |
///
/// Other differences, fixed rather than configurable: JSON nested deeper than 127 levels is refused; a client
/// acknowledgement id has at most 16 digits and is at most 2^53 - 1; a connection holds at most one socket per
/// namespace, counting connection attempts still in middleware (Node starts another pending socket for each
/// repeated CONNECT); a namespace name is at most 256 bytes without `,` or control characters; socket ids have 120
/// random bits (Node: 96 plus a counter); a packet that does not decode, a missing payload, a text packet type with
/// anything but a decimal digit, or a placeholder index that is not an integer closes the connection.
#[derive(Debug, Clone)]
pub struct IoOptions {
    pub(crate) engine: EngineOptions,
    pub(crate) connect_timeout: Duration,
    pub(crate) ack_timeout: Duration,
    pub(crate) max_attachments: usize,
    pub(crate) max_inflight: usize,
    pub(crate) max_pending_acks: usize,
    pub(crate) max_decoded_bytes: usize,
    pub(crate) max_auth_bytes: usize,
    pub(crate) max_event_name_len: usize,
    pub(crate) max_args: usize,
    pub(crate) attachment_timeout: Duration,
    pub(crate) middleware_timeout: Duration,
    pub(crate) max_address_share: Option<usize>,
    pub(crate) admission_timeout: Duration,
    pub(crate) finish_handlers_after_client_close: bool,
    pub(crate) max_rooms_per_socket: usize,
    pub(crate) max_room_name_length: usize,
    pub(crate) max_broadcast_ack_bytes: usize,
    pub(crate) adapter_timeout: Duration,
    pub(crate) adapter: AdapterFactory,
}

impl Default for IoOptions {
    fn default() -> Self {
        IoOptions {
            engine: EngineOptions::new(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            ack_timeout: DEFAULT_ACK_TIMEOUT,
            max_attachments: DEFAULT_MAX_ATTACHMENTS,
            max_inflight: DEFAULT_MAX_INFLIGHT,
            max_pending_acks: DEFAULT_MAX_PENDING_ACKS,
            max_decoded_bytes: DEFAULT_MAX_DECODED_BYTES,
            max_auth_bytes: DEFAULT_MAX_AUTH_BYTES,
            max_event_name_len: DEFAULT_MAX_EVENT_NAME_LEN,
            max_args: DEFAULT_MAX_ARGS,
            attachment_timeout: DEFAULT_ATTACHMENT_TIMEOUT,
            middleware_timeout: DEFAULT_MIDDLEWARE_TIMEOUT,
            max_address_share: None,
            admission_timeout: DEFAULT_ADMISSION_TIMEOUT,
            finish_handlers_after_client_close: false,
            max_rooms_per_socket: DEFAULT_MAX_ROOMS_PER_SOCKET,
            max_room_name_length: DEFAULT_MAX_ROOM_NAME_LENGTH,
            max_broadcast_ack_bytes: DEFAULT_MAX_BROADCAST_ACK_BYTES,
            adapter_timeout: DEFAULT_ADAPTER_TIMEOUT,
            adapter: AdapterFactory::in_memory(),
        }
    }
}

impl IoOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The Engine.IO options (path, heartbeat, buffer limits, session caps, CORS, `allow_request`).
    pub fn engine(mut self, engine: EngineOptions) -> Self {
        self.engine = engine;
        self
    }

    /// Node's `connectTimeout`: a connection that has joined no namespace this long after it opened is closed. A
    /// connection attempt still in middleware does not count as joined.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// The deadline of `emit_with_ack` when the call does not set one with `timeout()`; an unanswered call then
    /// fails with `operation has timed out`. Node waits forever.
    pub fn ack_timeout(mut self, timeout: Duration) -> Self {
        self.ack_timeout = timeout;
        self
    }

    /// The most attachments of one binary packet, in both directions. A received packet that declares more closes
    /// the connection before any attachment is accepted; an emit with more fails without sending. Node's decoder
    /// has the same limit (10); its encoder has none.
    pub fn max_attachments(mut self, attachments: usize) -> Self {
        self.max_attachments = attachments;
        self
    }

    /// The most event handlers of one connection (all its namespaces together) that run at the same time. At the
    /// limit the connection is not read until one finishes. Node has no limit.
    pub fn max_inflight(mut self, handlers: usize) -> Self {
        self.max_inflight = handlers;
        self
    }

    /// The most unanswered acknowledged emits of one socket; a call over it fails at once with
    /// `AckError::TooManyPending` and sends nothing. Node has no limit.
    pub fn max_pending_acks(mut self, acks: usize) -> Self {
        self.max_pending_acks = acks;
        self
    }

    /// The most a decoded packet may cost in memory: an upper bound of the heap its structure occupies (every value
    /// takes a 40-byte slot, an object adds a box and a hash table, a string its bytes, and every allocation 24
    /// bytes of bookkeeping), worked out from the text before anything is built. A packet above it closes the
    /// connection and nothing is allocated for it. Packets of ordinary JSON decode to 11 to 20 times their size and
    /// the worst shape (nested objects with empty keys) to 58 times, so with 1 MB of text the default 32 MiB keeps
    /// anything up to 33 times. The cost is reserved in the server-wide unread budget before the packet is decoded and
    /// charged until its handler finishes. Must be at least `max_http_buffer_size`, and at most the engine's
    /// `max_total_unread_bytes`, so that one packet can always be reserved. Node has no limit.
    pub fn max_decoded_bytes(mut self, bytes: usize) -> Self {
        self.max_decoded_bytes = bytes;
        self
    }

    /// The most the `auth` payload of a CONNECT may cost once decoded (same measure as `max_decoded_bytes`); the
    /// payload is kept for as long as the socket lives. Node has no limit.
    pub fn max_auth_bytes(mut self, bytes: usize) -> Self {
        self.max_auth_bytes = bytes;
        self
    }

    /// The longest event name a client may send, in bytes. Node has no limit.
    pub fn max_event_name_len(mut self, bytes: usize) -> Self {
        self.max_event_name_len = bytes;
        self
    }

    /// The most arguments of one event or acknowledgement received from a client. Node has no limit.
    pub fn max_args(mut self, args: usize) -> Self {
        self.max_args = args;
        self
    }

    /// How long the attachments of one binary packet may take to arrive after its header; a connection that has
    /// not completed the packet by then is closed, so a half-sent binary event cannot be held while the client
    /// keeps answering pings. Node has no limit.
    pub fn attachment_timeout(mut self, timeout: Duration) -> Self {
        self.attachment_timeout = timeout;
        self
    }

    /// How long the connect middleware of one connection attempt may run before it is refused with
    /// `Internal server error`. Node has no limit.
    pub fn middleware_timeout(mut self, timeout: Duration) -> Self {
        self.middleware_timeout = timeout;
        self
    }

    /// The most of the server-wide unread budget (the engine's `max_total_unread_bytes`) that the sessions of one
    /// client address may hold together, in every state: received messages, packets waiting for room, packets being
    /// reassembled, decoded packets and binary attachments until their handlers finish (handlers that outlive their
    /// session included), and the handshake `auth` of their sockets. Each of the `max_sessions_per_ip` sessions an
    /// address may have open also keeps its guaranteed share on top of it. Default: 1/16 of
    /// `max_total_unread_bytes`, but never less than one maximum packet (`max_decoded_bytes` plus the largest binary
    /// packet's wire bytes), or an address could not complete a single packet; with the default `max_decoded_bytes`
    /// that floor is the larger number, so see [`address_share`](Self::address_share) for the value in force. An
    /// explicit value must be between that floor and `max_total_unread_bytes`. How many addresses at their share
    /// exhaust the budget is `max_total_unread_bytes / share`: an application behind a reverse proxy must set
    /// `trust_proxy` so that the address is the client's and not the proxy's. Node has no limit.
    pub fn max_address_share(mut self, bytes: usize) -> Self {
        self.max_address_share = Some(bytes);
        self
    }

    /// How long a packet that found no room in the unread budget may wait before its connection is closed with the
    /// disconnect reason `admission timeout`. While a connection waits only because the server-wide budget is
    /// exhausted (not because of what it or its address holds) its heartbeat is held, so it is not closed with `ping
    /// timeout` for memory it did not hold; this bounds that wait. Default 30 s, above zero, at most a day. Node has
    /// no limit.
    pub fn admission_timeout(mut self, timeout: Duration) -> Self {
        self.admission_timeout = timeout;
        self
    }

    /// What happens to the event handlers that are running when the client drops the connection (the transport closes
    /// or fails, or the client stops answering pings). By default they are cancelled at once: a handler must not
    /// assume it runs to completion after its client is gone, because a client that reconnects in a loop would
    /// otherwise leave a crowd of handlers behind. With `true` they get `close_timeout` to finish, as they always do
    /// when the server closes the connection or shuts down; the sessions of an address that has
    /// `max_sessions_per_ip x max_inflight` such handlers still running are refused with `503` until some finish.
    /// Node runs its handlers to completion.
    pub fn finish_handlers_after_client_close(mut self, finish: bool) -> Self {
        self.finish_handlers_after_client_close = finish;
        self
    }

    /// The most rooms one socket may be in, the room named after its own id included. A join that would pass it
    /// fails with `RoomError::TooManyRooms` and joins none of its rooms. Default 1,000, above zero. Node has no limit.
    pub fn max_rooms_per_socket(mut self, rooms: usize) -> Self {
        self.max_rooms_per_socket = rooms;
        self
    }

    /// The longest room name, in bytes, in `join`, `to`, `within` and `except`. A join with a longer name fails, and
    /// a broadcast operator given one fails every operation without sending anything (the name is never dropped:
    /// dropping it could widen the broadcast). Default 256, at least 20 (a socket id must fit). Node accepts any
    /// value.
    pub fn max_room_name_length(mut self, bytes: usize) -> Self {
        self.max_room_name_length = bytes;
        self
    }

    /// What the answers collected by one broadcast with acknowledgements may cost, in the decoded-size model of
    /// [`max_decoded_bytes`](Self::max_decoded_bytes). Once an answer would pass it the answers that arrived are
    /// kept, the rest are dropped and the call fails with `BroadcastAckErrorKind::ResponsesTooLarge`. Default 16
    /// MiB, above zero. Node has no limit.
    pub fn max_broadcast_ack_bytes(mut self, bytes: usize) -> Self {
        self.max_broadcast_ack_bytes = bytes;
        self
    }

    /// How long one call of the adapter may take when an operator makes it (`emit`, `fetch_sockets`, `sockets_join`,
    /// `sockets_leave`, `disconnect_sockets`) and when the adapter is initialised, before the call fails with
    /// `EmitError::Adapter(AdapterError::Timeout)`. An adapter that never answers cannot hang its caller. The
    /// in-memory adapter finishes at once; a cluster adapter needs a value above its own round trip. A broadcast with
    /// acknowledgements is bounded by its own deadline. Default 5 s, above zero and at most one day. Node has no
    /// limit.
    pub fn adapter_timeout(mut self, timeout: Duration) -> Self {
        self.adapter_timeout = timeout;
        self
    }

    /// The adapter of every namespace: `factory` is called once per namespace, when it is registered, with a handle to
    /// the namespace's local sockets (Node's `io.adapter(Adapter)`). The default is the in-memory adapter.
    ///
    /// Unstable and hidden until the Redis adapter (#36) implements the trait once.
    #[doc(hidden)]
    pub fn adapter<A, F>(mut self, factory: F) -> Self
    where
        A: Adapter,
        F: Fn(LocalNamespace) -> A + Send + Sync + 'static,
    {
        self.adapter = AdapterFactory::new(move |local| Arc::new(factory(local)) as Arc<dyn Adapter>);
        self
    }

    /// The least a session's address must be allowed to hold: one maximum packet, a decoded packet (`max_decoded_bytes`)
    /// next to the wire bytes of the largest packet (`max_attachments` attachments of `max_http_buffer_size`, or the one
    /// text message), each with its accounting overhead.
    pub fn address_share_floor(&self) -> usize {
        let message = self.engine.max_http_buffer_size.saturating_add(super::engine::session::UNREAD_MESSAGE_OVERHEAD);
        self.max_decoded_bytes.saturating_add(message.saturating_mul(self.max_attachments.max(1))).min(self.engine.max_total_unread_bytes)
    }

    /// The most the answers held by calls with acknowledgements may total, server-wide: a quarter of the engine's
    /// `max_total_unread_bytes`, never less than one call may hold (`max_broadcast_ack_bytes`).
    pub(crate) fn ack_budget(&self) -> usize {
        (self.engine.max_total_unread_bytes / 4).max(self.max_broadcast_ack_bytes)
    }

    /// The share of the server-wide unread budget one address may hold: `max_address_share`, or 1/16 of
    /// `max_total_unread_bytes` raised to [`address_share_floor`](Self::address_share_floor).
    pub fn address_share(&self) -> usize {
        self.max_address_share
            .unwrap_or_else(|| (self.engine.max_total_unread_bytes / DEFAULT_ADDRESS_SHARE_DIVISOR).max(self.address_share_floor()))
    }

    /// Checks the options; the server does this when it is created.
    pub fn validate(&self) -> Result<(), IoOptionsError> {
        self.engine.validate()?;
        let durations = [
            ("connect_timeout", self.connect_timeout),
            ("ack_timeout", self.ack_timeout),
            ("attachment_timeout", self.attachment_timeout),
            ("middleware_timeout", self.middleware_timeout),
            ("admission_timeout", self.admission_timeout),
            ("adapter_timeout", self.adapter_timeout),
        ];
        for (option, value) in durations {
            if value.is_zero() {
                return Err(IoOptionsError::Zero { option });
            }
            if value > MAX_DURATION {
                return Err(IoOptionsError::Range { option, reason: format!("must be at most one day, got {value:?}") });
            }
        }
        let sizes = [
            ("max_attachments", self.max_attachments),
            ("max_inflight", self.max_inflight),
            ("max_pending_acks", self.max_pending_acks),
            ("max_decoded_bytes", self.max_decoded_bytes),
            ("max_auth_bytes", self.max_auth_bytes),
            ("max_event_name_len", self.max_event_name_len),
            ("max_args", self.max_args),
            ("max_rooms_per_socket", self.max_rooms_per_socket),
            ("max_room_name_length", self.max_room_name_length),
            ("max_broadcast_ack_bytes", self.max_broadcast_ack_bytes),
        ];
        for (option, value) in sizes {
            if value == 0 {
                return Err(IoOptionsError::Zero { option });
            }
        }
        if self.max_room_name_length < MIN_ROOM_NAME_LENGTH {
            return Err(IoOptionsError::Range {
                option: "max_room_name_length",
                reason: format!("must be at least {MIN_ROOM_NAME_LENGTH} (a socket id must fit), got {}", self.max_room_name_length),
            });
        }
        // One maximal binary packet (a header and `max_attachments` attachments) must fit in the unread budget, or
        // a reassembly could pause the reads it needs to complete.
        let packet = self.max_attachments.saturating_add(1).saturating_mul(self.engine.max_http_buffer_size);
        if packet > self.engine.max_unread_bytes {
            return Err(IoOptionsError::Range {
                option: "max_attachments",
                reason: format!(
                    "allows a binary packet of {packet} bytes ((max_attachments + 1) x max_http_buffer_size), above the engine's max_unread_bytes ({})",
                    self.engine.max_unread_bytes
                ),
            });
        }
        if self.max_decoded_bytes < self.engine.max_http_buffer_size {
            return Err(IoOptionsError::Range {
                option: "max_decoded_bytes",
                reason: format!(
                    "must be at least the engine's max_http_buffer_size ({}), got {}",
                    self.engine.max_http_buffer_size, self.max_decoded_bytes
                ),
            });
        }
        if self.max_decoded_bytes > self.engine.max_total_unread_bytes {
            return Err(IoOptionsError::Range {
                option: "max_decoded_bytes",
                reason: format!(
                    "must be at most the engine's max_total_unread_bytes ({}), got {}: a packet could never be reserved",
                    self.engine.max_total_unread_bytes, self.max_decoded_bytes
                ),
            });
        }
        if self.max_auth_bytes > self.max_decoded_bytes {
            return Err(IoOptionsError::Range {
                option: "max_auth_bytes",
                reason: format!("must be at most max_decoded_bytes ({}), got {}", self.max_decoded_bytes, self.max_auth_bytes),
            });
        }
        if let Some(share) = self.max_address_share {
            if share == 0 {
                return Err(IoOptionsError::Zero { option: "max_address_share" });
            }
            let floor = self.address_share_floor();
            if share < floor {
                return Err(IoOptionsError::Range {
                    option: "max_address_share",
                    reason: format!(
                        "must be at least one maximum packet ({floor} bytes: max_decoded_bytes plus the wire bytes of the largest packet), got {share}"
                    ),
                });
            }
            if share > self.engine.max_total_unread_bytes {
                return Err(IoOptionsError::Range {
                    option: "max_address_share",
                    reason: format!(
                        "must be at most the engine's max_total_unread_bytes ({}), got {share}",
                        self.engine.max_total_unread_bytes
                    ),
                });
            }
        }
        Ok(())
    }

    /// The Engine.IO options this layer runs on: the application's, with the address share and the pause grace this
    /// layer's limits define.
    pub(crate) fn effective_engine_options(&self) -> EngineOptions {
        self.engine
            .clone()
            .max_address_unread_bytes(self.address_share())
            .pressure_pause_timeout(self.admission_timeout)
            .max_lingering_per_ip(self.max_lingering_per_ip())
    }

    /// The most handlers that may outlive their connection for one client address: what the sessions an address may
    /// have open can run (`max_sessions_per_ip x max_inflight`). New sessions from an address at it are refused with
    /// `503` until some finish.
    pub fn max_lingering_per_ip(&self) -> usize {
        self.engine.max_sessions_per_ip.saturating_mul(self.max_inflight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_ones_and_valid() {
        let o = IoOptions::default();
        assert_eq!(o.connect_timeout, Duration::from_millis(45_000));
        assert_eq!(o.ack_timeout, Duration::from_secs(30));
        assert_eq!(o.max_attachments, 10);
        assert_eq!(o.max_inflight, 32);
        assert_eq!(o.max_pending_acks, 1_000);
        assert_eq!(o.max_decoded_bytes, 32 * 1024 * 1024);
        assert_eq!(o.max_auth_bytes, 16 * 1024);
        assert_eq!(o.max_event_name_len, 256);
        assert_eq!(o.max_args, 128);
        assert_eq!(o.attachment_timeout, Duration::from_secs(30));
        assert_eq!(o.middleware_timeout, Duration::from_secs(10));
        assert_eq!(o.max_rooms_per_socket, 1_000);
        assert_eq!(o.max_room_name_length, 256);
        assert_eq!(o.max_broadcast_ack_bytes, 16 * 1024 * 1024);
        assert_eq!(o.adapter_timeout, Duration::from_secs(5));
        assert!(o.validate().is_ok());
    }

    #[test]
    fn a_room_name_length_shorter_than_a_socket_id_is_refused() {
        let err = IoOptions::new().max_room_name_length(19).validate().unwrap_err();
        assert!(matches!(err, IoOptionsError::Range { option: "max_room_name_length", .. }), "{err}");
        assert!(IoOptions::new().max_room_name_length(20).validate().is_ok());
        assert!(IoOptions::new().max_rooms_per_socket(1).validate().is_ok());
    }

    #[test]
    fn every_zero_value_is_refused_by_name() {
        let cases: Vec<(&str, IoOptions)> = vec![
            ("connect_timeout", IoOptions::new().connect_timeout(Duration::ZERO)),
            ("ack_timeout", IoOptions::new().ack_timeout(Duration::ZERO)),
            ("attachment_timeout", IoOptions::new().attachment_timeout(Duration::ZERO)),
            ("middleware_timeout", IoOptions::new().middleware_timeout(Duration::ZERO)),
            ("adapter_timeout", IoOptions::new().adapter_timeout(Duration::ZERO)),
            ("max_attachments", IoOptions::new().max_attachments(0)),
            ("max_inflight", IoOptions::new().max_inflight(0)),
            ("max_pending_acks", IoOptions::new().max_pending_acks(0)),
            ("max_decoded_bytes", IoOptions::new().max_decoded_bytes(0)),
            ("max_auth_bytes", IoOptions::new().max_auth_bytes(0)),
            ("max_event_name_len", IoOptions::new().max_event_name_len(0)),
            ("max_args", IoOptions::new().max_args(0)),
            ("max_rooms_per_socket", IoOptions::new().max_rooms_per_socket(0)),
            ("max_room_name_length", IoOptions::new().max_room_name_length(0)),
            ("max_broadcast_ack_bytes", IoOptions::new().max_broadcast_ack_bytes(0)),
        ];
        for (name, options) in cases {
            let err = options.validate().unwrap_err();
            assert_eq!(err, IoOptionsError::Zero { option: name });
            assert!(err.to_string().contains(name), "{err}");
        }
    }

    #[test]
    fn durations_above_a_day_are_refused_by_name() {
        for (name, options) in [
            ("connect_timeout", IoOptions::new().connect_timeout(Duration::MAX)),
            ("ack_timeout", IoOptions::new().ack_timeout(Duration::from_secs(2 * 24 * 3600))),
            ("attachment_timeout", IoOptions::new().attachment_timeout(Duration::MAX)),
            ("middleware_timeout", IoOptions::new().middleware_timeout(Duration::MAX)),
            ("adapter_timeout", IoOptions::new().adapter_timeout(Duration::MAX)),
        ] {
            let err = options.validate().unwrap_err();
            assert!(matches!(err, IoOptionsError::Range { option, .. } if option == name), "{name}: {err}");
        }
    }

    #[test]
    fn a_binary_packet_that_cannot_fit_in_the_unread_budget_is_refused() {
        let err = IoOptions::new().max_attachments(100).validate().unwrap_err();
        assert!(matches!(err, IoOptionsError::Range { option: "max_attachments", .. }), "{err}");
        assert!(err.to_string().contains("max_attachments") && err.to_string().contains("max_unread_bytes"));
        // The product does not overflow.
        let huge = IoOptions::new().max_attachments(usize::MAX);
        assert!(matches!(huge.validate().unwrap_err(), IoOptionsError::Range { option: "max_attachments", .. }));
        // 10 attachments of the default 1,000,000 bytes (11 MB) fit in 16 MiB; 15 do not.
        assert!(IoOptions::new().max_attachments(15).validate().is_ok());
        assert!(IoOptions::new().max_attachments(16).validate().is_err());
        // Bigger budget, more attachments.
        let engine = EngineOptions::new().max_unread_bytes(64 * 1024 * 1024);
        assert!(IoOptions::new().engine(engine).max_attachments(60).validate().is_ok());
    }

    #[test]
    fn decoded_size_limits_are_consistent_with_the_engine() {
        let err = IoOptions::new().max_decoded_bytes(999_999).validate().unwrap_err();
        assert!(matches!(err, IoOptionsError::Range { option: "max_decoded_bytes", .. }), "{err}");
        let err = IoOptions::new().max_auth_bytes(33 * 1024 * 1024).validate().unwrap_err();
        assert!(matches!(err, IoOptionsError::Range { option: "max_auth_bytes", .. }), "{err}");
    }

    #[test]
    fn one_decoded_packet_must_fit_in_the_server_wide_budget() {
        // The default packet (32 MiB) needs a budget of at least 32 MiB.
        let small = EngineOptions::new().max_total_unread_bytes(16 * 1024 * 1024);
        let err = IoOptions::new().engine(small.clone()).validate().unwrap_err();
        assert!(matches!(err, IoOptionsError::Range { option: "max_decoded_bytes", .. }), "{err}");
        assert!(err.to_string().contains("max_total_unread_bytes"));
        assert!(IoOptions::new().engine(small).max_decoded_bytes(16 * 1024 * 1024).validate().is_ok());
        let exact = EngineOptions::new().max_total_unread_bytes(32 * 1024 * 1024);
        assert!(IoOptions::new().engine(exact).validate().is_ok());
    }

    #[test]
    fn the_address_share_defaults_to_a_sixteenth_of_the_budget_but_never_less_than_one_maximum_packet() {
        // One maximum packet: the decoded packet and the wire bytes of the largest binary packet, 10 attachments of 1 MB
        // with 64 bytes of accounting each: with the defaults that is the larger number, 32 MiB + 10,000,640 bytes.
        let defaults = IoOptions::new();
        assert_eq!(defaults.address_share_floor(), 32 * 1024 * 1024 + 10 * (1_000_000 + 64));
        assert_eq!(defaults.address_share(), defaults.address_share_floor(), "a sixteenth of 256 MiB (16 MiB) is below it");
        // With a budget large enough the sixteenth wins.
        let big = EngineOptions::new().max_total_unread_bytes(2 * 1024 * 1024 * 1024);
        assert_eq!(IoOptions::new().engine(big).address_share(), 128 * 1024 * 1024);
        // The floor follows the packet limits and never exceeds the budget.
        let small = IoOptions::new()
            .max_decoded_bytes(4 * 1024 * 1024)
            .max_attachments(1)
            .engine(EngineOptions::new().max_total_unread_bytes(64 * 1024 * 1024));
        assert_eq!(small.address_share(), 4 * 1024 * 1024 + 1_000_064);
        let exact = EngineOptions::new().max_total_unread_bytes(32 * 1024 * 1024);
        assert_eq!(IoOptions::new().engine(exact).address_share(), 32 * 1024 * 1024);
        // The share is the engine option the layer sets, and the pause grace is the admission timeout.
        let engine = defaults.effective_engine_options();
        assert_eq!(engine.max_address_unread_bytes, Some(defaults.address_share()));
        assert_eq!(engine.pressure_pause_timeout, Some(DEFAULT_ADMISSION_TIMEOUT));
        assert!(engine.validate().is_ok());
    }

    #[test]
    fn an_explicit_address_share_must_hold_one_maximum_packet_and_fit_the_budget() {
        let options = IoOptions::new()
            .max_decoded_bytes(4 * 1024 * 1024)
            .max_attachments(1)
            .engine(EngineOptions::new().max_total_unread_bytes(64 * 1024 * 1024));
        let floor = options.address_share_floor();
        let err = options.clone().max_address_share(floor - 1).validate().unwrap_err();
        assert!(matches!(err, IoOptionsError::Range { option: "max_address_share", .. }), "{err}");
        assert!(err.to_string().contains("max_address_share") && err.to_string().contains("one maximum packet"));
        assert!(options.clone().max_address_share(floor).validate().is_ok());
        assert_eq!(options.clone().max_address_share(floor).address_share(), floor);
        let total = options.engine.max_total_unread_bytes;
        assert!(options.clone().max_address_share(total).validate().is_ok());
        let err = options.clone().max_address_share(total + 1).validate().unwrap_err();
        assert!(matches!(err, IoOptionsError::Range { option: "max_address_share", .. }), "{err}");
        assert_eq!(options.max_address_share(0).validate().unwrap_err(), IoOptionsError::Zero { option: "max_address_share" });
    }

    #[test]
    fn the_admission_timeout_defaults_to_30_seconds_and_is_validated() {
        assert_eq!(IoOptions::new().admission_timeout, Duration::from_secs(30));
        assert_eq!(
            IoOptions::new().admission_timeout(Duration::ZERO).validate().unwrap_err(),
            IoOptionsError::Zero { option: "admission_timeout" }
        );
        let err = IoOptions::new().admission_timeout(Duration::from_secs(2 * 24 * 3600)).validate().unwrap_err();
        assert!(matches!(err, IoOptionsError::Range { option: "admission_timeout", .. }), "{err}");
        assert!(IoOptions::new().admission_timeout(Duration::from_secs(1)).validate().is_ok());
    }

    #[test]
    fn the_engine_options_are_validated_too() {
        let err = IoOptions::new().engine(EngineOptions::new().max_sessions(0)).validate().unwrap_err();
        assert_eq!(err, IoOptionsError::Engine(EngineOptionsError::Zero { option: "max_sessions" }));
        assert!(err.to_string().contains("max_sessions"));
    }
}
