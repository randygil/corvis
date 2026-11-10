//! A raw Engine.IO WebSocket client for the memory tests: frames are written straight from the caller's buffers
//! (zero masking key, no copy of the payload), from a source address of the caller's choice, so that what a counting
//! allocator sees is the server's memory and not the client's. It can send a message in frames of any size, to check
//! what the server keeps of fragmented messages.
#![allow(dead_code)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Mutex, mpsc};

#[derive(Debug)]
pub enum In {
    Text(String),
    Bin(usize),
    Close,
}

pub struct Raw {
    w: Arc<Mutex<OwnedWriteHalf>>,
    pub rx: mpsc::UnboundedReceiver<In>,
    pub closed: Arc<AtomicBool>,
    reader: tokio::task::JoinHandle<()>,
}

fn header(opcode: u8, fin: bool, len: usize) -> Vec<u8> {
    let mut h = vec![(if fin { 0x80 } else { 0 }) | opcode];
    if len < 126 {
        h.push(0x80 | len as u8);
    } else if len <= 0xffff {
        h.push(0x80 | 126);
        h.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        h.push(0x80 | 127);
        h.extend_from_slice(&(len as u64).to_be_bytes());
    }
    h.extend_from_slice(&[0, 0, 0, 0]);
    h
}

const UPGRADE: &str =
    "Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";

impl Raw {
    /// Opens an Engine.IO WebSocket session from `src` (any `127.x.y.z`). The reader answers pings with pongs, so a
    /// client that stops being answered is the server's doing.
    pub async fn open(addr: SocketAddr, src: Ipv4Addr) -> std::io::Result<Raw> {
        let sock = tokio::net::TcpSocket::new_v4()?;
        sock.bind(SocketAddr::new(src.into(), 0))?;
        let mut s = sock.connect(addr).await?;
        let req = format!("GET /socket.io/?EIO=4&transport=websocket HTTP/1.1\r\nHost: 127.0.0.1\r\n{UPGRADE}");
        s.write_all(req.as_bytes()).await?;
        let mut buf = Vec::new();
        let mut b = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            if s.read(&mut b).await? == 0 {
                return Err(std::io::Error::other("end of stream in the handshake"));
            }
            buf.push(b[0]);
        }
        if !buf.starts_with(b"HTTP/1.1 101") {
            return Err(std::io::Error::other(String::from_utf8_lossy(&buf).to_string()));
        }
        let (mut r, w) = s.into_split();
        let w = Arc::new(Mutex::new(w));
        let closed = Arc::new(AtomicBool::new(false));
        let open = read_frame(&mut r).await?;
        if !matches!(&open, Some((1, p)) if p.starts_with(b"0")) {
            return Err(std::io::Error::other(format!("no open packet: {open:?}")));
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let (w2, c2) = (w.clone(), closed.clone());
        let reader = tokio::spawn(async move {
            loop {
                match read_frame(&mut r).await {
                    Ok(Some((1, p))) => {
                        if p == b"2" {
                            let h = header(1, true, 1);
                            let mut g = w2.lock().await;
                            let _ = g.write_all(&h).await;
                            let _ = g.write_all(b"3").await;
                        } else {
                            let _ = tx.send(In::Text(String::from_utf8_lossy(&p).into_owned()));
                        }
                    }
                    Ok(Some((2, p))) => {
                        let _ = tx.send(In::Bin(p.len()));
                    }
                    Ok(Some((9, p))) => {
                        let h = header(10, true, p.len());
                        let mut g = w2.lock().await;
                        let _ = g.write_all(&h).await;
                        let _ = g.write_all(&p).await;
                    }
                    Ok(Some((8, _))) | Ok(None) | Err(_) => {
                        c2.store(true, Ordering::SeqCst);
                        let _ = tx.send(In::Close);
                        return;
                    }
                    Ok(Some(_)) => {}
                }
            }
        });
        Ok(Raw { w, rx, closed, reader })
    }

    /// The write side, to send from a task of its own.
    pub fn tx(&self) -> Tx {
        Tx { w: self.w.clone() }
    }

    pub async fn msg_str(&self, payload: &str) -> std::io::Result<()> {
        self.tx().msg(payload.as_bytes()).await
    }

    pub async fn recv(&mut self, wait: Duration) -> Option<In> {
        tokio::time::timeout(wait, self.rx.recv()).await.ok().flatten()
    }

    /// The next text that is not a ping, or `None` at a close or after `wait`.
    pub async fn recv_text(&mut self, wait: Duration) -> Option<String> {
        loop {
            match self.recv(wait).await? {
                In::Text(t) => return Some(t),
                In::Close => return None,
                In::Bin(_) => {}
            }
        }
    }

    /// Joins a namespace (`nsp` is `""` for the main one) and returns the answer.
    pub async fn connect(&mut self, nsp: &str) -> Option<String> {
        self.msg_str(&format!("0{nsp}")).await.ok()?;
        self.recv_text(Duration::from_secs(10)).await
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Waits until the server closed the connection, up to `wait`.
    pub async fn wait_closed(&mut self, wait: Duration) -> bool {
        let end = tokio::time::Instant::now() + wait;
        while tokio::time::Instant::now() < end {
            if self.is_closed() {
                return true;
            }
            let left = end - tokio::time::Instant::now();
            if let Some(In::Close) = self.recv(left).await {
                return true;
            }
        }
        self.is_closed()
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

async fn read_frame(r: &mut OwnedReadHalf) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let mut h = [0u8; 2];
    if r.read_exact(&mut h).await.is_err() {
        return Ok(None);
    }
    let op = h[0] & 0x0f;
    let mut len = (h[1] & 0x7f) as u64;
    if len == 126 {
        let mut b = [0u8; 2];
        r.read_exact(&mut b).await?;
        len = u16::from_be_bytes(b) as u64;
    } else if len == 127 {
        let mut b = [0u8; 8];
        r.read_exact(&mut b).await?;
        len = u64::from_be_bytes(b);
    }
    let mut p = vec![0u8; len as usize];
    r.read_exact(&mut p).await?;
    Ok(Some((op, p)))
}

pub fn ip(last: u8) -> Ipv4Addr {
    Ipv4Addr::new(127, 0, 0, last)
}

/// The write side of a client, cloneable into tasks.
#[derive(Clone)]
pub struct Tx {
    w: Arc<Mutex<OwnedWriteHalf>>,
}

impl Tx {
    async fn frame(&self, opcode: u8, fin: bool, parts: &[&[u8]]) -> std::io::Result<()> {
        let len = parts.iter().map(|p| p.len()).sum();
        let h = header(opcode, fin, len);
        let mut g = self.w.lock().await;
        g.write_all(&h).await?;
        for p in parts {
            g.write_all(p).await?;
        }
        Ok(())
    }

    /// An Engine.IO message (`4` + payload) as one text frame.
    pub async fn msg(&self, payload: &[u8]) -> std::io::Result<()> {
        self.frame(1, true, &[b"4", payload]).await
    }

    /// A binary message as one frame.
    pub async fn bin(&self, data: &[u8]) -> std::io::Result<()> {
        self.frame(2, true, &[data]).await
    }

    /// A binary message in frames of the given sizes (the last one takes what remains).
    pub async fn bin_fragmented(&self, data: &[u8], chunks: &[usize]) -> std::io::Result<()> {
        let mut at = 0;
        for (i, n) in chunks.iter().enumerate() {
            let last = i + 1 == chunks.len();
            let end = if last { data.len() } else { at + n };
            self.frame(if i == 0 { 2 } else { 0 }, last, &[&data[at..end]]).await?;
            at = end;
        }
        Ok(())
    }

    /// A text message (`4` + payload) in frames of `chunk` bytes.
    pub async fn msg_fragmented(&self, payload: &[u8], chunk: usize) -> std::io::Result<()> {
        let mut framed = Vec::with_capacity(payload.len() + 1);
        framed.push(b'4');
        framed.extend_from_slice(payload);
        let n = framed.chunks(chunk).count();
        for (i, c) in framed.chunks(chunk).enumerate() {
            self.frame(if i == 0 { 1 } else { 0 }, i + 1 == n, &[c]).await?;
        }
        Ok(())
    }
}
