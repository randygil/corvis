//! From a handler's response to the response hyper writes.

use std::convert::Infallible;

use bytes::Bytes;
use futures::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Frame;

use crate::http::{Body, Response};

use super::ResBody;

pub(super) fn full(data: impl Into<Bytes>) -> ResBody {
    Full::new(data.into()).map_err(|never: Infallible| match never {}).boxed_unsync()
}

pub(crate) fn empty_body() -> ResBody {
    full(Bytes::new())
}

/// Converts a corvis response into a hyper response.
pub(crate) fn into_hyper(resp: Response, head: bool) -> hyper::Response<ResBody> {
    let status = resp.status.unwrap_or(http::StatusCode::OK);
    let mut builder = hyper::Response::builder().status(status);
    let mut headers = resp.headers;
    for (name, value) in resp.default_headers.iter() {
        if !headers.contains_key(name) {
            headers.insert(name.clone(), value.clone());
        }
    }
    let is_empty = matches!(resp.body, Body::Empty);
    let (content_type, body): (Option<String>, ResBody) = match resp.body {
        Body::Empty => (None, full(Bytes::new())),
        Body::Json(v) => (Some("application/json; charset=utf-8".into()), full(serde_json::to_vec(&v).unwrap_or_default())),
        Body::Text(t) => (Some("text/html; charset=utf-8".into()), full(t)),
        Body::Html(t) => (Some("text/html; charset=utf-8".into()), full(t)),
        Body::Bytes { data, content_type } => (Some(content_type), full(data)),
        Body::Stream { stream, content_type } => {
            let s = stream.map(|r| r.map(Frame::data));
            (Some(content_type), BodyExt::boxed_unsync(StreamBody::new(s)))
        }
    };
    // An empty type (`res.end()` without a `Content-Type`) writes no header, like Node.
    if let Some(ct) = content_type.filter(|ct| !ct.is_empty())
        && !headers.contains_key(http::header::CONTENT_TYPE)
        && let Ok(v) = http::HeaderValue::from_str(&ct)
    {
        headers.insert(http::header::CONTENT_TYPE, v);
    }
    // `X-Powered-By`, ETag, ... are added by `http::express::finalize` (see `handle`).
    let bodyless = status == http::StatusCode::NO_CONTENT || status == http::StatusCode::NOT_MODIFIED || status.is_informational();
    // An empty answer (`res.send()` without a body) sends no length for HEAD.
    if head && !bodyless && !is_empty && !headers.contains_key(http::header::CONTENT_LENGTH) {
        // HEAD keeps the length of the body it would have sent (like Node).
        if let Some(len) = hyper::body::Body::size_hint(&body).exact() {
            headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from(len));
        }
    }
    // A `Content-Length` that disagrees with a body of known size would make the peer read the extra bytes as a
    // second response: the real length wins. (HEAD keeps the length it was given, as Node does.)
    if !head
        && let Some(declared) = headers.get(http::header::CONTENT_LENGTH)
        && let Some(len) = hyper::body::Body::size_hint(&body).exact()
        && declared.to_str().ok().and_then(|d| d.trim().parse::<u64>().ok()) != Some(len)
    {
        headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from(len));
    }
    if let Some(h) = builder.headers_mut() {
        *h = headers;
    }
    let body = if head { full(Bytes::new()) } else { body };
    builder.body(body).unwrap_or_else(|_| hyper::Response::new(full(Bytes::new())))
}

/// Length of the in-memory body of a response before compression (response extension).
#[derive(Clone, Copy)]
pub(crate) struct PlainLen(pub u64);

/// Finishes a handler's response: the Express defaults, the conversion to the wire response and,
/// when the application enabled it, compression. The server and the test client both end here.
pub(super) fn finish_response(
    mut resp: Response,
    info: &crate::http::express::RequestInfo,
    settings: &crate::http::HttpSettings,
    head: bool,
) -> hyper::Response<ResBody> {
    crate::http::express::finalize(&mut resp, info, settings);
    #[allow(unused_mut)]
    let mut wire = into_hyper(resp, head);
    // Compression hides the size of an in-memory body: the server still holds it until the body ends.
    if let Some(len) = hyper::body::Body::size_hint(wire.body()).exact() {
        wire.extensions_mut().insert(PlainLen(len));
    }
    #[cfg(feature = "compression")]
    if let Some(options) = &settings.compression {
        return crate::http::compression::apply(wire, info, options);
    }
    wire
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wrong `Content-Length` set by a handler must not frame a second response on a keep-alive connection.
    #[test]
    fn content_length_always_matches_a_body_of_known_size() {
        let resp = Response::bytes(vec![b'x'; 20], "image/png").with_header("Content-Length", "5");
        let out = into_hyper(resp, false);
        assert_eq!(out.headers().get("content-length").unwrap(), "20");
        let resp = Response::bytes(vec![b'x'; 20], "image/png").with_header("Content-Length", "20");
        assert_eq!(into_hyper(resp, false).headers().get("content-length").unwrap(), "20");
        let resp = Response::bytes(vec![b'x'; 20], "image/png").with_header("Content-Length", "junk");
        assert_eq!(into_hyper(resp, false).headers().get("content-length").unwrap(), "20");
    }
}
