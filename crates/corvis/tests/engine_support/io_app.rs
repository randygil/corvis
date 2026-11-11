//! The Corvis side of the Socket.IO scenarios: the same application as `parity/socket-io/reference/socket-server.mjs`,
//! written with `IoServer`. Change one and you change the other. Servers bind to 127.0.0.1.
#![allow(dead_code)]

use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use corvis::prelude::*;
use corvis::router::cors::CorsOptions;
use corvis::socket_io::{Args, BroadcastOperator, ConnectError, EngineOptions, IoNamespace, IoOptions, IoServer, IoSocket, IoValue};
use serde_json::{Value, json};

/// What happened on the server side, in order: served by `GET /harness/log`.
pub static LOG: LazyLock<Mutex<Vec<Value>>> = LazyLock::new(|| Mutex::new(Vec::new()));

pub fn record(entry: Value) {
    LOG.lock().unwrap().push(entry);
}

/// The rooms of a socket with its own id replaced by `<id>` (ids are random), sorted.
fn rooms_of(socket: &IoSocket) -> Vec<String> {
    let mut rooms: Vec<String> = socket.rooms().into_iter().map(|r| if r == socket.id() { "<id>".to_string() } else { r }).collect();
    rooms.sort();
    rooms
}

/// The one JSON argument of a command.
fn arg(args: &Args) -> Value {
    args.to_json().ok().and_then(|v| v.into_iter().next()).unwrap_or(Value::Null)
}

fn strings(arg: &Value, key: &str) -> Vec<String> {
    arg.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn selection(mut operator: BroadcastOperator, arg: &Value) -> BroadcastOperator {
    let (to, except) = (strings(arg, "to"), strings(arg, "except"));
    // `emptyTo`: a `to` given an empty list, as when the list came from a client.
    if arg.get("emptyTo").and_then(Value::as_bool) == Some(true) {
        operator = operator.to(Vec::<String>::new());
    }
    if !to.is_empty() {
        operator = operator.to(to);
    }
    if !except.is_empty() {
        operator = operator.except(except);
    }
    operator
}

/// The event `done` to every namespace of the server: what the scenarios wait on after a broadcast.
async fn mark_done(io: &IoServer) {
    for name in ["/", "/admin"] {
        let _ = io.of(name).unwrap().emit("done", ()).await;
    }
}

fn register(namespace: &IoNamespace, io: &IoServer) {
    let name = namespace.name().to_string();
    let io = io.clone();
    match name.as_str() {
        "/admin" => {
            namespace.use_middleware(|socket| async move {
                if socket.handshake().auth().get("token").and_then(Value::as_str) == Some("admin") {
                    Ok(())
                } else {
                    Err(ConnectError::new("not authorized").data(json!({ "content": "retry later" })))
                }
            });
        }
        "/denied" => {
            namespace.use_middleware(|_socket| async move { Err(ConnectError::new("not authorized")) });
        }
        "/slow" => {
            namespace.use_middleware(|_socket| async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(())
            });
        }
        _ => {}
    }
    namespace.on_connection(move |socket| {
        let nsp = name.clone();
        record(json!({ "event": "connection", "nsp": nsp, "auth": socket.handshake().auth() }));
        {
            // The rooms the socket was in while it disconnected, and after; and a farewell to the people in them.
            let nsp = nsp.clone();
            let during = Arc::new(Mutex::new(Vec::<String>::new()));
            let seen = during.clone();
            socket.on_disconnecting(move |socket, _reason| {
                *seen.lock().unwrap() = rooms_of(socket);
                let rooms: Vec<String> = socket.rooms().into_iter().collect();
                let socket = socket.clone();
                tokio::spawn(async move {
                    let _ = socket.to(rooms).emit("left", ()).await;
                });
            });
            socket.on_disconnect(move |socket, reason| {
                record(json!({
                    "event": "disconnect",
                    "nsp": nsp,
                    "reason": reason.as_str(),
                    "roomsDuring": *during.lock().unwrap(),
                    "roomsAfter": rooms_of(socket),
                }));
            });
        }
        socket.on("join", |socket, args, ack| async move {
            let rooms = strings(&arg(&args), "rooms");
            let answer = match socket.join(rooms) {
                Ok(()) => json!({ "ok": true, "rooms": rooms_of(&socket) }),
                Err(error) => json!({ "ok": false, "error": error.to_string() }),
            };
            if let Some(ack) = ack {
                let _ = ack.send((answer,));
            }
        });
        socket.on("room-leave", |socket, args, ack| async move {
            if let Some(room) = arg(&args).get("room").and_then(Value::as_str) {
                socket.leave(room);
            }
            if let Some(ack) = ack {
                let _ = ack.send((json!({ "rooms": rooms_of(&socket) }),));
            }
        });
        socket.on("rooms", |socket, _args, ack| async move {
            if let Some(ack) = ack {
                let _ = ack.send((json!({ "rooms": rooms_of(&socket) }),));
            }
        });
        {
            let io = io.clone();
            socket.on("broadcast", move |socket, args, ack| {
                let io = io.clone();
                async move {
                    let arg = arg(&args);
                    let event = arg.get("event").and_then(Value::as_str).unwrap_or("event").to_string();
                    let values: Vec<IoValue> = arg
                        .get("args")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(|v| IoValue::from_serialize(v).ok()).collect())
                        .unwrap_or_default();
                    let base = if arg.get("from").and_then(Value::as_str) == Some("socket") {
                        socket.broadcast()
                    } else {
                        socket.namespace().operator()
                    };
                    let _ = selection(base, &arg).emit(&event, values).await;
                    mark_done(&io).await;
                    if let Some(ack) = ack {
                        let _ = ack.send((json!({ "ok": true }),));
                    }
                }
            });
        }
        socket.on("broadcast-ack", |socket, args, ack| async move {
            // Node keeps the first value of each client's answer.
            let first = |responses: &[Args]| -> Vec<String> {
                let mut out: Vec<String> = responses
                    .iter()
                    .map(|r| r.to_json().ok().and_then(|v| v.into_iter().next()).unwrap_or(Value::Null).to_string())
                    .collect();
                out.sort();
                out
            };
            let arg = arg(&args);
            let mut operator = socket.namespace().operator();
            if let Some(ms) = arg.get("timeout").and_then(Value::as_u64) {
                operator = operator.timeout(Duration::from_millis(ms));
            }
            let answer = match selection(operator, &arg).emit_with_ack("question", ()).await {
                Ok(responses) => json!({ "ok": true, "responses": first(&responses) }),
                Err(error) => json!({ "ok": false, "message": error.to_string(), "responses": first(error.responses()) }),
            };
            if let Some(ack) = ack {
                let _ = ack.send((answer,));
            }
        });
        socket.on("sockets-join", |socket, args, ack| async move {
            let arg = arg(&args);
            let _ = selection(socket.namespace().operator(), &arg).sockets_join(strings(&arg, "rooms")).await;
            if let Some(ack) = ack {
                let _ = ack.send((json!({ "ok": true }),));
            }
        });
        socket.on("sockets-leave", |socket, args, ack| async move {
            let arg = arg(&args);
            let _ = selection(socket.namespace().operator(), &arg).sockets_leave(strings(&arg, "rooms")).await;
            if let Some(ack) = ack {
                let _ = ack.send((json!({ "ok": true }),));
            }
        });
        socket.on("disconnect-sockets", |socket, args, ack| async move {
            let arg = arg(&args);
            let close = arg.get("close").and_then(Value::as_bool) == Some(true);
            let _ = selection(socket.namespace().operator(), &arg).disconnect_sockets(close).await;
            if let Some(ack) = ack {
                let _ = ack.send((json!({ "ok": true }),));
            }
        });
        socket.on("fetch-sockets", |socket, args, ack| async move {
            let sockets = selection(socket.namespace().operator(), &arg(&args)).fetch_sockets().await.unwrap_or_default();
            let sockets: Vec<Value> =
                sockets.iter().map(|s| json!({ "id": s.id(), "rooms": s.rooms().iter().map(|r| &**r).collect::<Vec<&str>>() })).collect();
            if let Some(ack) = ack {
                let _ = ack.send((json!({ "sockets": sockets }),));
            }
        });
        socket.on("echo", |_socket, args, ack| async move {
            if let Some(ack) = ack {
                let _ = ack.send(args);
            }
        });
        socket.on("sum", |_socket, args, ack| async move {
            let total: i64 = args.iter().filter_map(|a| a.as_i64()).sum();
            if let Some(ack) = ack {
                let _ = ack.send((total, "ok"));
            }
        });
        socket.on("emit-update", |socket, _args, _ack| async move {
            let _ = socket.emit("update", (1, "two", json!({ "three": 3 })));
        });
        for (event, millis, result) in [("ask", 5_000u64, "ask-result"), ("ask-timeout", 200, "ask-timeout-result")] {
            let nsp = nsp.clone();
            socket.on(event, move |socket, _args, _ack| {
                let nsp = nsp.clone();
                async move {
                    let outcome = socket.timeout(Duration::from_millis(millis)).emit_with_ack("question", ()).await;
                    record(match outcome {
                        Ok(args) => json!({ "event": result, "nsp": nsp, "ok": true, "args": args.to_json().unwrap_or_default() }),
                        Err(error) => json!({ "event": result, "nsp": nsp, "ok": false, "message": error.to_string() }),
                    });
                }
            });
        }
        socket.on("bin-emit", |socket, args, _ack| async move {
            let _ = socket.emit("bin", args);
        });
        socket.on("leave", |socket, _args, _ack| async move { socket.disconnect(false) });
        socket.on("kick", |socket, _args, _ack| async move { socket.disconnect(true) });
        let nsp_for_reserved = nsp.clone();
        socket.on("emit-reserved", move |socket, _args, _ack| {
            let nsp = nsp_for_reserved.clone();
            async move {
                record(match socket.emit("disconnect", ()) {
                    Ok(()) => json!({ "event": "emit-reserved", "nsp": nsp, "ok": true }),
                    Err(error) => json!({ "event": "emit-reserved", "nsp": nsp, "ok": false, "message": error.to_string() }),
                });
            }
        });
    });
}

/// One Socket.IO server of the scenario app at `path`.
pub fn scenario_server(path: &str, connect_timeout: Duration) -> IoServer {
    let io = IoServer::new(IoOptions::new().engine(EngineOptions::new().path(path)).connect_timeout(connect_timeout)).unwrap();
    for name in ["/", "/admin", "/slow", "/denied"] {
        register(&io.of(name).unwrap(), &io);
    }
    io
}

/// Attaches the scenario servers (`/sio/` and `/sio-short/`) to the application.
pub fn attach_scenario_servers(app: &CorvisApplication) -> Vec<IoServer> {
    let mut servers = Vec::new();
    for (path, connect_timeout) in [("/sio/", Duration::from_millis(45_000)), ("/sio-short/", Duration::from_millis(1_000))] {
        let io = scenario_server(path, connect_timeout);
        io.attach(&app.get_http_server()).unwrap();
        servers.push(io);
    }
    servers
}

/// The server the upstream `socket.io-protocol` suite requires: ping 300 / 200 ms, connect timeout 1 s, CORS for any
/// origin, `auth` emitted on connect, `message` answered with `message-back`, `message-with-ack` acknowledged, and the
/// namespace `/custom`.
pub fn suite_server() -> IoServer {
    let io = IoServer::new(
        IoOptions::new()
            .engine(
                EngineOptions::new()
                    .ping_interval(Duration::from_millis(300))
                    .ping_timeout(Duration::from_millis(200))
                    .cors(CorsOptions::new().origin("*")),
            )
            .connect_timeout(Duration::from_millis(1_000)),
    )
    .unwrap();
    for name in ["/", "/custom"] {
        io.of(name).unwrap().on_connection(|socket| {
            let _ = socket.emit("auth", (socket.handshake().auth().clone(),));
            socket.on("message", |socket, args, _ack| async move {
                let _ = socket.emit("message-back", args);
            });
            socket.on("message-with-ack", |_socket, args, ack| async move {
                if let Some(ack) = ack {
                    let _ = ack.send(args);
                }
            });
        });
    }
    io
}
