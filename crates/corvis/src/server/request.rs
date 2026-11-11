//! From the request hyper read to the request the router dispatches: the body limits, the pending body, the drain of
//! a body nobody read, and `respond`, the request path shared by the server and the test client.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, LengthLimitError, Limited};
use tokio::time::Instant;

use crate::http::{HttpException, Request, Response};
use crate::router::{AppShared, dispatch};

use super::response::finish_response;
use super::{ResBody, ServerOptions, ServerTasks};

/// The body of a request as the server keeps it unread: hyper's body over a socket, an in-memory body in the test
/// client.
pub(crate) type RequestBody = UnsyncBoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

/// A request body that has not been read yet (multipart bodies: only a file interceptor, after
/// the guards, reads them; see [`crate::multer::read_body`]).
pub(crate) struct PendingBody {
    pub body: RequestBody,
    /// `requestTimeout` deadline of the request.
    pub deadline: Option<Instant>,
}

impl PendingBody {
    pub(crate) fn new<B>(body: B, deadline: Option<Instant>) -> Self
    where
        B: hyper::body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        PendingBody { body: body.map_err(Into::into).boxed_unsync(), deadline }
    }
}

/// Request extension holding the unread body. Shared by all the clones of the request; the
/// first reader takes it.
#[derive(Clone)]
pub(crate) struct PendingBodySlot(pub Arc<Mutex<Option<PendingBody>>>);

impl PendingBodySlot {
    pub(crate) fn take(req: &Request) -> Option<PendingBody> {
        let slot = req.get::<PendingBodySlot>()?;
        let mut guard = slot.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.take()
    }
}

/// `408` of `server.requestTimeout`.
pub(crate) fn request_timeout_error() -> HttpException {
    HttpException::http_error(408, "Request Timeout")
}

/// A body that could not be read (client aborted, malformed chunked encoding, ...).
pub(crate) fn request_aborted_error() -> HttpException {
    HttpException::http_error(400, "request aborted")
}

pub(crate) fn entity_too_large_error() -> HttpException {
    HttpException::http_error(413, "request entity too large")
}

/// `Content-Length` of the request, when present and valid.
pub(crate) fn content_length(headers: &http::HeaderMap) -> Option<u64> {
    headers.get(http::header::CONTENT_LENGTH)?.to_str().ok()?.trim().parse().ok()
}

/// How many bytes of a non-multipart body are read: the limit of the body parser that will
/// parse it (JSON / urlencoded), otherwise the transport limit (`set_body_limit`), so that
/// `req.raw_body()` keeps working for other types.
fn read_limit(headers: &http::HeaderMap, transport: usize, parser: &crate::http::body_parser::BodyParserOptions) -> usize {
    if !parser.enabled {
        return transport;
    }
    let identity =
        headers.get(http::header::CONTENT_ENCODING).is_none_or(|v| v.to_str().is_ok_and(|s| s.trim().eq_ignore_ascii_case("identity")));
    if !identity {
        // The parser answers 415 whatever the size: keep the transport limit.
        return transport;
    }
    let Some(ct) = headers.get(http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) else { return transport };
    let mut params = ct.split(';');
    let essence = params.next().unwrap_or("").trim_matches([' ', '\t']).to_ascii_lowercase();
    let charset = params.find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case("charset").then(|| v.trim().trim_matches('"').to_ascii_lowercase())
    });
    let parser_limit = match essence.as_str() {
        "application/json" if charset.as_deref().is_none_or(|c| c.starts_with("utf-")) => parser.json_limit,
        "application/x-www-form-urlencoded" if charset.as_deref().is_none_or(|c| c == "utf-8" || c == "iso-8859-1") => {
            parser.urlencoded_limit
        }
        _ => return transport,
    };
    parser_limit.min(transport)
}

/// Converts a hyper request into a corvis request.
///
/// - `multipart/*` bodies are not read here: the body stays in the request and only a file
///   interceptor reads it, streaming, with its own limits (Express: multer consumes the stream
///   only on routes that use it, after the guards).
/// - Other bodies are read up to the limit of the parser that accepts them (`json` /
///   `urlencoded`) or up to the transport limit (`body_limit`), and within `request_timeout`.
pub(crate) async fn from_hyper<B>(
    req: hyper::Request<B>,
    remote: Option<SocketAddr>,
    limit: usize,
    parser: &crate::http::body_parser::BodyParserOptions,
    request_timeout: Option<Duration>,
) -> Request
where
    B: hyper::body::Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let (parts, body) = req.into_parts();
    let deadline = request_timeout.and_then(|t| Instant::now().checked_add(t));
    if crate::multer::is_multipart_content_type(&parts.headers) {
        let mut r = Request::with_parser(parts.method, parts.uri, parts.headers, Bytes::new(), parser);
        r.version = parts.version;
        r.remote_addr = remote;
        let mut ext = parts.extensions;
        ext.insert(PendingBodySlot(Arc::new(Mutex::new(Some(PendingBody::new(body, deadline))))));
        *r.extensions.write().unwrap() = ext;
        return r;
    }
    let limit = read_limit(&parts.headers, limit, parser);
    let mut unread = None;
    let mut client_aborted = false;
    let (bytes, error) = if content_length(&parts.headers).is_some_and(|n| n > limit as u64) {
        // raw-body: a declared length over the limit is rejected without reading the body
        // (at most a bounded part of it is discarded after the response, see `drain`).
        unread = Some(PendingBody::new(body, deadline));
        (Bytes::new(), Some(entity_too_large_error()))
    } else {
        let read = Limited::new(body, limit).collect();
        let result = match deadline {
            Some(d) => tokio::time::timeout_at(d, read).await.ok(),
            None => Some(read.await),
        };
        match result {
            Some(Ok(c)) => (c.to_bytes(), None),
            Some(Err(e)) if e.downcast_ref::<LengthLimitError>().is_some() => (Bytes::new(), Some(entity_too_large_error())),
            Some(Err(_)) => {
                client_aborted = true;
                (Bytes::new(), Some(request_aborted_error()))
            }
            None => (Bytes::new(), Some(request_timeout_error())),
        }
    };
    // Body-parser errors stay in the request: the router answers them after CORS and through
    // the global filters (Express `next(err)`).
    let mut r = Request::with_parser(parts.method, parts.uri, parts.headers, bytes, parser);
    if let Some(e) = error {
        r.body_error = Some(e);
    }
    r.client_aborted = client_aborted;
    r.version = parts.version;
    r.remote_addr = remote;
    let mut ext = parts.extensions;
    if let Some(p) = unread {
        ext.insert(PendingBodySlot(Arc::new(Mutex::new(Some(p)))));
    }
    *r.extensions.write().unwrap() = ext;
    r
}

/// Discards what is left of a body nobody read, up to `cap` bytes and until its deadline, so a
/// client that already sent it still reads the response and the connection can be reused
/// (Node drains unread bodies too). Past the cap or the deadline the body is dropped and hyper
/// closes the connection: the rest is never read nor kept in memory.
pub(crate) async fn drain(pending: PendingBody, cap: u64) {
    let PendingBody { mut body, deadline } = pending;
    let deadline = deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(30));
    let mut read: u64 = 0;
    while read <= cap {
        match tokio::time::timeout_at(deadline, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    read += data.len() as u64;
                }
            }
            _ => break,
        }
    }
}

/// The request path shared by the server and the test client: the conversion with the body limits (a pending body
/// for multipart and for a declared length over the limit), the dispatch, the drain of a body nobody read, and the
/// response finishing. Sets `client_aborted` when an HTTP/2 client reset the stream while its body was read: such a
/// request is not dispatched.
pub(crate) async fn respond<B>(
    shared: &Arc<AppShared>,
    req: hyper::Request<B>,
    remote: Option<SocketAddr>,
    options: &ServerOptions,
    tasks: &ServerTasks,
    client_aborted: &mut bool,
) -> hyper::Response<ResBody>
where
    B: hyper::body::Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let head = req.method() == http::Method::HEAD;
    let info = crate::http::express::RequestInfo::new(req.method(), req.headers());
    #[cfg(feature = "compression")]
    let info = {
        let mut info = info.with_compression(req.uri().path(), req.headers(), &shared.http_settings);
        info.conn_slots = req.extensions().get::<crate::http::compression::ConnectionSlots>().cloned();
        info
    };
    let r = from_hyper(req, remote, shared.body_limit, &shared.http_settings.body_parser, options.request_timeout).await;
    // An HTTP/2 body that fails to read means the client reset the stream or the connection broke: nobody waits for
    // the answer, so it is not dispatched (and not logged as an error) and counts as a cancelled request.
    let resp = if r.client_aborted && r.version == http::Version::HTTP_2 {
        *client_aborted = true;
        Response::json(request_aborted_error().get_response().clone()).with_status(400)
    } else {
        let pending = r.get::<PendingBodySlot>();
        let resp = dispatch(shared.clone(), r).await;
        // A body nobody read (or left half read by a failed upload) is drained now, not
        // when the last clone of the request goes away.
        if let Some(slot) = pending {
            let unread = slot.0.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(p) = unread {
                tasks.spawn(drain(p, shared.body_limit as u64));
            }
        }
        resp
    };
    finish_response(resp, &info, &shared.http_settings, head)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> http::HeaderMap {
        let mut h = http::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(http::header::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
        }
        h
    }

    #[test]
    fn read_limit_follows_the_parser_that_accepts_the_body() {
        let p = crate::http::body_parser::BodyParserOptions::default();
        let t = 1024 * 1024;
        assert_eq!(read_limit(&headers(&[("content-type", "application/json")]), t, &p), p.json_limit);
        assert_eq!(read_limit(&headers(&[("content-type", "Application/JSON; charset=utf-8")]), t, &p), p.json_limit);
        assert_eq!(read_limit(&headers(&[("content-type", "application/x-www-form-urlencoded")]), t, &p), p.urlencoded_limit);
        // 415 cases keep the transport limit (the parser answers them whatever the size).
        assert_eq!(read_limit(&headers(&[("content-type", "application/json; charset=latin1")]), t, &p), t);
        assert_eq!(read_limit(&headers(&[("content-type", "application/json"), ("content-encoding", "gzip")]), t, &p), t);
        // Other types: raw body up to the transport limit.
        assert_eq!(read_limit(&headers(&[("content-type", "text/plain")]), t, &p), t);
        assert_eq!(read_limit(&headers(&[]), t, &p), t);
        // A transport limit below the parser's wins.
        assert_eq!(read_limit(&headers(&[("content-type", "application/json")]), 10, &p), 10);
    }
}
