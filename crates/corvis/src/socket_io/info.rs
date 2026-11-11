//! A socket as plain data: what `fetch_sockets` returns and what a remote adapter sends for a socket of another process.

use serde::{Deserialize, Serialize};

use super::adapter::Room;
use super::socket::Handshake;

/// A handshake as plain data, the form that crosses processes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PlainHandshake {
    auth: serde_json::Value,
    headers: Vec<(String, String)>,
    url: String,
    address: String,
    issued_at: u64,
}

#[derive(Clone)]
enum HandshakeRepr {
    /// A socket of this process: shares its handshake, copies nothing.
    Local(Handshake),
    /// A socket described by another process.
    Plain(Box<PlainHandshake>),
}

/// What a client sent when it connected (`socket.handshake`). For a socket of this process it shares the socket's own
/// handshake instead of copying it, and that handshake shares the request its Engine.IO session keeps once for all the
/// sockets of the connection (see [`Handshake`](super::Handshake)); it serializes as plain data so another process can
/// describe a socket it does not hold.
///
/// `Debug` hides credentials: it prints the names of the headers and of the `auth` keys, never their values, and the
/// URL without its query.
#[derive(Clone)]
pub struct HandshakeInfo {
    repr: HandshakeRepr,
}

/// The only headers whose values `Debug` prints: ones that carry no credential. Every other header, whatever its
/// name (`x-api-key`, `x-auth-token`, a custom one), prints as `<hidden>`.
const SHOWN_HEADERS: [&str; 9] =
    ["host", "user-agent", "origin", "accept", "accept-encoding", "accept-language", "connection", "upgrade", "sec-websocket-version"];

impl HandshakeInfo {
    /// A handshake described by plain data (what a remote adapter builds from what another process sent).
    pub fn new(auth: serde_json::Value, headers: Vec<(String, String)>, url: String, address: String, issued_at: u64) -> Self {
        HandshakeInfo { repr: HandshakeRepr::Plain(Box::new(PlainHandshake { auth, headers, url, address, issued_at })) }
    }

    pub(crate) fn local(handshake: Handshake) -> Self {
        HandshakeInfo { repr: HandshakeRepr::Local(handshake) }
    }

    /// The `auth` object of the CONNECT packet.
    pub fn auth(&self) -> &serde_json::Value {
        match &self.repr {
            HandshakeRepr::Local(h) => h.auth(),
            HandshakeRepr::Plain(p) => &p.auth,
        }
    }

    /// The headers of the request that opened the connection (those that are valid text), in order.
    pub fn headers(&self) -> Vec<(&str, &str)> {
        match &self.repr {
            HandshakeRepr::Local(h) => h.headers().iter().filter_map(|(k, v)| Some((k.as_str(), v.to_str().ok()?))).collect(),
            HandshakeRepr::Plain(p) => p.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect(),
        }
    }

    /// The URL of the request, query included.
    pub fn url(&self) -> String {
        match &self.repr {
            HandshakeRepr::Local(h) => h.request().url(),
            HandshakeRepr::Plain(p) => p.url.clone(),
        }
    }

    /// The client address.
    pub fn address(&self) -> String {
        match &self.repr {
            HandshakeRepr::Local(h) => h.ip(),
            HandshakeRepr::Plain(p) => p.address.clone(),
        }
    }

    /// Milliseconds since the Unix epoch.
    pub fn issued_at(&self) -> u64 {
        match &self.repr {
            HandshakeRepr::Local(h) => h.issued_at().duration_since(std::time::SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            HandshakeRepr::Plain(p) => p.issued_at,
        }
    }

    fn to_plain(&self) -> PlainHandshake {
        PlainHandshake {
            auth: self.auth().clone(),
            headers: self.headers().into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            url: self.url(),
            address: self.address(),
            issued_at: self.issued_at(),
        }
    }
}

impl PartialEq for HandshakeInfo {
    fn eq(&self, other: &Self) -> bool {
        self.to_plain() == other.to_plain()
    }
}

impl Serialize for HandshakeInfo {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_plain().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HandshakeInfo {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(HandshakeInfo { repr: HandshakeRepr::Plain(Box::new(PlainHandshake::deserialize(deserializer)?)) })
    }
}

impl std::fmt::Debug for HandshakeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let auth_keys: Vec<&str> = match self.auth() {
            serde_json::Value::Object(map) => map.keys().map(String::as_str).collect(),
            _ => Vec::new(),
        };
        let headers: Vec<String> = self
            .headers()
            .into_iter()
            .map(|(name, value)| {
                if SHOWN_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                    format!("{name}: {value}")
                } else {
                    format!("{name}: <hidden>")
                }
            })
            .collect();
        let url = self.url();
        let url = match url.split_once('?') {
            Some((path, _)) => format!("{path}?<hidden>"),
            None => url,
        };
        f.debug_struct("HandshakeInfo")
            .field("auth_keys", &auth_keys)
            .field("headers", &headers)
            .field("url", &url)
            .field("address", &self.address())
            .field("issued_at", &self.issued_at())
            .finish()
    }
}

/// Rooms as a plain list of names on the wire; in memory each is a shared string.
mod rooms_serde {
    use super::Room;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(rooms: &[Room], serializer: S) -> Result<S::Ok, S::Error> {
        let names: Vec<&str> = rooms.iter().map(|r| &**r).collect();
        names.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Room>, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        Ok(names.into_iter().map(|n| Room::from(n.as_str())).collect())
    }
}

/// A socket described by data: what `fetch_sockets` returns, whichever process holds the socket. For a socket of this
/// process nothing big is copied: the handshake is shared, and the rooms are shared names (16 bytes each, at most
/// `max_rooms_per_socket`). `data` is a copy of the application's own `socket.data`.
///
/// `Debug` hides credentials (see [`HandshakeInfo`]).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SocketInfo {
    pub id: String,
    pub handshake: HandshakeInfo,
    #[serde(with = "rooms_serde")]
    pub rooms: Vec<Room>,
    pub data: serde_json::Value,
}

impl std::fmt::Debug for SocketInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SocketInfo")
            .field("id", &self.id)
            .field("handshake", &self.handshake)
            .field("rooms", &self.rooms.len())
            .finish_non_exhaustive()
    }
}
