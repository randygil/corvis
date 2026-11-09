use std::io::Read;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use serde_json::Value;

use super::body::{Coding, brotli_window};
use super::negotiate::negotiate;
use super::*;
use crate::server::ResBody;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/compression/compression.json")).unwrap()
}

// ---- The recorded tables of the libraries ----

#[test]
fn negotiation_matches_negotiator() {
    let fixture = fixture();
    let table = fixture["negotiation"].as_array().unwrap();
    assert!(table.len() > 50);
    for entry in table {
        let header = entry["acceptEncoding"].as_str();
        let expected = entry["result"].as_str();
        assert_eq!(negotiate(header), expected, "Accept-Encoding {header:?}");
    }
}

#[test]
fn compressible_matches_the_library() {
    let fixture = fixture();
    assert_eq!(fixture["versions"]["mime-db"], "1.54.0");
    assert_eq!(compressible::COMPRESSIBLE.len(), 822);
    for entry in fixture["compressible"].as_array().unwrap() {
        let Some(ty) = entry["type"].as_str().filter(|t| !t.is_empty()) else { continue };
        let expected = entry["result"].as_bool();
        assert_eq!(compressible(ty), expected, "type {ty:?}");
    }
}

#[test]
fn should_compress_adds_only_the_event_stream_rule() {
    let fixture = fixture();
    let headers = |ty: &str| {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_str(ty).unwrap());
        h
    };
    for entry in fixture["compressible"].as_array().unwrap() {
        let Some(ty) = entry["type"].as_str() else { continue };
        let h = headers(ty);
        let ctx = CompressionContext {
            method: &Method::GET,
            path: "/",
            request_headers: &HeaderMap::new(),
            status: StatusCode::OK,
            response_headers: &h,
        };
        let expected = entry["result"].as_bool().unwrap_or(false) && !ty.starts_with("text/event-stream");
        assert_eq!(should_compress(&ctx), expected, "type {ty:?}");
    }
    let none = HeaderMap::new();
    let ctx =
        CompressionContext { method: &Method::GET, path: "/", request_headers: &none, status: StatusCode::OK, response_headers: &none };
    assert!(!should_compress(&ctx), "no Content-Type");
}

// ---- Options ----

#[test]
fn options_are_clamped() {
    let o = CompressionOptions::new();
    assert_eq!((o.threshold_bytes(), o.gzip_level(), o.quality()), (1024, 6, 4));
    let o = CompressionOptions::new().level(20).brotli_quality(50).threshold(5);
    assert_eq!((o.threshold_bytes(), o.gzip_level(), o.quality()), (5, 9, 11));
}

#[test]
fn brotli_window_follows_the_body_length() {
    assert_eq!(brotli_window(Some(0)), 16);
    assert_eq!(brotli_window(Some(1024)), 16);
    assert_eq!(brotli_window(Some(65_520)), 16);
    assert_eq!(brotli_window(Some(65_521)), 17);
    assert_eq!(brotli_window(Some(262_128)), 18);
    assert_eq!(brotli_window(Some(262_129)), 19);
    assert_eq!(brotli_window(Some(10 * 1024 * 1024)), 19);
    assert_eq!(brotli_window(None), 19);
}

#[test]
fn vary_follows_the_vary_package() {
    assert_eq!(vary_append("", "Accept-Encoding"), "Accept-Encoding");
    assert_eq!(vary_append("Origin", "Accept-Encoding"), "Origin, Accept-Encoding");
    assert_eq!(vary_append("Origin,Cookie", "Accept-Encoding"), "Origin,Cookie, Accept-Encoding");
    assert_eq!(vary_append("origin, accept-encoding", "Accept-Encoding"), "origin, accept-encoding");
    assert_eq!(vary_append("*", "Accept-Encoding"), "*");
    assert_eq!(vary_append("Origin, *", "Accept-Encoding"), "*");
}

// ---- Decisions ----

struct Case {
    method: Method,
    accept: Option<&'static str>,
    status: StatusCode,
    headers: HeaderMap,
    len: Option<u64>,
    opts: CompressionOptions,
}

impl Case {
    fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("10240"));
        Case {
            method: Method::GET,
            accept: Some("gzip"),
            status: StatusCode::OK,
            headers,
            len: Some(10240),
            opts: CompressionOptions::new(),
        }
    }
    fn header(mut self, name: header::HeaderName, value: &'static str) -> Self {
        self.headers.insert(name, HeaderValue::from_static(value));
        self
    }
    fn without(mut self, name: header::HeaderName) -> Self {
        self.headers.remove(name);
        self
    }
    fn decide(&self) -> Decision {
        let mut req_headers = HeaderMap::new();
        if let Some(a) = self.accept {
            req_headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_static(a));
        }
        let mut info = RequestInfo::new(&self.method, &HeaderMap::new());
        info.headers = req_headers;
        decide(&info, &ResponseFacts { status: self.status, headers: &self.headers, body_len: self.len }, &self.opts)
    }
}

fn compressed(enc: &'static str) -> Decision {
    Decision { add_vary: true, encoding: Some(enc) }
}
const VARY_ONLY: Decision = Decision { add_vary: true, encoding: None };
const UNTOUCHED: Decision = Decision { add_vary: false, encoding: None };

#[test]
fn decisions_follow_the_reference_order() {
    assert_eq!(Case::new().decide(), compressed("gzip"));
    // filter and no-transform come before Vary
    assert_eq!(Case::new().header(header::CONTENT_TYPE, "image/png").decide(), UNTOUCHED);
    assert_eq!(Case::new().without(header::CONTENT_TYPE).decide(), UNTOUCHED);
    assert_eq!(Case::new().header(header::CONTENT_TYPE, "text/event-stream").decide(), UNTOUCHED);
    assert_eq!(Case::new().header(header::CACHE_CONTROL, "private, no-transform").decide(), UNTOUCHED);
    assert_eq!(Case::new().header(header::CACHE_CONTROL, "No-Transform").decide(), UNTOUCHED);
    assert_eq!(Case::new().header(header::CACHE_CONTROL, "max-age=1, no-transform ,public").decide(), UNTOUCHED);
    assert_eq!(Case::new().header(header::CACHE_CONTROL, "x-no-transform").decide(), compressed("gzip"));
    // content type charset and case
    assert_eq!(Case::new().header(header::CONTENT_TYPE, "Text/Plain").decide(), compressed("gzip"));
    assert_eq!(Case::new().header(header::CONTENT_TYPE, "application/octet-stream").decide(), compressed("gzip"));
}

#[test]
fn threshold_and_length() {
    let c = |len: &'static str| Case::new().header(header::CONTENT_LENGTH, len).decide();
    assert_eq!(c("1023"), VARY_ONLY);
    assert_eq!(c("1024"), compressed("gzip"));
    assert_eq!(c("abc"), compressed("gzip"));
    // without Content-Length the exact body size decides; unknown sizes are compressed
    let mut case = Case::new().without(header::CONTENT_LENGTH);
    case.len = Some(1023);
    assert_eq!(case.decide(), VARY_ONLY);
    case.len = Some(1024);
    assert_eq!(case.decide(), compressed("gzip"));
    case.len = None;
    assert_eq!(case.decide(), compressed("gzip"));
    case.len = Some(0);
    case.opts = CompressionOptions::new().threshold(0);
    assert_eq!(case.decide(), compressed("gzip"));
}

#[test]
fn encoded_head_bodyless_and_ranged() {
    assert_eq!(Case::new().header(header::CONTENT_ENCODING, "gzip").decide(), VARY_ONLY);
    assert_eq!(Case::new().header(header::CONTENT_ENCODING, "identity").decide(), compressed("gzip"));
    assert_eq!(Case::new().header(header::CONTENT_ENCODING, "Identity").decide(), VARY_ONLY);
    let mut case = Case::new();
    case.method = Method::HEAD;
    assert_eq!(case.decide(), VARY_ONLY);
    for status in [StatusCode::NO_CONTENT, StatusCode::NOT_MODIFIED, StatusCode::CONTINUE] {
        let mut case = Case::new();
        case.status = status;
        assert_eq!(case.decide(), VARY_ONLY, "{status}");
    }
    let mut case = Case::new().header(header::CONTENT_RANGE, "bytes 0-49999/102400");
    case.status = StatusCode::PARTIAL_CONTENT;
    assert_eq!(case.decide(), VARY_ONLY);
    assert_eq!(Case::new().header(header::CONTENT_RANGE, "bytes */102400").decide(), VARY_ONLY);
}

#[test]
fn negotiation_in_decisions() {
    let accept = |a: Option<&'static str>| {
        let mut c = Case::new();
        c.accept = a;
        c.decide()
    };
    assert_eq!(accept(None), VARY_ONLY);
    assert_eq!(accept(Some("")), VARY_ONLY);
    assert_eq!(accept(Some("identity")), VARY_ONLY);
    assert_eq!(accept(Some("identity;q=0, gzip;q=0")), VARY_ONLY);
    assert_eq!(accept(Some("gzip, deflate, br")), compressed("br"));
    assert_eq!(accept(Some("br;q=0.5, gzip")), compressed("gzip"));
    assert_eq!(accept(Some("deflate")), compressed("deflate"));
    assert_eq!(accept(Some("*")), compressed("br"));
    assert_eq!(accept(Some("compress")), VARY_ONLY);
}

#[test]
fn a_custom_filter_replaces_the_default() {
    let mut case = Case::new().header(header::CONTENT_TYPE, "image/png");
    case.opts = CompressionOptions::new().filter(|_| true);
    assert_eq!(case.decide(), compressed("gzip"));
    case.opts = CompressionOptions::new().filter(|ctx| ctx.status == StatusCode::OK && should_compress(ctx));
    assert_eq!(case.decide(), UNTOUCHED);
}

// ---- The compressed body ----

fn body_of(chunks: Vec<Bytes>) -> ResBody {
    let stream = futures::stream::iter(chunks.into_iter().map(|c| Ok::<_, std::io::Error>(Frame::data(c))));
    StreamBody::new(stream).boxed_unsync()
}

fn decode(coding: Coding, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    match coding {
        Coding::Gzip => flate2::read::GzDecoder::new(data).read_to_end(&mut out).unwrap(),
        Coding::Deflate => flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).unwrap(),
        Coding::Brotli => brotli::Decompressor::new(data, 4096).read_to_end(&mut out).unwrap(),
    };
    out
}

fn compress(coding: Coding, chunks: Vec<Bytes>, len: Option<u64>) -> CompressedBody {
    CompressedBody::new(body_of(chunks), coding, 6, 4, len, None)
}

fn sample(n: usize) -> Vec<u8> {
    (0..n).map(|i| b"The quick brown fox jumps over the lazy dog. "[i % 45]).collect()
}

const CODINGS: [Coding; 3] = [Coding::Gzip, Coding::Deflate, Coding::Brotli];

#[tokio::test]
async fn bodies_decode_to_the_original() {
    for coding in CODINGS {
        for size in [0usize, 1, 100, 70_000, 5 * 1024 * 1024] {
            let data = sample(size);
            let chunks = if size == 0 { vec![] } else { data.chunks(48 * 1024).map(Bytes::copy_from_slice).collect() };
            let out = compress(coding, chunks, Some(size as u64)).collect().await.unwrap().to_bytes();
            assert!(decode(coding, &out) == data, "{coding:?} {size}");
        }
        // many small chunks and empty ones in between
        let data = sample(20_000);
        let mut chunks = Vec::new();
        for piece in data.chunks(7) {
            chunks.push(Bytes::copy_from_slice(piece));
            chunks.push(Bytes::new());
        }
        let out = compress(coding, chunks, None).collect().await.unwrap().to_bytes();
        assert!(decode(coding, &out) == data, "{coding:?} small chunks");
    }
}

#[tokio::test]
async fn a_large_body_round_trips_frame_by_frame() {
    let data = sample(1024 * 1024);
    let mut body = compress(Coding::Gzip, vec![Bytes::from(data.clone())], Some(data.len() as u64));
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        out.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    assert!(decode(Coding::Gzip, &out) == data);
}

#[tokio::test]
async fn a_failing_source_ends_the_body_with_an_error() {
    for coding in CODINGS {
        let stream = futures::stream::iter(vec![
            Ok::<_, std::io::Error>(Frame::data(Bytes::from(sample(5000)))),
            Err(std::io::Error::other("disk failure")),
        ]);
        let mut body = CompressedBody::new(StreamBody::new(stream).boxed_unsync(), coding, 6, 4, None, None);
        let mut error = None;
        while let Some(frame) = body.frame().await {
            if let Err(e) = frame {
                error = Some(e);
            }
        }
        assert_eq!(error.expect("the error is passed through").to_string(), "disk failure", "{coding:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_slow_stream_is_flushed_before_the_next_chunk() {
    for coding in CODINGS {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
        let stream =
            futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|c| (Ok::<_, std::io::Error>(Frame::data(c)), rx)) });
        let mut body = CompressedBody::new(StreamBody::new(stream).boxed_unsync(), coding, 6, 4, None, None);
        let first = sample(2048);
        tx.send(Bytes::from(first.clone())).unwrap();
        let mut got = Vec::new();
        // The stream stays open and idle: the bytes of the first chunk must still arrive.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, body.frame()).await {
            got.extend_from_slice(&frame.unwrap().into_data().unwrap());
            if let Some(decoded) = try_decode(coding, &got)
                && decoded.len() >= first.len()
            {
                break;
            }
        }
        let decoded = try_decode(coding, &got).unwrap_or_default();
        assert!(decoded == first, "{coding:?}: the first chunk decodes before the second exists ({} bytes)", decoded.len());
        tx.send(Bytes::from_static(b"second")).unwrap();
        drop(tx);
        while let Some(frame) = body.frame().await {
            got.extend_from_slice(&frame.unwrap().into_data().unwrap());
        }
        let mut all = first.clone();
        all.extend_from_slice(b"second");
        assert!(decode(coding, &got) == all, "{coding:?}");
    }
}

/// Decodes as much as a truncated stream allows.
fn try_decode(coding: Coding, data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    let mut reader: Box<dyn Read + '_> = match coding {
        Coding::Gzip => Box::new(flate2::read::GzDecoder::new(data)),
        Coding::Deflate => Box::new(flate2::read::ZlibDecoder::new(data)),
        Coding::Brotli => Box::new(brotli::Decompressor::new(data, 4096)),
    };
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => return Some(out),
        }
    }
}

#[tokio::test]
async fn the_compressed_body_has_no_exact_size() {
    let body = compress(Coding::Gzip, vec![Bytes::from_static(b"abc")], Some(3));
    assert!(hyper::body::Body::size_hint(&body).exact().is_none());
    let mut stream = http_body_util::BodyDataStream::new(body);
    while let Some(chunk) = stream.next().await {
        chunk.unwrap();
    }
}

// ---- The cap on compressions in progress ----

/// A JSON response of `size` bytes in one chunk; with `stalled` the body then never ends.
fn wire(size: usize, stalled: bool) -> hyper::Response<ResBody> {
    let first = futures::stream::iter([Ok::<_, std::io::Error>(Frame::data(Bytes::from(sample(size))))]);
    let body = if stalled {
        StreamBody::new(first.chain(futures::stream::pending())).boxed_unsync()
    } else {
        StreamBody::new(first).boxed_unsync()
    };
    let mut resp = hyper::Response::new(body);
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    resp
}

fn request_accepting(accept: &str) -> RequestInfo {
    let mut info = RequestInfo::new(&Method::GET, &HeaderMap::new());
    info.headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_str(accept).unwrap());
    info
}

fn encoding_of(resp: &hyper::Response<ResBody>) -> Option<&str> {
    resp.headers().get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn at_the_cap_brotli_falls_back_to_gzip_and_then_to_identity() {
    // two slots, so brotli may use one
    let opts = CompressionOptions::new().threshold(0).max_concurrent(2);
    assert_eq!(opts.max_concurrent_compressions(), 2);
    let req = request_accepting("br, gzip");
    let first = apply(wire(2000, true), &req, &opts);
    assert_eq!(encoding_of(&first), Some("br"));
    assert_eq!(opts.available_slots(), (1, 0));
    // the brotli sub-cap is spent: gzip takes the last slot
    let second = apply(wire(2000, true), &req, &opts);
    assert_eq!(encoding_of(&second), Some("gzip"));
    assert_eq!(opts.available_slots(), (0, 0));
    // nothing is free: uncompressed, but still marked as varying so caches stay correct
    let third = apply(wire(2000, true), &req, &opts);
    assert_eq!(encoding_of(&third), None);
    assert_eq!(third.headers().get(header::VARY).unwrap(), "Accept-Encoding");
    // a client that only takes brotli gets nothing either
    let fourth = apply(wire(2000, true), &request_accepting("br"), &opts);
    assert_eq!(encoding_of(&fourth), None);
    assert_eq!(fourth.headers().get(header::VARY).unwrap(), "Accept-Encoding");
    // dropping a response frees its slots
    drop(first);
    assert_eq!(opts.available_slots(), (1, 1));
    let fifth = apply(wire(2000, true), &req, &opts);
    assert_eq!(encoding_of(&fifth), Some("br"));
    drop((second, fifth));
    assert_eq!(opts.available_slots(), (2, 1));
}

#[tokio::test]
async fn the_slot_is_released_when_the_body_ends() {
    let opts = CompressionOptions::new().threshold(0).max_concurrent(4);
    let resp = apply(wire(5000, false), &request_accepting("gzip"), &opts);
    assert_eq!(opts.available_slots().0, 3);
    let mut body = resp.into_body();
    while let Some(frame) = body.frame().await {
        frame.unwrap();
    }
    // the body is still alive, its slot is already free
    assert_eq!(opts.available_slots().0, 4);
}

#[tokio::test]
async fn the_slot_is_released_when_a_stalled_body_is_dropped() {
    let opts = CompressionOptions::new().threshold(0).max_concurrent(4);
    let resp = apply(wire(5000, true), &request_accepting("br"), &opts);
    let mut body = resp.into_body();
    // read the first bytes and stop, as a client that goes away does
    let _ = tokio::time::timeout(Duration::from_millis(50), body.frame()).await;
    assert_eq!(opts.available_slots(), (3, 1));
    drop(body);
    assert_eq!(opts.available_slots(), (4, 2));
}

#[tokio::test]
async fn one_connection_cannot_hold_more_than_its_cap() {
    let opts = CompressionOptions::new().threshold(0).max_per_connection(4);
    assert_eq!(opts.max_per_connection_compressions(), 4);
    assert_eq!(CompressionOptions::new().max_per_connection(0).max_per_connection_compressions(), 1);
    let conn = ConnectionSlots::default();
    let mut req = request_accepting("br, gzip");
    req.conn_slots = Some(conn.clone());
    let mut held: Vec<_> = (0..4).map(|_| apply(wire(2000, true), &req, &opts)).collect();
    assert!(held.iter().all(|r| encoding_of(r).is_some()));
    assert_eq!(conn.in_use(), 4);
    // the fifth response of the same connection goes out uncompressed but still varies
    let fifth = apply(wire(2000, true), &req, &opts);
    assert_eq!(encoding_of(&fifth), None);
    assert_eq!(fifth.headers().get(header::VARY).unwrap(), "Accept-Encoding");
    assert_eq!(opts.available_slots().0, opts.max_concurrent_compressions() - 4, "the global cap was not touched");
    // another connection, and a request without a connection, are not affected
    let mut other = request_accepting("gzip");
    other.conn_slots = Some(ConnectionSlots::default());
    assert_eq!(encoding_of(&apply(wire(2000, true), &other, &opts)), Some("gzip"));
    assert_eq!(encoding_of(&apply(wire(2000, true), &request_accepting("gzip"), &opts)), Some("gzip"));
    // a dropped body frees the connection's slot
    drop(held.pop());
    assert_eq!(conn.in_use(), 3);
    let again = apply(wire(2000, true), &req, &opts);
    assert!(encoding_of(&again).is_some());
    assert_eq!(conn.in_use(), 4);
    drop((held, again));
    assert_eq!(conn.in_use(), 0);
}

#[tokio::test]
async fn the_connection_slot_is_released_when_the_body_ends() {
    let opts = CompressionOptions::new().threshold(0).max_per_connection(1);
    let conn = ConnectionSlots::default();
    let mut req = request_accepting("gzip");
    req.conn_slots = Some(conn.clone());
    let resp = apply(wire(5000, false), &req, &opts);
    assert_eq!(conn.in_use(), 1);
    let mut body = resp.into_body();
    while let Some(frame) = body.frame().await {
        frame.unwrap();
    }
    assert_eq!(conn.in_use(), 0, "released at the end of the body, which is still alive");
    assert!(encoding_of(&apply(wire(5000, false), &req, &opts)).is_some());
}

#[tokio::test]
async fn clones_share_the_cap_and_zero_means_one() {
    let opts = CompressionOptions::new().threshold(0).max_concurrent(0);
    assert_eq!(opts.max_concurrent_compressions(), 1);
    let copy = opts.clone();
    let held = apply(wire(2000, true), &request_accepting("gzip"), &opts);
    assert_eq!(encoding_of(&held), Some("gzip"));
    let other = apply(wire(2000, true), &request_accepting("gzip"), &copy);
    assert_eq!(encoding_of(&other), None);
    assert!(CompressionOptions::new().max_concurrent_compressions() >= 8);
}
