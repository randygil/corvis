//! Unit tests of the framing check, and the property test that compares it with hyper.

use super::*;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use proptest::prelude::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// ---------------------------------------------------------------------------
// Driving the core the way `TimedIo` does
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Outcome {
    /// Bytes that go to hyper, including the rejection bytes.
    out: Vec<u8>,
    ending: Option<Ending>,
    wait: Option<u64>,
    completed: u64,
    holding: bool,
    /// The stream ended inside a head the check had buffered, so hyper got it incomplete.
    partial: bool,
    msgs: Vec<TestMsg>,
    /// Every head forwarded as a request, finished or not.
    heads: Vec<Vec<u8>>,
}

/// Feeds the pieces like `poll_read` does (in-place prefix first, then the machine), then end of input.
fn run_pieces(max_head: usize, pieces: &[&[u8]]) -> Outcome {
    run_pieces_with(max_head, pieces, false)
}

/// With `resume`, a connection held after an `Upgrade` request behaves as if the request was answered without `101`.
fn run_pieces_with(max_head: usize, pieces: &[&[u8]], resume: bool) -> Outcome {
    let mut fr = Framing::new(max_head);
    let mut out = Vec::new();
    for piece in pieces {
        let k = fr.passthrough_prefix(piece);
        out.extend_from_slice(&piece[..k]);
        fr.feed(&piece[k..], &mut out);
    }
    let mut holding = fr.is_holding();
    if resume {
        while fr.is_holding() {
            fr.resume(&mut out);
        }
        holding = false;
    }
    let partial = fr.has_partial_head();
    fr.eof(&mut out);
    let terminal = fr.pending_terminal();
    if terminal.is_some() {
        fr.send_terminal(&mut out);
    }
    Outcome {
        out,
        ending: terminal.map(|t| t.ending),
        wait: terminal.map(|t| t.wait),
        completed: fr.log.msgs.len() as u64,
        holding,
        partial,
        msgs: fr.log.msgs.clone(),
        heads: fr.log.heads.clone(),
    }
}

fn run(stream: &[u8]) -> Outcome {
    run_pieces(32 * 1024, &[stream])
}

/// The outcome is the same for every way of splitting the stream into reads.
fn assert_split_invariant(stream: &[u8]) -> Outcome {
    let whole = run(stream);
    for at in 0..=stream.len() {
        let got = run_pieces(32 * 1024, &[&stream[..at], &stream[at..]]);
        assert_eq!(got, whole, "split at {at} of {:?}", String::from_utf8_lossy(stream));
    }
    let bytes: Vec<&[u8]> = stream.chunks(1).collect();
    assert_eq!(run_pieces(32 * 1024, &bytes), whole, "one byte at a time: {:?}", String::from_utf8_lossy(stream));
    for size in [2usize, 3, 5, 7] {
        let pieces: Vec<&[u8]> = stream.chunks(size).collect();
        assert_eq!(run_pieces(32 * 1024, &pieces), whole, "pieces of {size}: {:?}", String::from_utf8_lossy(stream));
    }
    whole
}

/// Everything is forwarded unchanged and the stream is understood to its end.
fn assert_accepted(stream: &[u8], messages: u64) {
    let o = assert_split_invariant(stream);
    assert_eq!(o.out, stream, "{:?}", String::from_utf8_lossy(stream));
    assert_eq!(o.ending, None);
    assert_eq!(o.completed, messages, "{:?}", String::from_utf8_lossy(stream));
}

/// The valid bytes before the offence are forwarded, then the rejection bytes.
fn assert_rejected(stream: &[u8], forwarded: &[u8]) {
    let o = assert_split_invariant(stream);
    let mut want = forwarded.to_vec();
    want.extend_from_slice(b"\r\0");
    assert_eq!(o.out, want, "{:?}", String::from_utf8_lossy(stream));
    assert_eq!(o.ending, Some(Ending::Reject));
}

/// The head is handed to hyper unchanged and the stream ends there.
fn assert_handed_off(stream: &[u8], head: &[u8]) {
    let o = assert_split_invariant(stream);
    assert_eq!(o.out, head, "{:?}", String::from_utf8_lossy(stream));
    assert_eq!(o.ending, Some(Ending::End));
}

const GET: &[u8] = b"GET /a HTTP/1.1\r\nHost: x\r\n\r\n";

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

// ---------------------------------------------------------------------------
// What the spec delta rejects
// ---------------------------------------------------------------------------

#[test]
fn bare_lf_in_the_head_is_rejected() {
    assert_rejected(b"GET / HTTP/1.1\nHost: x\n\n", b"");
    assert_rejected(b"GET / HTTP/1.1\nHost: x\r\n\r\n", b"");
    assert_rejected(b"GET / HTTP/1.1\r\nHost: x\r\n\n", b"");
    assert_rejected(b"GET / HTTP/1.1\r\nHost: x\n\r\n", b"");
    assert_rejected(b"GET / HTTP/1.1\r\n\nHost: x\r\n\r\n", b"");
}

#[test]
fn bare_cr_in_the_head_is_rejected() {
    assert_rejected(b"GET / HTTP/1.1\r\nHost: x\rX: y\r\n\r\n", b"");
    assert_rejected(b"GET / HTTP/1.1\r\r\n\r\n", b"");
    assert_rejected(b"\rGET / HTTP/1.1\r\n\r\n", b"");
}

#[test]
fn a_rejected_head_keeps_the_requests_before_it() {
    let stream = cat(&[GET, b"GET / HTTP/1.1\nHost: x\n\n", GET]);
    assert_rejected(&stream, GET);
}

#[test]
fn empty_lines_before_the_request_line_are_accepted_and_dropped() {
    for (stream, messages) in [(cat(&[b"\r\n", GET]), 1), (cat(&[b"\n", GET]), 1), (cat(&[b"\r\n\n\r\n", GET, b"\r\n", GET]), 2)] {
        let o = assert_split_invariant(&stream);
        // hyper never sees them: a lone `\r\n` left in its buffer would hide the rejection bytes that may follow.
        assert_eq!(o.out, cat(&[GET].repeat(messages as usize)), "{:?}", String::from_utf8_lossy(&stream));
        assert_eq!(o.ending, None);
        assert_eq!(o.completed, messages);
    }
}

#[test]
fn a_rejection_after_empty_lines_leaves_nothing_in_front_of_the_rejection_bytes() {
    for rejected_head in [
        &b"GET / HTTP/1.1\nHost: x\n\n"[..],
        b"GET / HTTP/1.1\r\nHost: x\r\n\n",
        b"GET / HTTP/1.1\r\nHost: x\rX: y\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n",
    ] {
        for empty in [&b"\r\n"[..], b"\n", b"\r\n\n\r\n"] {
            assert_rejected(&cat(&[empty, rejected_head, GET]), b"");
            assert_rejected(&cat(&[GET, empty, rejected_head, GET]), GET);
        }
    }
    // A lone CR at the end of the input is dropped, not forwarded.
    assert_eq!(run(&cat(&[GET, b"\r"])).out, GET);
}

#[test]
fn chunk_size_followed_by_whitespace_is_rejected() {
    let head = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
    for (body, forwarded) in [
        (&b"2 \r\nhi\r\n0\r\n\r\n"[..], &b"2"[..]),
        (b"2\t\r\nhi\r\n0\r\n\r\n", b"2"),
        (b"2 ;a=b\r\nhi\r\n0\r\n\r\n", b"2"),
        (b" 2\r\nhi\r\n0\r\n\r\n", b""),
        (b"2\r\nhi\r\n0 \r\n\r\n", b"2\r\nhi\r\n0"),
    ] {
        assert_rejected(&cat(&[head, body]), &cat(&[head, forwarded]));
    }
}

#[test]
fn chunk_extensions_and_leading_zeros_are_accepted() {
    let head = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
    for body in [
        &b"2;a=b\r\nhi\r\n0\r\n\r\n"[..],
        b"2;a=\"x y\";b\r\nhi\r\n0\r\n\r\n",
        b"00000000000000000000000002\r\nhi\r\n0\r\n\r\n",
        b"2\r\nhi\r\n0;last\r\n\r\n",
        b"A\r\n0123456789\r\n0\r\n\r\n",
        b"a\r\n0123456789\r\n0\r\n\r\n",
        b"0\r\n\r\n",
        b"1\r\n\r\r\n0\r\n\r\n",
        b"2\r\nhi\r\n0\r\nX-A: b\r\n\r\n",
        b"2\r\nhi\r\n0\r\nX-A: b\r\nX-C: d\r\n\r\n",
    ] {
        assert_accepted(&cat(&[head, body]), 1);
        assert_accepted(&cat(&[head, body, head, body, GET]), 3);
    }
}

#[test]
fn already_rejected_chunk_errors_are_rejected_here_too() {
    let head = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
    for body in [
        &b"10000000000000002\r\nhi\r\n0\r\n\r\n"[..],
        b"2\nhi\n0\n\n",
        b"2;a=b\nc\r\nhi\r\n0\r\n\r\n",
        b"\r\nhi\r\n0\r\n\r\n",
        b";a\r\nhi\r\n0\r\n\r\n",
        b"zz\r\nhi\r\n0\r\n\r\n",
        b"2\r\nhiXX0\r\n\r\n",
        b"2\r\nhi\rX0\r\n\r\n",
        b"2\r",
    ] {
        let o = run(&cat(&[head, body, GET]));
        if o.ending == Some(Ending::Reject) {
            assert!(o.out.ends_with(b"\r\0"));
            assert!(!o.out.windows(GET.len()).any(|w| w == GET), "{:?}", String::from_utf8_lossy(body));
        } else {
            // `2\r` and nothing more is just an incomplete stream.
            assert_eq!(body, b"2\r");
        }
    }
}

#[test]
fn trailer_with_a_bare_lf_is_rejected() {
    let head = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
    for body in [
        &b"2\r\nhi\r\n0\r\nX-A: b\nX-C: d\r\n\r\n"[..],
        b"2\r\nhi\r\n0\r\nX-A: b\n\nGARBAGE\r\n\r\n",
        b"2\r\nhi\r\n0\r\nX-A: b\r\n\n",
        b"2\r\nhi\r\n0\r\n\n",
        b"2\r\nhi\r\n0\r\nX-A: b\rX\r\n\r\n",
    ] {
        let o = assert_split_invariant(&cat(&[head, body, GET]));
        assert_eq!(o.ending, Some(Ending::Reject), "{:?}", String::from_utf8_lossy(body));
        assert!(!o.out.windows(GET.len()).any(|w| w == GET));
        assert_eq!(o.completed, 0);
    }
}

#[test]
fn content_length_with_transfer_encoding_is_rejected_in_any_order() {
    let a = b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n";
    let b = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nContent-Length: 4\r\n\r\n2\r\nhi\r\n0\r\n\r\n";
    let c = b"POST / HTTP/1.1\r\nHost: x\r\ncontent-length: 4\r\nTRANSFER-ENCODING: gzip\r\n\r\nabcd";
    for s in [&a[..], &b[..], &c[..]] {
        assert_rejected(&cat(&[s, GET]), b"");
    }
    assert_rejected(&cat(&[GET, a, GET]), GET);
}

// ---------------------------------------------------------------------------
// What is accepted
// ---------------------------------------------------------------------------

#[test]
fn content_length_bodies_are_forwarded_by_count() {
    let post = b"POST /p HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello";
    assert_accepted(post, 1);
    assert_accepted(&cat(&[post, post, GET]), 3);
    // The body may look like anything.
    assert_accepted(b"POST /p HTTP/1.1\r\nHost: x\r\nContent-Length: 12\r\n\r\n\n\r\n\r\nGET / H\0", 1);
    assert_accepted(b"POST /p HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n", 1);
    assert_accepted(b"POST /p HTTP/1.1\r\nHost: x\r\nContent-Length: 05\r\n\r\nhello", 1);
    assert_accepted(b"POST /p HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nhello", 1);
    assert_accepted(b"POST /p HTTP/1.1\r\nHost: x\r\ncontent-length:\t5 \r\n\r\nhello", 1);
}

#[test]
fn chunked_variants_that_have_one_reading_are_accepted() {
    let tail = b"\r\n\r\n2\r\nhi\r\n0\r\n\r\n";
    for te in [
        &b"Transfer-Encoding: chunked"[..],
        b"transfer-encoding: CHUNKED",
        b"Transfer-Encoding: chunked, chunked",
        b"Transfer-Encoding: gzip, chunked",
        b"Transfer-Encoding: gzip\r\nTransfer-Encoding: chunked",
    ] {
        let s = cat(&[b"POST / HTTP/1.1\r\nHost: x\r\n", te, tail]);
        assert_accepted(&s, 1);
    }
}

#[test]
fn heads_hyper_handles_alone_are_handed_off() {
    for head in [
        // `chunked` is not the last coding.
        &b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked, gzip\r\n\r\n"[..],
        b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: gzip\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: gzip\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding:\r\n\r\n",
        // Content-Length that is not one number.
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nContent-Length: 5\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 4, 5\r\n\r\n",
        // hyper's server reads Content-Length with `from_digits`: a list, even of equal values, is a `400`.
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5, 5\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5,\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: +4\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: -4\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 4x\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length:\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 18446744073709551616\r\n\r\n",
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 18446744073709551614\r\n\r\n",
        // HTTP/1.0 with Transfer-Encoding, other versions.
        b"POST / HTTP/1.0\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n",
        b"GET / HTTP/1.2\r\nHost: x\r\n\r\n",
        b"GET / HTTP/2.0\r\nHost: x\r\n\r\n",
        b"GET /\r\nHost: x\r\n\r\n",
        b"GET / HTTP/1.1 \r\nHost: x\r\n\r\n",
    ] {
        assert_handed_off(&cat(&[head, b"hi", GET]), head);
    }
}

#[test]
fn http10_and_unusual_methods_are_framed() {
    assert_accepted(b"GET / HTTP/1.0\r\n\r\nGET / HTTP/1.0\r\nHost: x\r\n\r\n", 2);
    assert_accepted(b"POST / HTTP/1.0\r\nContent-Length: 2\r\n\r\nhiGET / HTTP/1.0\r\n\r\n", 2);
    assert_accepted(b"OPTIONS * HTTP/1.1\r\nHost: x\r\n\r\n", 1);
}

#[test]
fn an_oversized_head_is_handed_to_hyper() {
    let mut head = b"GET / HTTP/1.1\r\nHost: x\r\n".to_vec();
    head.extend_from_slice(&b"X-Long: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n".repeat(400));
    let o = run_pieces(8 * 1024, &[&head]);
    assert_eq!(o.ending, Some(Ending::End));
    assert!(o.out.len() >= 8 * 1024 && head.starts_with(&o.out), "{}", o.out.len());
    // Split in small reads the result is the same bytes up to the limit.
    let pieces: Vec<&[u8]> = head.chunks(100).collect();
    let o2 = run_pieces(8 * 1024, &pieces);
    assert_eq!(o2.ending, Some(Ending::End));
    assert!(head.starts_with(&o2.out) && o2.out.len() >= 8 * 1024);
}

#[test]
fn end_of_input_forwards_what_is_buffered() {
    let o = run(b"GET / HTTP/1.1\r\nHost: x\r\n");
    assert_eq!(o.out, b"GET / HTTP/1.1\r\nHost: x\r\n");
    assert_eq!(o.ending, None);
    // A lone CR is the start of an empty line: it is dropped, like the empty lines themselves.
    assert_eq!(run(b"\r").out, b"");
    assert_eq!(run(b"PRI * HT").out, b"PRI * HT");
    assert_eq!(run(b"").out, b"");
}

#[test]
fn rejection_waits_for_the_requests_before_it() {
    // A rejected head after one request: wait for that request.
    let o = run(&cat(&[GET, b"GET / HTTP/1.1\nHost: x\n\n"]));
    assert_eq!(o.wait, Some(1));
    // Mid-body of the second request: the second does not count, so wait for the first only.
    let o = run(&cat(&[GET, b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n2 \r\n"]));
    assert_eq!(o.wait, Some(1));
    // A head handed to hyper is not counted.
    let o = run(&cat(&[GET, b"GET / HTTP/1.9\r\n\r\n"]));
    assert_eq!(o.wait, Some(1));
    let o = run(b"GET / HTTP/1.1\nHost: x\n\n");
    assert_eq!(o.wait, Some(0));
}

// ---------------------------------------------------------------------------
// HTTP/2 preface, upgrades
// ---------------------------------------------------------------------------

#[test]
fn the_http2_preface_is_passed_through() {
    let mut s = PREFACE.to_vec();
    s.extend_from_slice(b"\0\0\0\x04\0\0\0\0\0 anything \n \r\0 at all");
    let o = assert_split_invariant(&s);
    assert_eq!(o.out, s);
    assert_eq!(o.ending, None);
    // A prefix of the preface that stops matching is a plain request head.
    assert_accepted(b"PRI * HTTP/1.1\r\nHost: x\r\n\r\n", 1);
    assert_rejected(b"PRI * HTTP/2.0\r\n\nSM\r\n\r\n", b"");
}

#[test]
fn bytes_after_an_upgrade_request_are_held() {
    let up = b"GET /ws HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n";
    let o = run(&cat(&[up, b"\x81\x05hello"]));
    assert!(o.holding);
    assert_eq!(o.out, up);
    assert_eq!(o.completed, 1);

    // Answered with 101: the held bytes and everything after them go through unchanged.
    let mut fr = Framing::new(32 * 1024);
    let mut out = Vec::new();
    fr.feed(&cat(&[up, b"\x81\x05hel"]), &mut out);
    fr.feed(b"lo\n\r\0", &mut out);
    assert_eq!(out, up);
    fr.release(&mut out);
    assert_eq!(&out[up.len()..], b"\x81\x05hello\n\r\0");
    assert_eq!(fr.passthrough_prefix(b"more"), 4);
    assert!(fr.pending_terminal().is_none());

    // Answered with something else: the held bytes are the next request, checked as one.
    let mut fr = Framing::new(32 * 1024);
    let mut out = Vec::new();
    fr.feed(&cat(&[up, b"GET /b HTTP/1.1\nHost: x\n\n", GET]), &mut out);
    assert_eq!(out, up);
    fr.resume(&mut out);
    assert_eq!(fr.pending_terminal().map(|t| (t.ending, t.wait)), Some((Ending::Reject, 1)));
    assert_eq!(&out[up.len()..], b"");

    let mut fr = Framing::new(32 * 1024);
    let mut out = Vec::new();
    fr.feed(&cat(&[up, GET]), &mut out);
    fr.resume(&mut out);
    assert_eq!(out, cat(&[up, GET]));
    assert_eq!(fr.log.msgs.len(), 2);
}

#[test]
fn an_upgrade_request_with_a_body_holds_after_the_body() {
    let up = b"POST /ws HTTP/1.1\r\nHost: x\r\nUpgrade: foo\r\nContent-Length: 3\r\n\r\nabc";
    let o = run(&cat(&[up, b"rest"]));
    assert!(o.holding);
    assert_eq!(o.out, up);
    let connect = b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\n\r\n";
    let o = run(&cat(&[connect, b"tunnel bytes"]));
    assert!(o.holding);
    assert_eq!(o.out, connect);
}

#[test]
fn held_bytes_are_bounded_by_the_cap_the_reader_checks() {
    let up = b"GET /ws HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n";
    let mut fr = Framing::new(32 * 1024);
    let mut out = Vec::new();
    fr.feed(up, &mut out);
    assert!(!fr.hold_full());
    fr.feed(&vec![0u8; HOLD_CAP], &mut out);
    assert!(fr.hold_full());
}

// ---------------------------------------------------------------------------
// hyper (the version in Cargo.lock) rejects the two bytes of a rejection in every position
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    method: String,
    path: String,
    body: Vec<u8>,
    trailers: Vec<(String, String)>,
}

struct HyperRun {
    /// Requests whose body was read to its end.
    seen: Vec<Seen>,
    /// Requests whose body failed.
    body_errors: usize,
    /// How many times the service was called.
    calls: usize,
    response: Vec<u8>,
    ok: bool,
}

async fn run_hyper(bytes: Vec<u8>) -> HyperRun {
    let log: Arc<Mutex<(Vec<Seen>, usize, usize)>> = Arc::default();
    let (mut client, server) = tokio::io::duplex(4 * 1024 * 1024);
    let svc_log = log.clone();
    let service = service_fn(move |req: hyper::Request<Incoming>| {
        let log = svc_log.clone();
        async move {
            log.lock().unwrap().2 += 1;
            let (parts, body) = req.into_parts();
            let (status, bytes) = match body.collect().await {
                Ok(collected) => {
                    let trailers = collected
                        .trailers()
                        .map(|t| t.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("?").to_string())).collect())
                        .unwrap_or_default();
                    log.lock().unwrap().0.push(Seen {
                        method: parts.method.to_string(),
                        path: parts.uri.path().to_string(),
                        body: collected.to_bytes().to_vec(),
                        trailers,
                    });
                    (200, "ok")
                }
                Err(_) => {
                    log.lock().unwrap().1 += 1;
                    (400, "body error")
                }
            };
            Ok::<_, std::convert::Infallible>(hyper::Response::builder().status(status).body(Full::new(Bytes::from(bytes))).unwrap())
        }
    });
    let conn = tokio::spawn(async move {
        let r = hyper::server::conn::http1::Builder::new().half_close(true).serve_connection(TokioIo::new(server), service).await;
        if let Err(e) = &r {
            eprintln!("hyper: {e:?}");
        }
        r.is_ok()
    });
    let _ = client.write_all(&bytes).await;
    let _ = client.shutdown().await;
    let mut response = Vec::new();
    let _ = client.read_to_end(&mut response).await;
    let ok = conn.await.unwrap_or(false);
    let (seen, body_errors, calls) = std::mem::take(&mut *log.lock().unwrap());
    HyperRun { seen, body_errors, calls, response, ok }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
}

#[test]
fn hyper_answers_400_to_the_rejection_bytes_at_a_head() {
    let r = block_on(run_hyper(b"\r\0".to_vec()));
    assert!(r.response.starts_with(b"HTTP/1.1 400"), "{:?}", String::from_utf8_lossy(&r.response));
    assert_eq!(r.calls, 0);
    // Also after a request that was answered before.
    let r = block_on(run_hyper(cat(&[GET, b"\r\0"])));
    let text = String::from_utf8_lossy(&r.response).into_owned();
    assert!(text.starts_with("HTTP/1.1 200") && text.contains("HTTP/1.1 400"), "{text:?}");
    assert_eq!(r.calls, 1);
}

#[test]
fn hyper_fails_the_body_on_the_rejection_bytes_in_every_chunked_state() {
    let head = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
    for prefix in [
        &b""[..],                  // chunk size, no digit yet
        b"2",                      // chunk size digits
        b"2;a",                    // chunk extension
        b"2\r",                    // the LF of the size line
        b"2\r\nhi",                // the CR after the chunk data
        b"2\r\nhi\r",              // the LF after the data
        b"2\r\nhi\r\n0",           // the last chunk size
        b"2\r\nhi\r\n0\r\n",       // start of the trailer section
        b"2\r\nhi\r\n0\r\nX: a",   // inside a trailer line
        b"2\r\nhi\r\n0\r\nX: a\r", // the LF of a trailer line
        b"2\r\nhi\r\n0\r\n\r",     // the LF that ends the trailer section
    ] {
        let stream = cat(&[head, prefix, b"\r\0"]);
        let r = block_on(run_hyper(stream));
        assert_eq!(r.calls, 1, "{:?}", String::from_utf8_lossy(prefix));
        assert!(r.seen.is_empty(), "{:?}: {:?}", String::from_utf8_lossy(prefix), r.seen);
        assert_eq!(r.body_errors, 1, "{:?}", String::from_utf8_lossy(prefix));
    }
}

/// What the check forwards for a rejection that follows empty lines is answered with a `400` by hyper: nothing the
/// check forwarded may sit in hyper's buffer in front of the rejection bytes.
#[test]
fn hyper_answers_400_to_a_rejection_that_follows_empty_lines() {
    for rejected_head in
        [&b"GET / HTTP/1.1\nHost: x\n\n"[..], b"GET / HTTP/1.1\r\nHost: x\r\n\n", b"GET / HTTP/1.1\r\nHost: x\rX: y\r\n\r\n"]
    {
        for empty in [&b"\r\n"[..], b"\n", b"\r\n\n\r\n"] {
            for before in [&b""[..], GET] {
                let stream = cat(&[before, empty, rejected_head]);
                // Whole, in two parts and one byte at a time: the bytes hyper gets are the same.
                let o = run(&stream);
                let r = block_on(run_hyper(o.out.clone()));
                let text = String::from_utf8_lossy(&r.response).into_owned();
                assert!(text.contains("HTTP/1.1 400"), "{stream:?}: {text:?}");
                assert_eq!(r.calls, usize::from(!before.is_empty()));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property test against hyper
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum BodySpec {
    None,
    Length(Vec<u8>),
    Chunked { chunks: Vec<(Vec<u8>, u8, u8)>, trailers: u8 },
}

/// The shape of a request beyond its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    /// `Upgrade` and `Connection: upgrade` headers: the check holds what follows until the request is answered.
    Upgrade,
    /// HTTP/1.0 with `Connection: keep-alive`.
    Http10KeepAlive,
    /// `Expect: 100-continue` with a `Content-Length` body.
    Expect,
    /// Only in mutated streams: `CONNECT`, which makes hyper end the connection after its answer.
    Connect,
    /// Only in mutated streams: the biggest `Content-Length` hyper accepts; the rest of the stream is its body.
    HugeLength,
}

#[derive(Debug, Clone)]
struct ReqSpec {
    post: bool,
    leading: u8,
    extra_headers: u8,
    kind: Kind,
    body: BodySpec,
}

fn body_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop_oneof![3 => any::<u8>(), 1 => Just(b'\r'), 1 => Just(b'\n'), 1 => Just(b'0')], 0..24)
}

fn req_spec(odd: bool) -> impl Strategy<Value = ReqSpec> {
    let chunked = (prop::collection::vec((body_bytes(), 0u8..4, 0u8..4), 0..4), 0u8..3)
        .prop_map(|(chunks, trailers)| BodySpec::Chunked { chunks, trailers });
    let body = prop_oneof![2 => Just(BodySpec::None), 2 => body_bytes().prop_map(BodySpec::Length), 3 => chunked];
    let kind = if odd {
        prop_oneof![
            6 => Just(Kind::Plain),
            2 => Just(Kind::Upgrade),
            2 => Just(Kind::Http10KeepAlive),
            2 => Just(Kind::Expect),
            1 => Just(Kind::Connect),
            1 => Just(Kind::HugeLength)
        ]
        .boxed()
    } else {
        prop_oneof![6 => Just(Kind::Plain), 2 => Just(Kind::Upgrade), 2 => Just(Kind::Http10KeepAlive), 2 => Just(Kind::Expect)].boxed()
    };
    (any::<bool>(), 0u8..3, 0u8..3, kind, body).prop_map(|(post, leading, extra_headers, kind, body)| {
        // Shapes whose framing is fixed by the kind.
        let body = match kind {
            Kind::Plain => body,
            Kind::Upgrade | Kind::Connect => BodySpec::None,
            Kind::Http10KeepAlive => match body {
                BodySpec::Chunked { .. } => BodySpec::None,
                other => other,
            },
            Kind::Expect => match body {
                BodySpec::Length(b) => BodySpec::Length(b),
                _ => BodySpec::Length(b"expect".to_vec()),
            },
            Kind::HugeLength => BodySpec::Length(Vec::new()),
        };
        let post = post || !matches!(body, BodySpec::None);
        ReqSpec { post, leading, extra_headers, kind, body }
    })
}

/// The bytes of one request, the bytes the check forwards for it (empty lines are dropped) and what hyper must record.
fn render(i: usize, spec: &ReqSpec) -> (Vec<u8>, Vec<u8>, Seen) {
    let mut lead = Vec::new();
    for k in 0..spec.leading {
        lead.extend_from_slice(if k % 2 == 0 { b"\r\n" } else { b"\n" });
    }
    let mut out = Vec::new();
    let method = match spec.kind {
        Kind::Connect => "CONNECT",
        _ if spec.post => "POST",
        _ => "GET",
    };
    let (target, path) =
        if spec.kind == Kind::Connect { (format!("h{i}.test:443"), String::new()) } else { (format!("/r{i}"), format!("/r{i}")) };
    let version = if spec.kind == Kind::Http10KeepAlive { "HTTP/1.0" } else { "HTTP/1.1" };
    out.extend_from_slice(format!("{method} {target} {version}\r\nHost: x\r\n").as_bytes());
    for h in 0..spec.extra_headers {
        out.extend_from_slice(format!("X-H{h}: v{h} w\r\n").as_bytes());
    }
    match spec.kind {
        Kind::Upgrade => out.extend_from_slice(b"Connection: upgrade\r\nUpgrade: foo\r\n"),
        Kind::Http10KeepAlive => out.extend_from_slice(b"Connection: keep-alive\r\n"),
        Kind::Expect => out.extend_from_slice(b"Expect: 100-continue\r\n"),
        _ => {}
    }
    let mut body = Vec::new();
    let mut trailers = Vec::new();
    match &spec.body {
        BodySpec::None => out.extend_from_slice(b"\r\n"),
        BodySpec::Length(_) if spec.kind == Kind::HugeLength => {
            out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", u64::MAX - 2).as_bytes());
        }
        BodySpec::Length(b) => {
            out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", b.len()).as_bytes());
            out.extend_from_slice(b);
            body = b.clone();
        }
        BodySpec::Chunked { chunks, trailers: t } => {
            out.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
            for (data, ext, style) in chunks {
                if data.is_empty() {
                    continue;
                }
                let size = match style {
                    0 => format!("{:x}", data.len()),
                    1 => format!("{:X}", data.len()),
                    2 => format!("{:0>8x}", data.len()),
                    _ => format!("{:x}", data.len()),
                };
                out.extend_from_slice(size.as_bytes());
                match ext {
                    1 => out.extend_from_slice(b";a=b"),
                    2 => out.extend_from_slice(b";q=\"x y\";z"),
                    3 => out.extend_from_slice(b";"),
                    _ => {}
                }
                out.extend_from_slice(b"\r\n");
                out.extend_from_slice(data);
                out.extend_from_slice(b"\r\n");
                body.extend_from_slice(data);
            }
            out.extend_from_slice(b"0\r\n");
            for k in 0..*t {
                out.extend_from_slice(format!("X-T{k}: tv{k}\r\n").as_bytes());
                trailers.push((format!("x-t{k}"), format!("tv{k}")));
            }
            out.extend_from_slice(b"\r\n");
        }
    }
    let wire = cat(&[&lead, &out]);
    (wire, out, Seen { method: method.to_string(), path, body, trailers })
}

#[derive(Debug, Clone)]
enum Mutation {
    Insert(prop::sample::Index, u8),
    Delete(prop::sample::Index),
    Replace(prop::sample::Index, u8),
    /// Splice a header line in front of the empty line of the head of one of the requests.
    Header(prop::sample::Index, u8),
}

fn mutation() -> impl Strategy<Value = Mutation> {
    let byte = prop_oneof![Just(b'\r'), Just(b'\n'), Just(b' '), Just(b'\t'), Just(b':'), Just(b';'), Just(b'0'), any::<u8>()];
    prop_oneof![
        (any::<prop::sample::Index>(), byte.clone()).prop_map(|(i, b)| Mutation::Insert(i, b)),
        any::<prop::sample::Index>().prop_map(Mutation::Delete),
        (any::<prop::sample::Index>(), byte).prop_map(|(i, b)| Mutation::Replace(i, b)),
        (any::<prop::sample::Index>(), 0u8..12).prop_map(|(i, k)| Mutation::Header(i, k)),
    ]
}

const HEADER_MUTATIONS: [&[u8]; 12] = [
    b"Content-Length: 3\r\n",
    b"Content-Length: 3\r\nContent-Length: 4\r\n",
    b"Content-Length: 0\r\n",
    b"Transfer-Encoding: chunked\r\n",
    b"Transfer-Encoding: chunked, chunked\r\n",
    b"Transfer-Encoding: gzip\r\n",
    b"Content-Length: 2, 2\r\n",
    b"Content-Length: 2\r\nContent-Length: 2\r\n",
    b"Content-Length: 2 \r\n",
    b"Content-Length: +2\r\n",
    b"Upgrade: x\r\n",
    b"Connection: close\r\n",
];

fn apply(stream: &mut Vec<u8>, m: &Mutation) {
    match m {
        Mutation::Insert(i, b) => {
            let at = i.index(stream.len() + 1);
            stream.insert(at, *b);
        }
        Mutation::Delete(i) if !stream.is_empty() => {
            let at = i.index(stream.len());
            stream.remove(at);
        }
        Mutation::Replace(i, b) if !stream.is_empty() => {
            let at = i.index(stream.len());
            stream[at] = *b;
        }
        Mutation::Header(i, k) => {
            // The head ends at an empty line: find the CRLF CRLF at or after a position.
            let from = i.index(stream.len() + 1);
            if let Some(p) = stream[from..].windows(4).position(|w| w == b"\r\n\r\n") {
                let at = from + p + 2;
                for (n, b) in HEADER_MUTATIONS[*k as usize].iter().enumerate() {
                    stream.insert(at + n, *b);
                }
            }
        }
        _ => {}
    }
}

/// Splits `stream` at the given (unsorted) fractions.
fn split_at_points<'a>(stream: &'a [u8], cuts: &[prop::sample::Index]) -> Vec<&'a [u8]> {
    let mut points: Vec<usize> = cuts.iter().map(|c| c.index(stream.len() + 1)).collect();
    points.sort_unstable();
    let mut pieces = Vec::new();
    let mut last = 0;
    for p in points {
        pieces.push(&stream[last..p]);
        last = p;
    }
    pieces.push(&stream[last..]);
    pieces
}

fn config() -> ProptestConfig {
    let mut cfg = ProptestConfig::default();
    if std::env::var("PROPTEST_CASES").is_err() {
        cfg.cases = 400;
    }
    cfg
}

// What hyper may refuse that the check does not look at: header and URI syntax. A forwarded head or trailer section
// that fails this independent grammar check excuses an error response of hyper; anything else does not.

fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn header_line_is_clean(line: &[u8]) -> bool {
    let Some(colon) = line.iter().position(|&b| b == b':') else { return false };
    let (name, value) = (&line[..colon], &line[colon + 1..]);
    !name.is_empty() && name.iter().all(|&b| is_tchar(b)) && value.iter().all(|&b| b == b'\t' || (0x20..0x7f).contains(&b) || b >= 0x80)
}

fn head_is_clean(head: &[u8]) -> bool {
    let mut lines = head.split(|&b| b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));
    let Some(first) = lines.next() else { return false };
    let parts: Vec<&[u8]> = first.split(|&b| b == b' ').collect();
    if parts.len() != 3 || !(parts[2] == b"HTTP/1.1" || parts[2] == b"HTTP/1.0") {
        return false;
    }
    let (method, target) = (parts[0], parts[1]);
    if method.is_empty() || !method.iter().all(|&b| is_tchar(b)) {
        return false;
    }
    let target_ok = if method == b"CONNECT" {
        // `host:port`
        let mut parts = target.splitn(2, |&b| b == b':');
        let (host, port) = (parts.next().unwrap_or(&[]), parts.next().unwrap_or(&[]));
        !host.is_empty()
            && host.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'.')
            && !port.is_empty()
            && port.iter().all(u8::is_ascii_digit)
    } else {
        (target == b"*" || target.first() == Some(&b'/')) && target.iter().all(|&b| b.is_ascii_alphanumeric() || b"/_;.-".contains(&b))
    };
    target_ok && lines.filter(|l| !l.is_empty()).all(header_line_is_clean)
}

fn trailers_are_clean(trailers: &[u8]) -> bool {
    trailers.split(|&b| b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l)).filter(|l| !l.is_empty()).all(header_line_is_clean)
}

/// `(method, path)` of a forwarded head, when its request target is plain.
fn request_of(head: &[u8]) -> Option<(String, String)> {
    let line = head.split(|&b| b == b'\r').next()?;
    let mut parts = line.split(|&b| b == b' ');
    let (method, target) = (parts.next()?, parts.next()?);
    let method = std::str::from_utf8(method).ok()?;
    if method == "CONNECT" {
        return Some((method.to_string(), String::new()));
    }
    let target = std::str::from_utf8(target).ok()?;
    target.starts_with('/').then(|| (method.to_string(), target.to_string()))
}

fn status_codes(response: &[u8]) -> Vec<u16> {
    let mut codes = Vec::new();
    let mut i = 0;
    while i + 12 <= response.len() {
        if response[i..].starts_with(b"HTTP/1.1 ")
            && let Some(n) = std::str::from_utf8(&response[i + 9..i + 12]).ok().and_then(|s| s.parse::<u16>().ok())
        {
            codes.push(n);
            i += 12;
            continue;
        }
        i += 1;
    }
    codes
}

proptest! {
    #![proptest_config(config())]

    /// Streams the check accepts entirely are read by hyper as exactly the requests that were generated.
    #[test]
    fn unmutated_streams_are_accepted_and_read_the_same(
        specs in prop::collection::vec(req_spec(false), 1..4),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..6),
    ) {
        let mut stream = Vec::new();
        let mut forwarded = Vec::new();
        let mut want = Vec::new();
        for (i, spec) in specs.iter().enumerate() {
            let (wire, fwd, seen) = render(i, spec);
            stream.extend_from_slice(&wire);
            forwarded.extend_from_slice(&fwd);
            want.push(seen);
        }
        let pieces = split_at_points(&stream, &cuts);
        // A held connection (an `Upgrade` request) is resumed, as if the request was answered without `101`.
        let o = run_pieces_with(32 * 1024, &pieces, true);
        prop_assert_eq!(o.ending, None);
        prop_assert_eq!(&o.out, &forwarded);
        prop_assert_eq!(o.completed, specs.len() as u64);
        for (msg, seen) in o.msgs.iter().zip(&want) {
            prop_assert_eq!(&msg.body, &seen.body);
        }
        let r = block_on(run_hyper(o.out));
        prop_assert!(r.ok);
        prop_assert_eq!(r.body_errors, 0);
        prop_assert!(status_codes(&r.response).iter().all(|&c| c < 400), "{:?}", String::from_utf8_lossy(&r.response));
        prop_assert_eq!(r.seen, want);
    }

    /// For mutated streams: hyper reads the requests the check completed, where the check says they start and with the
    /// bodies it saw, never one more. A response of hyper with an error status, or a failed body, must be explained
    /// by something the check does not look at (header or URI syntax) or by the rejection itself; otherwise the two
    /// parsers disagree.
    #[test]
    fn mutated_streams_are_read_by_hyper_as_the_check_reads_them(
        specs in prop::collection::vec(req_spec(true), 1..4),
        mutations in prop::collection::vec(mutation(), 0..4),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..6),
    ) {
        let mut stream = Vec::new();
        for (i, spec) in specs.iter().enumerate() {
            stream.extend_from_slice(&render(i, spec).0);
        }
        for m in &mutations {
            apply(&mut stream, m);
        }
        let pieces = split_at_points(&stream, &cuts);
        let o = run_pieces_with(32 * 1024, &pieces, true);
        // The check gives the same answer for any read boundaries.
        prop_assert_eq!(&run_pieces_with(32 * 1024, &[&stream], true), &o);
        let r = block_on(run_hyper(o.out.clone()));
        let statuses = status_codes(&r.response);
        let seen = r.seen.len();

        // Hyper reads a request only where the check saw one (a head handed to hyper may be served by hyper).
        let bound = o.completed as usize + usize::from(o.ending == Some(Ending::End));
        prop_assert!(seen <= bound, "hyper read more requests ({seen}) than the check completed ({})", o.completed);
        // Same starts and bodies.
        for (i, s) in r.seen.iter().enumerate() {
            let Some(msg) = o.msgs.get(i) else { continue };
            if let Some((method, path)) = request_of(&msg.head)
                && head_is_clean(&msg.head)
            {
                prop_assert_eq!(&s.method, &method);
                prop_assert_eq!(&s.path, &path.split('?').next().unwrap_or("").to_string());
            }
            prop_assert_eq!(&s.body, &msg.body, "request {}", i);
        }

        // Error responses: the rejection and a handed-over head explain one; unclean syntax explains one more.
        let unclean_head = o.heads.iter().skip(seen).any(|h| !head_is_clean(h));
        let unclean_trailers = o.msgs.iter().skip(seen).any(|m| !trailers_are_clean(&m.trailers));
        let errors = statuses.iter().filter(|&&c| c >= 400).count().saturating_sub(r.body_errors);
        let explained = usize::from(o.ending.is_some()) + usize::from(unclean_head) + usize::from(o.partial);
        prop_assert!(errors <= explained, "hyper answered {statuses:?} (body errors {}), check ending {:?}, heads {:?}", r.body_errors, o.ending, o.heads);
        // A failed body: the rejection, a message still open at the end of the stream, or a bad trailer.
        let open = o.heads.len() > o.msgs.len();
        prop_assert!(
            r.body_errors == 0 || o.ending.is_some() || open || unclean_trailers || unclean_head,
            "hyper failed {} bodies, check ending {:?}", r.body_errors, o.ending
        );
        // Without any error on either side hyper reads exactly what the check completed, except where hyper stops
        // reading by design: HTTP/1.0 without keep-alive, `Connection: close` and `CONNECT` end the connection.
        let stops = o.out.windows(8).any(|w| w == b"HTTP/1.0")
            || o.out.windows(8).any(|w| w.eq_ignore_ascii_case(b"CONNECT "))
            || o.out.windows(17).any(|w| w.eq_ignore_ascii_case(b"connection: close"));
        if r.ok && errors == 0 && r.body_errors == 0 && o.ending.is_none() && !stops {
            prop_assert_eq!(seen as u64, o.completed);
        }
        // A rejected stream reaches a handler at most up to the request that was in progress.
        if o.ending == Some(Ending::Reject) {
            prop_assert!(r.calls as u64 <= o.completed + 1);
        }
    }
}
