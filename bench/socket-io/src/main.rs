//! The Socket.IO benchmark of Corvis (see `README.md`).
//!
//! - `server --port <p>`: an `IoServer` with default options on `127.0.0.1:<p>`, a counting global allocator and the
//!   route `GET /stats` (`GET /stats/reset` resets the counters and the peak first).
//! - `load --port <p> --scenario <s> --transport <t> --connections <n> --messages <m> --size <bytes>`: raw Engine.IO /
//!   Socket.IO clients that run one scenario and print one JSON line with the figures of the measured window.
//! - `report <dir>`: the Markdown table of the JSON lines found in `<dir>` (median, minimum and maximum per measurement).
//!
//! Scenarios: `emit` (each client sends `m` events with an acknowledgement, the handler answers), `server-emit` (the
//! server emits `m` events to each socket from a task), `broadcast` (the server emits `m` events to the whole namespace).
//! Every client sends a 4 KiB `cookie` header with its handshake.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use corvis::prelude::*;
use corvis::socket_io::{IoOptions, IoServer};
use futures::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpSocket, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

// ------------------------------------------------------------------------------------------- counting allocator

/// Counts the allocations and the bytes requested from the system allocator (reallocations count as one allocation of
/// their new size).
struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_RSS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn resident() -> u64 {
    memory_stats::memory_stats().map_or(0, |m| m.physical_mem as u64)
}

// ---------------------------------------------------------------------------------------------------- server

#[controller("stats")]
pub struct StatsController;

#[routes]
impl StatsController {
    #[get]
    fn read(&self) -> Value {
        json!({
            "allocations": ALLOCATIONS.load(Ordering::Relaxed),
            "bytes": BYTES.load(Ordering::Relaxed),
            "peak_rss": PEAK_RSS.load(Ordering::Relaxed).max(resident()),
        })
    }

    #[get("reset")]
    fn reset(&self) -> Value {
        PEAK_RSS.store(resident(), Ordering::Relaxed);
        ALLOCATIONS.store(0, Ordering::Relaxed);
        BYTES.store(0, Ordering::Relaxed);
        json!({ "reset": true })
    }
}

#[module(controllers = [StatsController])]
pub struct StatsModule;

/// Microseconds since the Unix epoch: the time stamp a delivery's latency is measured from (server and load generator
/// run on the same machine).
fn now_micros() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

/// The two numbers of a `server-emit` or `broadcast` command: the events to emit and the payload size.
fn command(args: &corvis::socket_io::Args) -> (usize, usize) {
    let number = |i: usize| args.get::<u64>(i).map_or(0, |n| n as usize);
    (number(0), number(1))
}

fn configure(io: &IoServer) {
    let main = io.main();
    let namespace = main.clone();
    main.on_connection(move |socket| {
        socket.on("emit", |_socket, args, ack| async move {
            if let Some(ack) = ack {
                let _ = ack.send(args);
            }
        });
        socket.on("server-emit", |socket, args, _ack| async move {
            let (count, size) = command(&args);
            let payload = "x".repeat(size);
            tokio::spawn(async move {
                for _ in 0..count {
                    let _ = socket.emit("m", (now_micros(), payload.as_str()));
                    tokio::task::yield_now().await;
                }
            });
        });
        let namespace = namespace.clone();
        socket.on("broadcast", move |_socket, args, _ack| {
            let namespace = namespace.clone();
            async move {
                let (count, size) = command(&args);
                let payload = "x".repeat(size);
                for _ in 0..count {
                    let _ = namespace.emit("m", (now_micros(), payload.as_str())).await;
                }
            }
        });
    });
}

async fn server(port: u16) -> corvis::Result<()> {
    corvis::logger::set_log_levels(&[corvis::logger::LogLevel::Error, corvis::logger::LogLevel::Fatal]);
    let app = CorvisFactory::create::<StatsModule>().await?;
    let io = IoServer::new(IoOptions::new())?;
    configure(&io);
    io.attach(&app.get_http_server())?;
    tokio::spawn(async {
        loop {
            PEAK_RSS.fetch_max(resident(), Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    app.listen(format!("127.0.0.1:{port}")).await
}

// ------------------------------------------------------------------------------------------------ load generator

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Emit,
    ServerEmit,
    Broadcast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    WebSocket,
    Polling,
}

#[derive(Debug, Clone)]
struct Load {
    port: u16,
    scenario: Scenario,
    transport: Transport,
    connections: usize,
    messages: usize,
    size: usize,
}

type Failure = Box<dyn std::error::Error + Send + Sync>;

/// The cookie every client sends (4 KiB), so what the server keeps or copies of the handshake request shows.
fn cookie() -> String {
    format!("bench={}", "c".repeat(4096 - "bench=".len()))
}

/// Clients are spread over 200 loopback addresses so the per-address caps of the default options never apply.
fn local_address(index: usize) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 1, (index % 200) as u8 + 1)), 0)
}

/// Connects from the loopback address of client `index`. A run opens thousands of connections from a few addresses and
/// the previous runs leave their ports in `TIME_WAIT`, so a port the system hands out may still be taken for this
/// destination: such an attempt is retried with another port.
async fn connect_tcp(port: u16, index: usize) -> Result<TcpStream, Failure> {
    let server = SocketAddr::from(([127, 0, 0, 1], port));
    let mut attempts = 0;
    loop {
        let socket = TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        socket.bind(local_address(index))?;
        socket.set_nodelay(true)?;
        match socket.connect(server).await {
            Ok(stream) => return Ok(stream),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && attempts < 50 => attempts += 1,
            Err(e) => return Err(e.into()),
        }
    }
}

/// One HTTP/1.1 connection to the server.
struct Http {
    sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
}

impl Http {
    async fn open(port: u16, index: usize) -> Result<Http, Failure> {
        let stream = connect_tcp(port, index).await?;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
        tokio::spawn(connection);
        Ok(Http { sender })
    }

    async fn request(&mut self, method: &str, path: &str, body: String) -> Result<String, Failure> {
        let request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1")
            .header("cookie", cookie())
            .header("content-type", "text/plain;charset=UTF-8")
            .body(Full::new(Bytes::from(body)))?;
        self.sender.ready().await?;
        let response = self.sender.send_request(request).await?;
        let status = response.status();
        let body = response.into_body().collect().await?.to_bytes();
        if !status.is_success() {
            return Err(format!("{method} {path}: {status}").into());
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

/// A connected client: Socket.IO packets in (without the Engine.IO `4`) and Engine.IO packets out.
struct Client {
    out: mpsc::UnboundedSender<String>,
    packets: mpsc::UnboundedReceiver<String>,
}

impl Client {
    fn send(&self, packet: String) -> Result<(), Failure> {
        self.out.send(format!("4{packet}")).map_err(|_| "the connection ended".into())
    }

    async fn next(&mut self) -> Result<String, Failure> {
        self.packets.recv().await.ok_or_else(|| "the connection ended".into())
    }

    /// Joins the main namespace and waits for its CONNECT.
    async fn join(&mut self) -> Result<(), Failure> {
        self.send("0".to_string())?;
        loop {
            if self.next().await?.starts_with('0') {
                return Ok(());
            }
        }
    }
}

/// Hands an Engine.IO packet received from the server to the client: Socket.IO packets go to `packets`, pings are
/// answered. `false` when the client is gone.
fn deliver(packet: &str, out: &mpsc::UnboundedSender<String>, packets: &mpsc::UnboundedSender<String>) -> bool {
    match packet.as_bytes().first() {
        Some(b'2') => out.send("3".to_string()).is_ok(),
        Some(b'4') => packets.send(packet[1..].to_string()).is_ok(),
        _ => true,
    }
}

async fn open_websocket(port: u16, index: usize) -> Result<Client, Failure> {
    let stream = connect_tcp(port, index).await?;
    let mut request = format!("ws://127.0.0.1:{port}/socket.io/?EIO=4&transport=websocket").into_client_request()?;
    request.headers_mut().insert("cookie", cookie().parse()?);
    let (ws, _) = tokio_tungstenite::client_async(request, stream).await?;
    let (mut sink, mut stream) = ws.split();
    let (out, mut outgoing) = mpsc::unbounded_channel::<String>();
    let (packets_tx, packets) = mpsc::unbounded_channel();
    let pong = out.clone();
    tokio::spawn(async move {
        while let Some(text) = outgoing.recv().await {
            if sink.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(Ok(message)) = stream.next().await {
            if let Message::Text(text) = message
                && !deliver(&text, &pong, &packets_tx)
            {
                break;
            }
        }
    });
    Ok(Client { out, packets })
}

async fn open_polling(port: u16, index: usize) -> Result<Client, Failure> {
    let mut get = Http::open(port, index).await?;
    let open = get.request("GET", "/socket.io/?EIO=4&transport=polling", String::new()).await?;
    let open: Value = serde_json::from_str(open.strip_prefix('0').ok_or("no open packet")?)?;
    let sid = open["sid"].as_str().ok_or("no sid")?.to_string();
    let path = format!("/socket.io/?EIO=4&transport=polling&sid={sid}");
    let mut post = Http::open(port, index).await?;
    let (out, mut outgoing) = mpsc::unbounded_channel::<String>();
    let (packets_tx, packets) = mpsc::unbounded_channel();
    let pong = out.clone();
    let post_path = path.clone();
    tokio::spawn(async move {
        while let Some(first) = outgoing.recv().await {
            // What queued meanwhile goes in the same `POST`.
            let mut body = first;
            while let Ok(next) = outgoing.try_recv() {
                body.push('\x1e');
                body.push_str(&next);
            }
            if post.request("POST", &post_path, body).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Ok(payload) = get.request("GET", &path, String::new()).await {
            if payload.split('\x1e').any(|packet| packet == "1" || !deliver(packet, &pong, &packets_tx)) {
                break;
            }
        }
    });
    Ok(Client { out, packets })
}

async fn open_client(load: &Load, index: usize) -> Result<Client, Failure> {
    let mut client = match load.transport {
        Transport::WebSocket => open_websocket(load.port, index).await?,
        Transport::Polling => open_polling(load.port, index).await?,
    };
    client.join().await?;
    Ok(client)
}

async fn stats(port: u16, path: &str) -> Result<Value, Failure> {
    let mut http = Http::open(port, 0).await?;
    Ok(serde_json::from_str(&http.request("GET", path, String::new()).await?)?)
}

/// The time stamp of an `m` event (`2["m",<micros>,"..."]`).
fn stamp_of(packet: &str) -> Option<u64> {
    let rest = packet.strip_prefix("2[\"m\",")?;
    rest[..rest.find(',')?].parse().ok()
}

/// What one client measured: the latency of every message, in microseconds.
async fn run_client(load: Load, index: usize, mut client: Client, start: tokio::sync::watch::Receiver<bool>) -> Result<Vec<u64>, Failure> {
    let mut start = start;
    while !*start.borrow_and_update() {
        start.changed().await?;
    }
    let mut latencies = Vec::with_capacity(load.messages);
    let payload = "x".repeat(load.size);
    match load.scenario {
        Scenario::Emit => {
            for id in 0..load.messages {
                let sent = Instant::now();
                client.send(format!("2{id}[\"emit\",\"{payload}\"]"))?;
                let answer = format!("3{id}[");
                while !client.next().await?.starts_with(&answer) {}
                latencies.push(sent.elapsed().as_micros() as u64);
            }
        }
        Scenario::ServerEmit | Scenario::Broadcast => {
            if load.scenario == Scenario::ServerEmit {
                client.send(format!("2[\"server-emit\",{},{}]", load.messages, load.size))?;
            } else if index == 0 {
                client.send(format!("2[\"broadcast\",{},{}]", load.messages, load.size))?;
            }
            while latencies.len() < load.messages {
                let packet = client.next().await?;
                if let Some(stamp) = stamp_of(&packet) {
                    latencies.push(now_micros().saturating_sub(stamp));
                }
            }
        }
    }
    Ok(latencies)
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

async fn load(load: Load) -> Result<Value, Failure> {
    // Connect in waves, so the handshakes do not overflow the listen backlog.
    let mut clients = Vec::with_capacity(load.connections);
    for wave in (0..load.connections).collect::<Vec<_>>().chunks(64) {
        let opened = futures::future::join_all(wave.iter().map(|&i| open_client(&load, i))).await;
        for client in opened {
            clients.push(client?);
        }
    }
    let (go, start) = tokio::sync::watch::channel(false);
    let runs: Vec<_> =
        clients.into_iter().enumerate().map(|(i, client)| tokio::spawn(run_client(load.clone(), i, client, start.clone()))).collect();
    stats(load.port, "/stats/reset").await?;
    let began = Instant::now();
    go.send(true)?;
    let mut latencies = Vec::with_capacity(load.connections * load.messages);
    let finished = tokio::time::timeout(Duration::from_secs(600), futures::future::join_all(runs)).await.map_err(|_| "the run took longer than 600 s")?;
    let seconds = began.elapsed().as_secs_f64();
    let after = stats(load.port, "/stats").await?;
    for run in finished {
        latencies.extend(run??);
    }
    latencies.sort_unstable();
    let messages = (load.connections * load.messages) as f64;
    let per_message = |key: &str| after[key].as_u64().unwrap_or(0) as f64 / messages;
    Ok(json!({
        "scenario": match load.scenario { Scenario::Emit => "emit", Scenario::ServerEmit => "server-emit", Scenario::Broadcast => "broadcast" },
        "transport": match load.transport { Transport::WebSocket => "websocket", Transport::Polling => "polling" },
        "connections": load.connections,
        "messages": load.messages,
        "size": load.size,
        "seconds": seconds,
        "rate": messages / seconds,
        "p50_us": percentile(&latencies, 0.50),
        "p99_us": percentile(&latencies, 0.99),
        "max_us": latencies.last().copied().unwrap_or(0),
        "allocations_per_message": per_message("allocations"),
        "bytes_per_message": per_message("bytes"),
        "peak_rss": after["peak_rss"],
    }))
}

// -------------------------------------------------------------------------------------------------------- report

/// The Markdown table of the JSON lines in `dir`: one row per measurement (scenario, transport, connections, messages,
/// size), each figure the median of its runs with the minimum and maximum.
fn report(dir: &str) -> Result<String, Failure> {
    let mut groups: BTreeMap<(String, String, u64, u64, u64), Vec<Value>> = BTreeMap::new();
    let mut files: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).map(|e| e.path()).collect();
    files.sort();
    for file in files.iter().filter(|p| p.extension().is_some_and(|e| e == "json")) {
        for line in std::fs::read_to_string(file)?.lines().filter(|l| l.starts_with('{')) {
            let run: Value = serde_json::from_str(line)?;
            let key = (
                run["scenario"].as_str().unwrap_or_default().to_string(),
                run["transport"].as_str().unwrap_or_default().to_string(),
                run["connections"].as_u64().unwrap_or(0),
                run["messages"].as_u64().unwrap_or(0),
                run["size"].as_u64().unwrap_or(0),
            );
            groups.entry(key).or_default().push(run);
        }
    }
    let mut out = String::from(
        "| Scenario | Transport | Connections | Messages | Payload | Runs | Messages/s | p50 (ms) | p99 (ms) | Max (ms) | Allocations/msg | Bytes/msg | Peak RSS (MiB) |\n\
         |---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    for ((scenario, transport, connections, messages, size), runs) in &groups {
        let cell = |key: &str, scale: f64, decimals: usize| {
            let mut values: Vec<f64> = runs.iter().map(|r| r[key].as_f64().unwrap_or(0.0) / scale).collect();
            values.sort_by(f64::total_cmp);
            let median = values[values.len() / 2];
            format!("{median:.decimals$} ({:.decimals$} to {:.decimals$})", values[0], values[values.len() - 1])
        };
        out.push_str(&format!(
            "| {scenario} | {transport} | {connections} | {messages} | {size} B | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            runs.len(),
            cell("rate", 1.0, 0),
            cell("p50_us", 1000.0, 2),
            cell("p99_us", 1000.0, 2),
            cell("max_us", 1000.0, 2),
            cell("allocations_per_message", 1.0, 1),
            cell("bytes_per_message", 1.0, 0),
            cell("peak_rss", 1024.0 * 1024.0, 1),
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------- main

const USAGE: &str = "usage:
  corvis-sio-bench server --port <port>
  corvis-sio-bench load --port <port> --scenario emit|server-emit|broadcast --transport websocket|polling \\
                        --connections <n> --messages <m> --size <bytes>
  corvis-sio-bench report <raw directory>";

fn flag<T: std::str::FromStr>(args: &[String], name: &str) -> Result<T, Failure> {
    let at = args.iter().position(|a| a == name).ok_or_else(|| format!("missing {name}\n{USAGE}"))?;
    let value = args.get(at + 1).ok_or_else(|| format!("{name} needs a value\n{USAGE}"))?;
    value.parse().map_err(|_| format!("{name}: invalid value {value:?}\n{USAGE}").into())
}

fn parse_load(args: &[String]) -> Result<Load, Failure> {
    let scenario = match flag::<String>(args, "--scenario")?.as_str() {
        "emit" => Scenario::Emit,
        "server-emit" => Scenario::ServerEmit,
        "broadcast" => Scenario::Broadcast,
        other => return Err(format!("unknown scenario {other:?}\n{USAGE}").into()),
    };
    let transport = match flag::<String>(args, "--transport")?.as_str() {
        "websocket" => Transport::WebSocket,
        "polling" => Transport::Polling,
        other => return Err(format!("unknown transport {other:?}\n{USAGE}").into()),
    };
    Ok(Load {
        port: flag(args, "--port")?,
        scenario,
        transport,
        connections: flag(args, "--connections")?,
        messages: flag(args, "--messages")?,
        size: flag(args, "--size")?,
    })
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result: Result<(), Failure> = match args.first().map(String::as_str) {
        Some("server") => match flag(&args, "--port") {
            Ok(port) => server(port).await.map_err(|e| e.to_string().into()),
            Err(e) => Err(e),
        },
        Some("load") => match parse_load(&args) {
            Ok(spec) => load(spec).await.map(|figures| println!("{figures}")),
            Err(e) => Err(e),
        },
        Some("report") => match args.get(1) {
            Some(dir) => report(dir).map(|table| print!("{table}")),
            None => Err(USAGE.into()),
        },
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("corvis-sio-bench: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
