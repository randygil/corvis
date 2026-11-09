//! Shared helpers of the TLS test binaries: a throwaway PKI (root, intermediate and leaf for `localhost` and
//! `127.0.0.1`, generated per run so no key is committed), TLS clients that trust only that root, and small HTTP
//! helpers over them.
#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use corvis::tls::TlsOptions;
use corvis::tls::rustls::ClientConfig;
use corvis::tls::rustls::RootCertStore;
use corvis::tls::rustls::crypto::ring;
use corvis::tls::rustls::pki_types::{CertificateDer, ServerName};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rcgen::{BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

pub struct Pki {
    pub root: CertificateDer<'static>,
    /// Leaf first, then the intermediate.
    pub chain_pem: String,
    pub key_pem: String,
    pub leaf: CertificateDer<'static>,
}

fn ca_params(name: &str, constrained: bool) -> CertificateParams {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.is_ca = if constrained { IsCa::Ca(BasicConstraints::Constrained(0)) } else { IsCa::Ca(BasicConstraints::Unconstrained) };
    p.distinguished_name.push(DnType::CommonName, name);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    p
}

/// A root, an intermediate and a leaf valid for `names` (DNS names; `127.0.0.1` is always added).
pub fn pki_for(names: &[&str]) -> Pki {
    let root_params = ca_params("Corvis test root", false);
    let root_key = KeyPair::generate().unwrap();
    let root = root_params.self_signed(&root_key).unwrap();
    let root_issuer = Issuer::new(root_params, root_key);

    let inter_params = ca_params("Corvis test intermediate", true);
    let inter_key = KeyPair::generate().unwrap();
    let inter = inter_params.signed_by(&inter_key, &root_issuer).unwrap();
    let inter_issuer = Issuer::new(inter_params, inter_key);

    let mut leaf_params = CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).unwrap();
    leaf_params.subject_alt_names.push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    leaf_params.distinguished_name.push(DnType::CommonName, names.first().copied().unwrap_or("localhost"));
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = leaf_params.signed_by(&leaf_key, &inter_issuer).unwrap();

    Pki {
        root: root.der().clone(),
        chain_pem: format!("{}{}", leaf.pem(), inter.pem()),
        key_pem: leaf_key.serialize_pem(),
        leaf: leaf.der().clone(),
    }
}

pub fn pki() -> Pki {
    pki_for(&["localhost"])
}

pub fn options(pki: &Pki) -> TlsOptions {
    TlsOptions::from_pem(pki.chain_pem.as_bytes(), pki.key_pem.as_bytes()).unwrap()
}

pub fn client_config(pki: &Pki, alpn: &[&[u8]]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(pki.root.clone()).unwrap();
    let mut config = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Arc::new(config)
}

pub const H1: &[&[u8]] = &[b"http/1.1"];
pub const H2: &[&[u8]] = &[b"h2", b"http/1.1"];

/// The server is addressed by IP, so no server name goes in the ClientHello.
pub fn server_name() -> ServerName<'static> {
    ServerName::from(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

pub async fn handshake(tcp: TcpStream, pki: &Pki, alpn: &[&[u8]]) -> std::io::Result<TlsStream<TcpStream>> {
    TlsConnector::from(client_config(pki, alpn)).connect(server_name(), tcp).await
}

pub async fn connect(addr: SocketAddr, pki: &Pki, alpn: &[&[u8]]) -> TlsStream<TcpStream> {
    handshake(TcpStream::connect(addr).await.unwrap(), pki, alpn).await.unwrap()
}

/// A TLS client whose socket receive buffer holds a few KiB: what it does not read blocks the server's writes.
pub async fn connect_tiny(addr: SocketAddr, pki: &Pki, alpn: &[&[u8]]) -> TlsStream<TcpStream> {
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    handshake(socket.connect(addr).await.unwrap(), pki, alpn).await.unwrap()
}

pub type B = BoxBody<Bytes, std::io::Error>;

pub fn empty() -> B {
    Empty::<Bytes>::new().map_err(|e| match e {}).boxed()
}

pub struct Reply {
    pub status: u16,
    pub version: http::Version,
    pub headers: http::HeaderMap,
    pub body: Bytes,
}

/// GET over TLS with hyper's client; HTTP/2 when `h2` is set (ALPN `h2`), HTTP/1.1 otherwise.
pub async fn https_get(addr: SocketAddr, pki: &Pki, path: &str, h2: bool) -> Reply {
    let tls = connect(addr, pki, if h2 { H2 } else { H1 }).await;
    let io = TokioIo::new(tls);
    let req = hyper::Request::get(format!("https://localhost:{}{path}", addr.port())).header("host", "localhost").body(empty()).unwrap();
    let resp = if h2 {
        let (mut sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), io).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        sender.send_request(req).await.unwrap()
    } else {
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        sender.send_request(req).await.unwrap()
    };
    let (parts, body) = resp.into_parts();
    let body = body.collect().await.unwrap().to_bytes();
    Reply { status: parts.status.as_u16(), version: parts.version, headers: parts.headers, body }
}

/// Reads what arrives until the end of the stream, an error or `wait`.
pub async fn read_all<R: tokio::io::AsyncRead + Unpin>(s: &mut R, wait: Duration) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    let _ = tokio::time::timeout(wait, async {
        loop {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
    })
    .await;
    out
}

/// `true` when the server ends the stream (end of input or an error) within `wait`; reads and discards what comes.
pub async fn ends_within<R: tokio::io::AsyncRead + Unpin>(s: &mut R, wait: Duration) -> bool {
    let mut buf = [0u8; 4096];
    tokio::time::timeout(wait, async {
        loop {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await
    .is_ok()
}

pub async fn send<W: tokio::io::AsyncWrite + Unpin>(s: &mut W, bytes: &[u8]) {
    s.write_all(bytes).await.unwrap();
    s.flush().await.unwrap();
}

// ---- Raw HTTP/2 over a stream ----

pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub fn frame_bytes(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut v = vec![(len >> 16) as u8, (len >> 8) as u8, len as u8, kind, flags];
    v.extend_from_slice(&stream.to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// HPACK block of `GET <path>` over https, authority "x" (static table indexes, no Huffman).
pub fn request_block(path: &str) -> Vec<u8> {
    let mut b = vec![0x82, 0x87, 0x04, path.len() as u8];
    b.extend_from_slice(path.as_bytes());
    b.extend_from_slice(&[0x01, 0x01, 0x78]);
    b
}

pub struct RawFrame {
    pub kind: u8,
    pub flags: u8,
    pub stream: u32,
    pub payload: Vec<u8>,
}

pub async fn read_frame<R: tokio::io::AsyncRead + Unpin>(rd: &mut R) -> Option<RawFrame> {
    let mut head = [0u8; 9];
    rd.read_exact(&mut head).await.ok()?;
    let len = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
    let mut payload = vec![0u8; len];
    rd.read_exact(&mut payload).await.ok()?;
    Some(RawFrame { kind: head[3], flags: head[4], stream: u32::from_be_bytes([head[5] & 0x7f, head[6], head[7], head[8]]), payload })
}
