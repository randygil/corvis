//! The long-polling transport: `GET` waits for data, `POST` delivers it, and the error answers of Node's
//! `engine.io` (`abortRequest`, `abortUpgrade`).

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, StatusCode};
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use tokio::time::Instant;

use super::packet::{Incoming as Decoded, Packet, decode_payload, encode_payload};
use super::session::{CloseReason, Polled, Session};
use crate::server::{PendingBody, ResBody};

/// The messages of Node's `Server.errorMessages`.
pub(super) fn error_message(code: u8) -> &'static str {
    match code {
        0 => "Transport unknown",
        1 => "Session ID unknown",
        2 => "Bad handshake method",
        3 => "Bad request",
        4 => "Forbidden",
        _ => "Unsupported protocol version",
    }
}

fn body(data: impl Into<Bytes>) -> ResBody {
    use http_body_util::Full;
    Full::new(data.into()).map_err(|never: std::convert::Infallible| match never {}).boxed_unsync()
}

/// A response without headers or body, like Node's `res.writeHead(status).end()`.
pub(super) fn empty(status: StatusCode) -> hyper::Response<ResBody> {
    hyper::Response::builder().status(status).body(body(Bytes::new())).expect("a valid response")
}

/// `{"code":<n>,"message":<text>}` with `Content-Type: application/json`.
pub(super) fn json_error(status: StatusCode, code: u8, message: &str) -> hyper::Response<ResBody> {
    let text = serde_json::json!({ "code": code, "message": message }).to_string();
    hyper::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(body(text))
        .expect("a valid response")
}

/// Node's `abortRequest`: `403` for code 4, `400` otherwise.
pub(super) fn abort_request(code: u8) -> hyper::Response<ResBody> {
    let status = if code == 4 { StatusCode::FORBIDDEN } else { StatusCode::BAD_REQUEST };
    json_error(status, code, error_message(code))
}

/// Node's `abortUpgrade`: a `400 Bad Request` with the message as `text/html`, then the connection is closed.
pub(super) fn abort_upgrade(code: u8) -> hyper::Response<ResBody> {
    let message = error_message(code);
    hyper::Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(http::header::CONNECTION, "close")
        .header(http::header::CONTENT_TYPE, "text/html")
        .body(body(message))
        .expect("a valid response")
}

fn with_headers(status: StatusCode, content_type: &'static str, payload: String) -> hyper::Response<ResBody> {
    hyper::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, content_type)
        .header(http::header::CACHE_CONTROL, "no-store")
        .body(body(payload))
        .expect("a valid response")
}

/// A polling `GET`: waits until there is something to say.
pub(super) async fn get(session: &Arc<Session>) -> hyper::Response<ResBody> {
    let payload = match session.poll().await {
        Polled::Overlap => return empty(StatusCode::BAD_REQUEST),
        Polled::Packets(packets) => encode_payload(&packets),
        Polled::Noop => encode_payload(&[Packet::Noop]),
        Polled::Close => encode_payload(&[Packet::Close]),
    };
    with_headers(StatusCode::OK, "text/plain; charset=UTF-8", payload)
}

fn is_octet_stream(headers: &HeaderMap) -> bool {
    headers.get(http::header::CONTENT_TYPE).is_some_and(|v| v.as_bytes() == b"application/octet-stream")
}

/// A polling `POST`: reads at most `max_http_buffer_size` bytes within `request_timeout`, hands the packets to the
/// session and answers `ok`.
pub(super) async fn post(
    session: &Arc<Session>,
    headers: &HeaderMap,
    body_in: Incoming,
    request_timeout: Option<Duration>,
) -> hyper::Response<ResBody> {
    let Some(guard) = session.begin_post() else { return empty(StatusCode::BAD_REQUEST) };
    // v4 has no binary `POST` bodies.
    if is_octet_stream(headers) {
        session.close(CloseReason::TransportError);
        return empty(StatusCode::BAD_REQUEST);
    }
    let cap = session.options.max_http_buffer_size;
    let deadline = request_timeout.and_then(|t| Instant::now().checked_add(t));
    if crate::server::content_length(headers).is_some_and(|n| n > cap as u64) {
        // Not read: at most `cap` more bytes are discarded in the background so the client can read the answer.
        tokio::spawn(crate::server::drain(PendingBody { body: body_in, deadline }, cap as u64));
        guard.finish();
        return empty(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let read = Limited::new(body_in, cap).collect();
    let result = match deadline {
        Some(d) => tokio::time::timeout_at(d, read).await.ok(),
        None => Some(read.await),
    };
    let bytes = match result {
        Some(Ok(collected)) => collected.to_bytes(),
        Some(Err(e)) if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() => {
            guard.finish();
            return empty(StatusCode::PAYLOAD_TOO_LARGE);
        }
        // The client went away mid-body: the guard drops unfinished and closes the session, as Node does.
        Some(Err(_)) => return empty(StatusCode::BAD_REQUEST),
        None => {
            guard.finish();
            return empty(StatusCode::REQUEST_TIMEOUT);
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let decoded = match decode_payload(&text, session.options.max_payload_packets) {
        Ok(decoded) => decoded,
        Err(_) => {
            // Refused as a whole, before any packet reaches the application.
            session.close(CloseReason::ParseError);
            guard.finish();
            return json_error(StatusCode::BAD_REQUEST, 3, "Too many packets");
        }
    };
    drop(text);
    for item in decoded {
        match item {
            Decoded::Packet(Packet::Close) => {
                session.close(CloseReason::TransportClose);
                break;
            }
            Decoded::Packet(packet) => session.on_packet(packet),
            Decoded::Invalid => {
                session.on_invalid();
                break;
            }
        }
    }
    // Received data waits within a bound: the answer is held until the application takes some of it.
    let mut closed = session.closed_watch();
    loop {
        let notified = session.unread.notify.notified();
        if !session.unread.over_limit() || session.is_closed() {
            break;
        }
        tokio::select! {
            _ = notified => {}
            _ = closed.changed() => {}
        }
    }
    guard.finish();
    let mut resp = with_headers(StatusCode::OK, "text/html", "ok".to_string());
    resp.headers_mut().insert(http::header::CONTENT_LENGTH, HeaderValue::from_static("2"));
    resp
}
