//! TLS termination in the built-in server (`tls` feature): defaults, ALPN, the handshake limits, the options of
//! `ServerOptions` over TLS, and the stalled-client protections (send timeout, watchdog, response cap) over TLS.
//! The certificates are generated per run (see `tls_common`).
#![cfg(feature = "tls")]

mod tls_common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use corvis::ServerOptions;
use corvis::prelude::*;
use corvis::testing::request;
use corvis::tls::TlsOptions;
use corvis::tls::rustls::crypto::CryptoProvider;
use corvis::tls::rustls::pki_types::pem::PemObject;
use corvis::tls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use corvis::tls::rustls::server::ResolvesServerCertUsingSni;
use corvis::tls::rustls::sign::CertifiedKey;
use corvis::tls::rustls::{ClientConnection, RootCertStore, ServerConfig};
use tls_common::*;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

const MIB: usize = 1024 * 1024;
const ENHANCE_YOUR_CALM: u32 = 0xb;

#[controller("t")]
pub struct TController;

#[routes]
impl TController {
    #[get("cats")]
    fn cats(&self) -> Value {
        json!([{ "name": "Tom" }, { "name": "Garfield" }])
    }

    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }

    #[post("json")]
    fn json(&self, #[body] body: Value) -> Value {
        body
    }

    /// 256 KiB with a length.
    #[get("quarter")]
    fn quarter(&self) -> String {
        "a".repeat(256 * 1024)
    }

    /// 256 KiB after half a second: a burst of these runs its handlers all at once.
    #[get("slow-quarter")]
    async fn slow_quarter(&self) -> String {
        tokio::time::sleep(Duration::from_millis(500)).await;
        "a".repeat(256 * 1024)
    }

    /// 8 MiB with a length.
    #[get("huge")]
    fn huge(&self) -> String {
        "a".repeat(8 * MIB)
    }
}

#[module(controllers = [TController])]
pub struct AppModule;

async fn app() -> CorvisApplication {
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

/// A TLS listener (the application lives as long as the process).
async fn serve(tls: TlsOptions, options: ServerOptions) -> SocketAddr {
    let mut app = app().await;
    app.set_server_options(options);
    let addr = app.listen_tls_in_background("127.0.0.1:0", tls).await.unwrap();
    std::mem::forget(app);
    addr
}

async fn serve_default(pki: &Pki) -> SocketAddr {
    serve(options(pki), ServerOptions::new()).await
}

/// No timeout may end a connection that is not misbehaving on the read side.
fn quiet(options: ServerOptions) -> ServerOptions {
    options.h2c(false).send_timeout(None).keep_alive_timeout(None).headers_timeout(None).request_timeout(None)
}

async fn until(what: &str, secs: u64, mut done: impl FnMut() -> bool) {
    let t = Instant::now();
    while !done() {
        assert!(t.elapsed() < Duration::from_secs(secs), "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A valid ClientHello (from rustls) cut after `keep` bytes.
fn partial_client_hello(pki: &Pki, keep: usize) -> Vec<u8> {
    let mut conn = ClientConnection::new(client_config(pki, H1), server_name()).unwrap();
    let mut hello = Vec::new();
    conn.write_tls(&mut hello).unwrap();
    hello.truncate(keep);
    hello
}

// ---- Loading and defaults ----

#[corvis::test]
async fn the_client_receives_the_whole_chain() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    let tls = connect(addr, &pki, H1).await;
    let certs = tls.get_ref().1.peer_certificates().expect("the server sent certificates");
    assert_eq!(certs.len(), 2, "leaf and intermediate");
    assert_eq!(certs[0], pki.leaf);
}

#[corvis::test]
async fn a_ready_configuration_with_sni_certificates_serves_the_name_the_client_sends() {
    let (a, b) = (pki_for(&["a.test"]), pki_for(&["b.test"]));
    let provider = Arc::new(corvis::tls::rustls::crypto::ring::default_provider());
    let key_of = |p: &Pki| -> Arc<CertifiedKey> {
        let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(p.chain_pem.as_bytes()).map(|c| c.unwrap()).collect();
        let key = PrivateKeyDer::from_pem_slice(p.key_pem.as_bytes()).unwrap();
        Arc::new(CertifiedKey::from_der(chain, key, &provider).unwrap())
    };
    let mut resolver = ResolvesServerCertUsingSni::new();
    resolver.add("a.test", (*key_of(&a)).clone()).unwrap();
    resolver.add("b.test", (*key_of(&b)).clone()).unwrap();
    let config = ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(resolver));
    let addr = serve(TlsOptions::from_server_config(Arc::new(config)), ServerOptions::new()).await;

    let mut roots = RootCertStore::empty();
    roots.add(a.root.clone()).unwrap();
    roots.add(b.root.clone()).unwrap();
    let client = corvis::tls::rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
    let tls = connector.connect(ServerName::try_from("b.test").unwrap(), TcpStream::connect(addr).await.unwrap()).await.unwrap();
    assert_eq!(tls.get_ref().1.peer_certificates().unwrap()[0], b.leaf);
}

#[corvis::test]
async fn no_provider_installed_builds_options_and_handshakes_with_ring() {
    assert!(CryptoProvider::get_default().is_none(), "this test binary installs no provider");
    let pki = pki();
    let addr = serve(TlsOptions::from_pem(pki.chain_pem.as_bytes(), pki.key_pem.as_bytes()).unwrap(), ServerOptions::new()).await;
    let r = https_get(addr, &pki, "/t/ok", false).await;
    assert_eq!(r.status, 200);
}

#[corvis::test]
async fn a_tls_1_1_client_gets_no_http() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    // A ClientHello that offers TLS 1.1 only (legacy_version 0x0302, one TLS 1.0-era suite, no extensions).
    let mut hello = vec![0x01, 0x00, 0x00, 0x29, 0x03, 0x02];
    hello.extend([7u8; 32]);
    hello.extend([0x00, 0x00, 0x02, 0x00, 0x2f, 0x01, 0x00]);
    let mut record = vec![0x16, 0x03, 0x01, 0x00, hello.len() as u8];
    record.extend(hello);
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(&record).await.unwrap();
    let got = read_all(&mut s, Duration::from_secs(5)).await;
    assert!(!got.starts_with(b"HTTP/"), "{got:?}");
    assert!(got.is_empty() || got[0] == 0x15, "the server answers with an alert or closes: {got:?}");
}

#[corvis::test]
async fn default_alpn_negotiates_h2() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    let tls = connect(addr, &pki, H2).await;
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
}

#[corvis::test]
async fn http2_off_negotiates_http_1_1() {
    let pki = pki();
    let addr = serve(options(&pki).http2(false), ServerOptions::new()).await;
    let tls = connect(addr, &pki, H2).await;
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
}

#[corvis::test]
async fn a_client_without_sni_gets_the_configured_certificate() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    // `connect` addresses the server by IP: no server name is sent.
    let tls = connect(addr, &pki, H1).await;
    assert_eq!(tls.get_ref().1.peer_certificates().unwrap()[0], pki.leaf);
}

// ---- The handshake cannot block the server ----

#[corvis::test]
async fn a_stalled_handshake_does_not_block_a_second_client_and_is_closed_at_the_timeout() {
    let pki = pki();
    let addr = serve(options(&pki).handshake_timeout(Duration::from_millis(500)), ServerOptions::new()).await;
    let mut stalled = TcpStream::connect(addr).await.unwrap();
    let t = Instant::now();
    let r = https_get(addr, &pki, "/t/ok", false).await;
    assert_eq!(r.status, 200);
    assert!(t.elapsed() < Duration::from_millis(400), "the second client waited for the first: {:?}", t.elapsed());
    assert!(ends_within(&mut stalled, Duration::from_secs(5)).await, "the stalled handshake was not closed");
    assert!(t.elapsed() >= Duration::from_millis(400), "closed before the timeout");
}

#[corvis::test]
async fn half_a_client_hello_is_closed_at_the_timeout() {
    let pki = pki();
    let addr = serve(options(&pki).handshake_timeout(Duration::from_millis(500)), ServerOptions::new()).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(&partial_client_hello(&pki, 20)).await.unwrap();
    let t = Instant::now();
    let got = read_all(&mut s, Duration::from_secs(5)).await;
    assert!(got.is_empty(), "no data goes back to a client that did not finish its handshake: {got:?}");
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
}

#[corvis::test]
async fn handshakes_count_against_max_connections() {
    let pki = pki();
    let addr = serve(options(&pki).handshake_timeout(Duration::from_secs(1)), ServerOptions::new().max_connections(1)).await;
    let mut stalled = TcpStream::connect(addr).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let t = Instant::now();
    let r = https_get(addr, &pki, "/t/ok", false).await;
    assert_eq!(r.status, 200);
    assert!(
        t.elapsed() >= Duration::from_millis(700),
        "the second connection was served while the first was handshaking: {:?}",
        t.elapsed()
    );
    assert!(ends_within(&mut stalled, Duration::from_secs(3)).await);
}

#[corvis::test]
async fn plain_http_to_the_tls_port_is_closed_without_a_response() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET /t/ok HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let got = read_all(&mut s, Duration::from_secs(5)).await;
    assert!(!got.starts_with(b"HTTP/"), "{:?}", String::from_utf8_lossy(&got));
}

// ---- The protocol comes from ALPN ----

#[corvis::test]
async fn http2_over_alpn_h2() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    let r = https_get(addr, &pki, "/t/cats", true).await;
    assert_eq!(r.version, http::Version::HTTP_2);
    assert_eq!(r.status, 200);
}

#[corvis::test]
async fn http1_over_alpn_http_1_1() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    let r = https_get(addr, &pki, "/t/cats", false).await;
    assert_eq!(r.version, http::Version::HTTP_11);
    assert_eq!(r.status, 200);
}

#[corvis::test]
async fn the_http2_preface_without_alpn_is_closed_even_with_h2c() {
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new().h2c(true)).await;
    let mut tls = connect(addr, &pki, &[]).await;
    assert_eq!(tls.get_ref().1.alpn_protocol(), None);
    let mut bytes = PREFACE.to_vec();
    bytes.extend(frame_bytes(4, 0, 0, &[]));
    send(&mut tls, &bytes).await;
    let got = read_all(&mut tls, Duration::from_secs(5)).await;
    assert!(got.is_empty(), "{got:?}");
}

async fn first_line(s: &mut (impl tokio::io::AsyncRead + Unpin)) -> String {
    let got = read_all(s, Duration::from_secs(5)).await;
    String::from_utf8_lossy(&got).lines().next().unwrap_or("").to_string()
}

const AMBIGUOUS: &[u8] =
    b"POST /t/json HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\n";

#[corvis::test]
async fn the_framing_check_answers_the_same_over_tls_as_over_plain() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    let mut plain_app = app().await;
    let plain = plain_app.listen_in_background("127.0.0.1:0").await.unwrap();

    let mut p = TcpStream::connect(plain).await.unwrap();
    p.write_all(AMBIGUOUS).await.unwrap();
    let plain_line = first_line(&mut p).await;

    let mut tls = connect(addr, &pki, H1).await;
    send(&mut tls, AMBIGUOUS).await;
    let tls_line = first_line(&mut tls).await;
    assert!(plain_line.starts_with("HTTP/1.1 400"), "{plain_line}");
    assert_eq!(tls_line, plain_line);
}

#[corvis::test]
async fn the_headers_timeout_counts_from_the_end_of_the_handshake() {
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new().headers_timeout(Duration::from_secs(1))).await;
    let mut tls = connect(addr, &pki, H1).await;
    let t = Instant::now();
    assert!(ends_within(&mut tls, Duration::from_secs(5)).await, "the connection was not closed");
    let e = t.elapsed();
    assert!(e >= Duration::from_millis(700) && e < Duration::from_secs(4), "{e:?}");
}

// ---- Same application over plain and TLS ----

#[corvis::test]
async fn a_tls_listener_answers_like_a_plain_one() {
    let pki = pki();
    let mut app = app().await;
    let plain = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let secure = app.listen_tls_in_background("127.0.0.1:0", options(&pki)).await.unwrap();
    std::mem::forget(app);

    let over_tls = https_get(secure, &pki, "/t/cats", false).await;
    let mut s = TcpStream::connect(plain).await.unwrap();
    s.write_all(b"GET /t/cats HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await.unwrap();
    let raw = String::from_utf8(read_all(&mut s, Duration::from_secs(5)).await).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(over_tls.status, 200);
    assert_eq!(over_tls.body.as_ref(), body.as_bytes());
    let ctype = over_tls.headers.get("content-type").unwrap().to_str().unwrap();
    assert!(head.to_lowercase().contains(&format!("content-type: {ctype}")), "{head}");
}

#[corvis::test]
async fn posture_enforced_stops_listen_tls_before_the_port_opens() {
    #[controller("open")]
    pub struct OpenController;

    #[routes]
    impl OpenController {
        #[get("x")]
        fn x(&self) -> Value {
            json!({})
        }
    }

    #[module(controllers = [OpenController])]
    pub struct OpenModule;

    let pki = pki();
    let module = corvis::testing::Test::create_testing_module().import::<OpenModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.silence_route_logs();
    app.require_route_posture();
    let err = app.listen_tls_in_background("127.0.0.1:0", options(&pki)).await.unwrap_err();
    assert!(err.to_string().starts_with("Route posture check failed"), "{err}");
    assert!(app.get_url().is_none(), "nothing was bound");
}

#[corvis::test]
async fn get_url_says_https_after_a_tls_listener() {
    let pki = pki();
    let mut app = app().await;
    let addr = app.listen_tls_in_background("127.0.0.1:0", options(&pki)).await.unwrap();
    assert_eq!(app.get_url(), Some(format!("https://127.0.0.1:{}", addr.port())));
    let plain = app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert_eq!(app.get_url(), Some(format!("http://127.0.0.1:{}", plain.port())));
    let _ = request(&app);
}

// ---- Options and protections that must hold over TLS ----

/// What a raw HTTP/2 client over TLS has seen.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Seen {
    Headers(u32),
    Data(u32, usize),
    Reset(u32, u32),
}

struct H2Raw {
    wr: Arc<tokio::sync::Mutex<tokio::io::WriteHalf<tokio_rustls::client::TlsStream<TcpStream>>>>,
    closed: Arc<AtomicBool>,
    seen: Arc<std::sync::Mutex<Vec<Seen>>>,
    /// The server's SETTINGS (identifier, value).
    settings: Arc<std::sync::Mutex<Vec<(u16, u32)>>>,
    next_stream: std::sync::Mutex<u32>,
}

impl H2Raw {
    async fn connect(addr: SocketAddr, pki: &Pki, settings: &[u8]) -> H2Raw {
        Self::over(connect(addr, pki, H2).await, settings).await
    }

    async fn over(tls: tokio_rustls::client::TlsStream<TcpStream>, settings: &[u8]) -> H2Raw {
        let (mut rd, mut wr) = tokio::io::split(tls);
        wr.write_all(PREFACE).await.unwrap();
        wr.write_all(&frame_bytes(4, 0, 0, settings)).await.unwrap();
        wr.flush().await.unwrap();
        let wr = Arc::new(tokio::sync::Mutex::new(wr));
        let closed = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let server_settings = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (w, c, s, ss) = (wr.clone(), closed.clone(), seen.clone(), server_settings.clone());
        tokio::spawn(async move {
            while let Some(f) = read_frame(&mut rd).await {
                match f.kind {
                    0 => s.lock().unwrap().push(Seen::Data(f.stream, f.payload.len())),
                    1 => s.lock().unwrap().push(Seen::Headers(f.stream)),
                    3 => {
                        let code = u32::from_be_bytes(f.payload[..4].try_into().unwrap());
                        s.lock().unwrap().push(Seen::Reset(f.stream, code));
                    }
                    4 if f.flags & 1 == 0 => {
                        let mut out = ss.lock().unwrap();
                        for e in f.payload.as_chunks::<6>().0 {
                            out.push((u16::from_be_bytes([e[0], e[1]]), u32::from_be_bytes([e[2], e[3], e[4], e[5]])));
                        }
                    }
                    _ => {}
                }
                let mut w = w.lock().await;
                if f.kind == 6 && f.flags & 1 == 0 && w.write_all(&frame_bytes(6, 1, 0, &f.payload)).await.is_err() {
                    break;
                }
                if f.kind == 4 && f.flags & 1 == 0 && w.write_all(&frame_bytes(4, 1, 0, &[])).await.is_err() {
                    break;
                }
                let _ = w.flush().await;
            }
            c.store(true, Ordering::SeqCst);
        });
        H2Raw { wr, closed, seen, settings: server_settings, next_stream: std::sync::Mutex::new(1) }
    }

    async fn send(&self, bytes: &[u8]) {
        let mut w = self.wr.lock().await;
        let _ = w.write_all(bytes).await;
        let _ = w.flush().await;
    }

    async fn open(&self, n: usize, path: &str) -> Vec<u32> {
        let mut ids = Vec::new();
        let mut buf = Vec::new();
        {
            let mut next = self.next_stream.lock().unwrap();
            for _ in 0..n {
                buf.extend(frame_bytes(1, 0x5, *next, &request_block(path)));
                ids.push(*next);
                *next += 2;
            }
        }
        self.send(&buf).await;
        ids
    }

    fn headers(&self, ids: &[u32]) -> Vec<u32> {
        let seen = self.seen.lock().unwrap();
        ids.iter().copied().filter(|id| seen.contains(&Seen::Headers(*id))).collect()
    }

    fn resets(&self, ids: &[u32], code: u32) -> usize {
        let seen = self.seen.lock().unwrap();
        ids.iter().filter(|id| seen.contains(&Seen::Reset(**id, code))).count()
    }

    fn data_of(&self, stream: u32) -> usize {
        self.seen.lock().unwrap().iter().map(|s| if let Seen::Data(id, n) = s { if *id == stream { *n } else { 0 } } else { 0 }).sum()
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

const SETTINGS_WINDOW_0: &[u8] = &[0, 4, 0, 0, 0, 0];

#[corvis::test]
async fn http2_limits_of_the_connection_builder_apply_over_tls() {
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new().http2_max_concurrent_streams(7).max_header_size(16 * 1024)).await;
    let c = H2Raw::connect(addr, &pki, &[]).await;
    until("the server settings", 5, || !c.settings.lock().unwrap().is_empty()).await;
    let s = c.settings.lock().unwrap().clone();
    assert!(s.contains(&(3, 7)), "SETTINGS_MAX_CONCURRENT_STREAMS: {s:?}");
    assert!(s.contains(&(6, 16 * 1024)), "SETTINGS_MAX_HEADER_LIST_SIZE: {s:?}");
}

#[corvis::test]
async fn http2_over_tls_keeps_the_keep_alive_phase_until_a_request_arrives() {
    let pki = pki();
    // Only HEADERS frames end the keep-alive phase: PINGs after the response must not extend the connection.
    let addr = serve(options(&pki), ServerOptions::new().keep_alive_timeout(Duration::from_secs(1))).await;
    let c = H2Raw::connect(addr, &pki, &[]).await;
    let ids = c.open(1, "/t/ok").await;
    until("the response", 5, || c.data_of(ids[0]) > 0).await;
    let t = Instant::now();
    let mut n = 0u64;
    while !c.is_closed() && t.elapsed() < Duration::from_secs(5) {
        c.send(&frame_bytes(6, 0, 0, &n.to_be_bytes())).await;
        n += 1;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(c.is_closed(), "PINGs kept an idle HTTP/2 connection open");
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
}

/// The send timeout cuts a TLS client that does not read an HTTP/1 response, and frees its slot.
#[corvis::test]
async fn a_tls_client_that_never_reads_an_http1_response_is_cut() {
    let pki = pki();
    let opts = quiet(ServerOptions::new()).send_timeout(Duration::from_secs(1)).max_connections(1);
    let addr = serve(options(&pki), opts).await;
    let mut stalled = connect_tiny(addr, &pki, H1).await;
    send(&mut stalled, b"GET /t/huge HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let t = Instant::now();
    let r = tokio::time::timeout(Duration::from_secs(15), https_get(addr, &pki, "/t/ok", false)).await;
    let r = r.expect("the stalled TLS connection kept its slot");
    assert_eq!(r.status, 200);
    assert!(t.elapsed() < Duration::from_secs(12), "{:?}", t.elapsed());
    drop(stalled);
}

/// The watchdog cuts an HTTP/2 connection over TLS whose windows stay closed.
#[corvis::test]
async fn a_tls_http2_client_that_never_opens_its_window_is_cut() {
    let pki = pki();
    let addr = serve(options(&pki), quiet(ServerOptions::new()).send_timeout(Duration::from_secs(1))).await;
    let c = H2Raw::connect(addr, &pki, SETTINGS_WINDOW_0).await;
    let ids = c.open(1, "/t/quarter").await;
    until("the connection was cut", 10, || c.is_closed()).await;
    assert_eq!(c.data_of(ids[0]), 0, "nothing could be sent with a closed window");
}

/// Window 0: 20 streams of 256 KiB against a 1 MiB cap. Four responses are kept, the other 16 are reset with
/// ENHANCE_YOUR_CALM, and the connection survives: the cap works through TLS as it does in cleartext.
#[corvis::test]
async fn the_pending_response_cap_holds_over_tls() {
    let pki = pki();
    let opts = quiet(ServerOptions::new().http2_max_concurrent_streams(20).max_pending_response_bytes(MIB));
    let addr = serve(options(&pki), opts).await;
    let c = H2Raw::connect(addr, &pki, SETTINGS_WINDOW_0).await;
    let ids = c.open(20, "/t/slow-quarter").await;
    until("every stream was answered or reset", 10, || c.headers(&ids).len() + c.resets(&ids, ENHANCE_YOUR_CALM) >= 20).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(c.headers(&ids).len(), 4, "only what fits the cap is kept");
    assert_eq!(c.resets(&ids, ENHANCE_YOUR_CALM), 16);
    assert!(!c.is_closed());
}

#[corvis::test]
async fn http1_responses_over_tls_are_complete_for_a_reading_client() {
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new().send_timeout(Duration::from_secs(1))).await;
    let r = https_get(addr, &pki, "/t/huge", false).await;
    assert_eq!(r.body.len(), 8 * MIB);
    let r = https_get(addr, &pki, "/t/huge", true).await;
    assert_eq!(r.body.len(), 8 * MIB);
}

// ---- ALPN `h2` cannot be used to speak HTTP/1 (the framing check would be skipped) ----

#[corvis::test]
async fn an_http1_request_over_an_h2_connection_gets_no_http_bytes() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    for request in [&b"GET /t/ok HTTP/1.1\r\nHost: x\r\n\r\n"[..], b"GET /t/ok HTTP/1.1\nHost: x\n\n", AMBIGUOUS] {
        let mut tls = connect(addr, &pki, H2).await;
        assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
        send(&mut tls, request).await;
        let got = read_all(&mut tls, Duration::from_secs(5)).await;
        assert!(!got.windows(5).any(|w| w == b"HTTP/"), "{:?}", String::from_utf8_lossy(&got));
        assert!(ends_within(&mut tls, Duration::from_secs(2)).await, "the connection stayed open");
    }
}

#[corvis::test]
async fn a_websocket_upgrade_over_an_h2_connection_is_not_upgraded() {
    let pki = pki();
    let addr = serve_default(&pki).await;
    let mut tls = connect(addr, &pki, H2).await;
    let upgrade = b"GET /t/ok HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
    send(&mut tls, upgrade).await;
    let got = read_all(&mut tls, Duration::from_secs(5)).await;
    assert!(!got.windows(3).any(|w| w == b"101") && !got.windows(5).any(|w| w == b"HTTP/"), "{:?}", String::from_utf8_lossy(&got));
}

#[corvis::test]
async fn the_http2_preface_over_an_http_1_1_connection_is_closed() {
    let pki = pki();
    let addr = serve(options(&pki), ServerOptions::new().h2c(true)).await;
    let mut tls = connect(addr, &pki, H1).await;
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
    let mut bytes = PREFACE.to_vec();
    bytes.extend(frame_bytes(4, 0, 0, &[]));
    send(&mut tls, &bytes).await;
    let got = read_all(&mut tls, Duration::from_secs(5)).await;
    assert!(got.is_empty(), "{got:?}");
}
