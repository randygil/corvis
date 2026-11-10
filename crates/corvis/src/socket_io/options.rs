//! Options of the Socket.IO server: Node's `socket.io` defaults where there is a Node default, and a bounded default
//! for everything a client can make the server hold.

use std::time::Duration;

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
/// What one packet may cost once decoded (see [`IoOptions::max_decoded_bytes`]): 16 MiB.
pub const DEFAULT_MAX_DECODED_BYTES: usize = 16 * 1024 * 1024;
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
/// | [`max_decoded_bytes`](Self::max_decoded_bytes) | 16 MiB per packet | none |
/// | [`max_auth_bytes`](Self::max_auth_bytes) | 16 KiB | none |
/// | [`max_event_name_len`](Self::max_event_name_len) | 256 bytes | none |
/// | [`max_args`](Self::max_args) | 128 | none |
/// | [`attachment_timeout`](Self::attachment_timeout) | 30 s | none |
/// | [`middleware_timeout`](Self::middleware_timeout) | 10 s | none |
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

    /// The most a decoded packet may cost in memory, counting every JSON value at its real size (an empty array
    /// costs as much as a small number: 32 bytes) plus the text of strings and keys. A packet above it closes the
    /// connection while it is being decoded, so the structure is never built. A decoded packet is charged against
    /// the session's `max_unread_bytes` until its handler finishes. Must be at least `max_http_buffer_size`. Node
    /// has no limit.
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

    /// Checks the options; the server does this when it is created.
    pub fn validate(&self) -> Result<(), IoOptionsError> {
        self.engine.validate()?;
        let durations = [
            ("connect_timeout", self.connect_timeout),
            ("ack_timeout", self.ack_timeout),
            ("attachment_timeout", self.attachment_timeout),
            ("middleware_timeout", self.middleware_timeout),
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
        ];
        for (option, value) in sizes {
            if value == 0 {
                return Err(IoOptionsError::Zero { option });
            }
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
        if self.max_auth_bytes > self.max_decoded_bytes {
            return Err(IoOptionsError::Range {
                option: "max_auth_bytes",
                reason: format!("must be at most max_decoded_bytes ({}), got {}", self.max_decoded_bytes, self.max_auth_bytes),
            });
        }
        Ok(())
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
        assert_eq!(o.max_decoded_bytes, 16 * 1024 * 1024);
        assert_eq!(o.max_auth_bytes, 16 * 1024);
        assert_eq!(o.max_event_name_len, 256);
        assert_eq!(o.max_args, 128);
        assert_eq!(o.attachment_timeout, Duration::from_secs(30));
        assert_eq!(o.middleware_timeout, Duration::from_secs(10));
        assert!(o.validate().is_ok());
    }

    #[test]
    fn every_zero_value_is_refused_by_name() {
        let cases: Vec<(&str, IoOptions)> = vec![
            ("connect_timeout", IoOptions::new().connect_timeout(Duration::ZERO)),
            ("ack_timeout", IoOptions::new().ack_timeout(Duration::ZERO)),
            ("attachment_timeout", IoOptions::new().attachment_timeout(Duration::ZERO)),
            ("middleware_timeout", IoOptions::new().middleware_timeout(Duration::ZERO)),
            ("max_attachments", IoOptions::new().max_attachments(0)),
            ("max_inflight", IoOptions::new().max_inflight(0)),
            ("max_pending_acks", IoOptions::new().max_pending_acks(0)),
            ("max_decoded_bytes", IoOptions::new().max_decoded_bytes(0)),
            ("max_auth_bytes", IoOptions::new().max_auth_bytes(0)),
            ("max_event_name_len", IoOptions::new().max_event_name_len(0)),
            ("max_args", IoOptions::new().max_args(0)),
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
        let err = IoOptions::new().max_auth_bytes(32 * 1024 * 1024).validate().unwrap_err();
        assert!(matches!(err, IoOptionsError::Range { option: "max_auth_bytes", .. }), "{err}");
    }

    #[test]
    fn the_engine_options_are_validated_too() {
        let err = IoOptions::new().engine(EngineOptions::new().max_sessions(0)).validate().unwrap_err();
        assert_eq!(err, IoOptionsError::Engine(EngineOptionsError::Zero { option: "max_sessions" }));
        assert!(err.to_string().contains("max_sessions"));
    }
}
