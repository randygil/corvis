//! HTTP server (hyper) integration.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full, Limited, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use tokio::net::TcpListener;

use crate::http::{Body, Request, Response};
use crate::router::{AppShared, dispatch};

pub(crate) type ResBody = UnsyncBoxBody<Bytes, std::io::Error>;

fn full(data: impl Into<Bytes>) -> ResBody {
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
    if let Some(ct) = content_type.filter(|ct| !ct.is_empty()) {
        if !headers.contains_key(http::header::CONTENT_TYPE) {
            if let Ok(v) = http::HeaderValue::from_str(&ct) {
                headers.insert(http::header::CONTENT_TYPE, v);
            }
        }
    }
    // `X-Powered-By`, ETag, ... are added by `http::express::finalize` (see `handle`).
    let bodyless = status == http::StatusCode::NO_CONTENT || status == http::StatusCode::NOT_MODIFIED || status.is_informational();
    if head && !bodyless && !headers.contains_key(http::header::CONTENT_LENGTH) {
        // HEAD keeps the length of the body it would have sent (like Node).
        if let Some(len) = hyper::body::Body::size_hint(&body).exact() {
            headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from(len));
        }
    }
    if let Some(h) = builder.headers_mut() {
        *h = headers;
    }
    let body = if head { full(Bytes::new()) } else { body };
    builder.body(body).unwrap_or_else(|_| hyper::Response::new(full(Bytes::new())))
}

/// Converts a hyper request into a corvis request (reading the body with a limit).
pub(crate) async fn from_hyper(
    req: hyper::Request<Incoming>,
    remote: Option<SocketAddr>,
    limit: usize,
    parser: &crate::http::body_parser::BodyParserOptions,
) -> Result<Request, Response> {
    let (parts, body) = req.into_parts();
    // Multipart bodies have no body-parser limit: multer applies its own `limits`.
    let limit = if crate::multer::is_multipart_content_type(&parts.headers) { usize::MAX } else { limit };
    let (bytes, too_large) = match Limited::new(body, limit).collect().await {
        Ok(c) => (c.to_bytes(), false),
        Err(_) => (Bytes::new(), true),
    };
    // Body-parser errors stay in the request: the router answers them after CORS and through
    // the global filters (Express `next(err)`).
    let mut r = Request::with_parser(parts.method, parts.uri, parts.headers, bytes, parser);
    if too_large {
        r.body_error = Some(crate::http::HttpException::http_error(413, "request entity too large"));
    }
    r.version = parts.version;
    r.remote_addr = remote;
    *r.extensions.write().unwrap() = parts.extensions;
    Ok(r)
}

async fn handle(
    shared: Arc<AppShared>,
    req: hyper::Request<Incoming>,
    remote: SocketAddr,
) -> Result<hyper::Response<ResBody>, crate::http_adapter::SocketDestroyed> {
    #[cfg(feature = "websockets")]
    {
        if let Some(gw) = crate::websockets::match_upgrade(&shared, req.headers(), req.uri().path()) {
            return Ok(crate::websockets::upgrade(gw, req, remote));
        }
    }
    // `server.on('upgrade', ...)` listeners take every upgrade request (Node semantics).
    if crate::http_adapter::is_upgrade_request(&req) {
        if shared.http_server.upgrade_listener_count() > 0 {
            return crate::http_adapter::dispatch_upgrade(&shared.http_server, req, remote).await;
        }
        // Like the `ws` adapter, upgrades for paths without a gateway are destroyed.
        #[cfg(feature = "websockets")]
        {
            if !shared.gateways.is_empty() {
                return Err(crate::http_adapter::SocketDestroyed);
            }
        }
    }
    let head = req.method() == http::Method::HEAD;
    let info = crate::http::express::RequestInfo::new(req.method(), req.headers());
    let mut resp = match from_hyper(req, Some(remote), shared.body_limit, &shared.http_settings.body_parser).await {
        Ok(r) => dispatch(shared.clone(), r).await,
        Err(resp) => resp,
    };
    crate::http::express::finalize(&mut resp, &info, &shared.http_settings);
    Ok(into_hyper(resp, head))
}

/// Accepts connections until `shutdown` resolves, then drains them gracefully.
pub(crate) async fn serve(
    listener: TcpListener,
    shared: Arc<AppShared>,
    shutdown: impl std::future::Future<Output = ()> + Send,
) -> std::io::Result<()> {
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let builder = auto::Builder::new(TokioExecutor::new());
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, remote) = match accepted {
                    Ok(x) => x,
                    Err(_) => continue,
                };
                let _ = stream.set_nodelay(true);
                let shared = shared.clone();
                let service = hyper::service::service_fn(move |req| handle(shared.clone(), req, remote));
                let conn = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
                let conn = graceful.watch(conn.into_owned());
                tokio::spawn(async move {
                    let _ = conn.await;
                });
            }
            _ = &mut shutdown => break,
        }
    }
    drop(listener);
    tokio::select! {
        _ = graceful.shutdown() => {}
        _ = tokio::time::sleep(Duration::from_secs(10)) => {}
    }
    Ok(())
}
