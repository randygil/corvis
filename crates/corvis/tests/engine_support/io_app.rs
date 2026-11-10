//! The Corvis side of the Socket.IO scenarios: the same application as `parity/socket-io/reference/socket-server.mjs`,
//! written with `IoServer`. Change one and you change the other. Servers bind to 127.0.0.1.
#![allow(dead_code)]

use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use corvis::prelude::*;
use corvis::router::cors::CorsOptions;
use corvis::socket_io::{ConnectError, EngineOptions, IoNamespace, IoOptions, IoServer};
use serde_json::{Value, json};

/// What happened on the server side, in order: served by `GET /harness/log`.
pub static LOG: LazyLock<Mutex<Vec<Value>>> = LazyLock::new(|| Mutex::new(Vec::new()));

pub fn record(entry: Value) {
    LOG.lock().unwrap().push(entry);
}

fn register(namespace: &IoNamespace) {
    let name = namespace.name().to_string();
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
            let nsp = nsp.clone();
            socket.on_disconnect(move |_socket, reason| record(json!({ "event": "disconnect", "nsp": nsp, "reason": reason.as_str() })));
        }
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
        register(&io.of(name).unwrap());
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
