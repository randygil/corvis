//! The WebSocket transport: a WebSocket-first session, or a candidate that upgrades a polling session.
//!
//! The loop follows the structure of the gateway loop (`websockets::run_connection`): a writer task drains the
//! session's bounded outbox, reads pause while the application holds too much unread data, protocol errors map to
//! the same close codes, and a closing connection waits at most `close_timeout` for the peer. The server's write
//! deadline and read credit apply because the connection was upgraded through hyper like any other WebSocket.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role as WsRole, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use super::EngineServer;
use super::packet::{Incoming, Packet, WsFrame, decode_frame};
use super::session::{Candidate, CloseReason, Ready, Session};
use crate::http_adapter::UpgradedIo;
use crate::logger::Logger;
use crate::ws::{WriteEnd, protocol_error, write_loop_with};

/// What the WebSocket is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    /// A WebSocket-first session: the socket is the transport from the start.
    First,
    /// An upgrade of a polling session.
    Candidate,
    /// A WebSocket that cannot be used (a transport that does not handle upgrades): closed right away.
    Refuse,
}

type Sink = SplitSink<WebSocketStream<UpgradedIo>, Message>;
type Stream = SplitStream<WebSocketStream<UpgradedIo>>;
type Writer = JoinHandle<(WriteEnd, Sink)>;

fn encode(packet: Packet) -> Message {
    match packet.encode_ws() {
        WsFrame::Text(t) => Message::Text(t),
        WsFrame::Binary(b) => Message::Binary(b.to_vec()),
    }
}

fn spawn_writer(sink: Sink, session: &Session) -> Writer {
    tokio::spawn(write_loop_with(sink, session.outbox.clone(), encode))
}

enum Exit {
    /// The session decided (it was closed, or became closing).
    Session,
    /// The peer closed the connection or sent a close frame.
    Peer { close_frame: bool },
    /// A protocol error: close with this code.
    Protocol(u16),
    /// The writer failed: the socket is gone.
    WriterFailed,
    /// A candidate that did not upgrade.
    CandidateEnded,
}

pub(super) async fn run(server: EngineServer, session: Arc<Session>, io: UpgradedIo, role: Role, close_timeout: Duration) {
    let max = session.options.max_http_buffer_size;
    let config = WebSocketConfig { max_message_size: Some(max), max_frame_size: Some(max), ..WebSocketConfig::default() };
    let ws = WebSocketStream::from_raw_socket(io, WsRole::Server, Some(config)).await;
    let (sink, mut read) = ws.split();

    let mut candidate: Option<Candidate> = None;
    let mut writer: Option<Writer> = None;
    let mut sink_slot: Option<Sink> = None;
    let mut active = false;
    match role {
        Role::First => {
            active = true;
            writer = Some(spawn_writer(sink, &session));
        }
        Role::Candidate => match session.begin_candidate() {
            Some(c) => {
                candidate = Some(c);
                sink_slot = Some(sink);
            }
            // An upgrade is in progress or done, or the session is gone: this WebSocket is closed (Node).
            None => return finish_socket(sink, read, None, false, close_timeout).await,
        },
        Role::Refuse => return finish_socket(sink, read, None, false, close_timeout).await,
    }
    if role == Role::First {
        server.notify_connection(&session);
    }

    let mut closed = session.closed_watch();
    let mut reason = CloseReason::TransportClose;
    let exit = loop {
        if session.is_closed() {
            break Exit::Session;
        }
        // Registered before the limit is read, so a release in between wakes this loop (`notify_waiters` stores
        // nothing for a waiter that is not registered yet).
        let unread_changed = session.unread.changed();
        let paused = active && session.unread.over_limit();
        let cancelled = async {
            match candidate.as_mut() {
                Some(c) => {
                    let _ = c.cancel.changed().await;
                }
                None => futures::future::pending::<()>().await,
            }
        };
        let writer_done = async {
            match writer.as_mut() {
                Some(w) => w.await,
                None => futures::future::pending().await,
            }
        };
        tokio::select! {
            incoming = read.next(), if !paused => match incoming {
                None => break Exit::Peer { close_frame: false },
                Some(Ok(Message::Close(_))) => break Exit::Peer { close_frame: true },
                Some(Ok(Message::Text(t))) => {
                    if let Some(exit) = on_frame(&session, &mut candidate, &mut sink_slot, &mut writer, &mut active, decode_frame(WsFrame::Text(t))).await {
                        break exit;
                    }
                }
                Some(Ok(Message::Binary(b))) => {
                    if let Some(exit) = on_frame(&session, &mut candidate, &mut sink_slot, &mut writer, &mut active, decode_frame(WsFrame::Binary(Bytes::from(b)))).await {
                        break exit;
                    }
                }
                // tungstenite answers pings by itself.
                Some(Ok(_)) => {}
                Some(Err(WsError::ConnectionClosed | WsError::AlreadyClosed)) => break Exit::Peer { close_frame: false },
                // The socket failed (a reset, or the server cut it after `send_timeout`): `ws` reports a socket error
                // as `transport error`, and the connection is gone, so there is no closing handshake.
                Some(Err(WsError::Io(_))) => break Exit::WriterFailed,
                Some(Err(e)) => {
                    let (_, code) = protocol_error(&e);
                    break Exit::Protocol(code);
                }
            },
            _ = unread_changed, if paused => {}
            _ = closed.changed() => break Exit::Session,
            _ = cancelled => break Exit::CandidateEnded,
            end = writer_done => {
                writer = None;
                match end {
                    Ok((WriteEnd::Failed(kind), _)) => {
                        if kind == std::io::ErrorKind::TimedOut {
                            Logger::new("EngineIO").warn(format!(
                                "session {}: the client accepted no data for the server's send_timeout, connection dropped",
                                session.log_id
                            ));
                        }
                        break Exit::WriterFailed;
                    }
                    Ok((_, sink)) => {
                        // The outbox ended: the session is closing (or closed).
                        sink_slot = Some(sink);
                        if session.ready() == Ready::Closing {
                            session.close(CloseReason::ForcedClose);
                        }
                        break Exit::Session;
                    }
                    Err(_) => break Exit::WriterFailed,
                }
            }
        }
    };

    let mut close_code: Option<u16> = None;
    let mut peer_close_frame = false;
    match exit {
        Exit::Peer { close_frame } => peer_close_frame = close_frame,
        Exit::Protocol(code) => {
            reason = CloseReason::TransportError;
            close_code = Some(code);
        }
        Exit::WriterFailed => reason = CloseReason::TransportError,
        Exit::Session | Exit::CandidateEnded => {}
    }
    if active {
        // The session ends with the transport (a no-op when it already ended).
        session.close(reason);
    } else if let Some(c) = &candidate {
        session.candidate_failed(c.id);
    }

    if matches!(exit, Exit::WriterFailed) {
        // The socket does not accept data: dropping both halves closes the connection.
        if let Some(w) = writer.take() {
            w.abort();
        }
        return;
    }
    // The writer ends by itself once the outbox is sealed; a write blocked on a peer that does not read is cut.
    if let Some(w) = writer.take() {
        match tokio::time::timeout(Duration::from_secs(1), w).await {
            Ok(Ok((_, sink))) => sink_slot = Some(sink),
            _ => return,
        }
    }
    let Some(sink) = sink_slot else { return };
    let frame = close_code.map(|code| CloseFrame { code: CloseCode::from(code), reason: Cow::Borrowed("") });
    finish_socket(sink, read, frame, peer_close_frame, close_timeout).await;
}

/// Handles one received frame. `Some` ends the loop.
async fn on_frame(
    session: &Arc<Session>,
    candidate: &mut Option<Candidate>,
    sink_slot: &mut Option<Sink>,
    writer: &mut Option<Writer>,
    active: &mut bool,
    incoming: Incoming,
) -> Option<Exit> {
    if *active {
        match incoming {
            Incoming::Packet(packet) => session.on_packet(packet),
            Incoming::Invalid => session.on_invalid(),
        }
        return None;
    }
    let id = candidate.as_ref()?.id;
    match incoming {
        Incoming::Packet(Packet::Ping(data)) if data == "probe" => {
            let sink = sink_slot.as_mut()?;
            if sink.send(encode(Packet::Pong("probe".to_string()))).await.is_err() {
                return Some(Exit::WriterFailed);
            }
            session.candidate_probed(id);
            None
        }
        Incoming::Packet(Packet::Upgrade) => {
            if session.candidate_upgraded(id) {
                *active = true;
                *candidate = None;
                *writer = Some(spawn_writer(sink_slot.take()?, session));
                None
            } else {
                Some(Exit::CandidateEnded)
            }
        }
        // Anything else closes the candidate, not the session.
        _ => Some(Exit::CandidateEnded),
    }
}

/// Closes the WebSocket: the close frame (a reply when the peer sent one first), then waits at most
/// `close_timeout` for the peer's.
async fn finish_socket(mut sink: Sink, read: Stream, frame: Option<CloseFrame<'static>>, peer_closed: bool, close_timeout: Duration) {
    if peer_closed {
        // tungstenite queued the reply to the peer's close frame; flushing writes it.
        let _ = tokio::time::timeout(close_timeout, sink.flush()).await;
        return;
    }
    if !matches!(tokio::time::timeout(close_timeout, sink.send(Message::Close(frame))).await, Ok(Ok(()))) {
        return;
    }
    // Wait for the peer to close its side. The raw stream is read and discarded (a protocol error leaves the
    // WebSocket reader unusable): dropping a socket that still has unread data makes the operating system reset the
    // connection, and the peer could lose the close frame we just wrote.
    let Ok(mut ws) = sink.reunite(read) else { return };
    let io = ws.get_mut();
    let mut scratch = [0u8; 8192];
    let _ = tokio::time::timeout(close_timeout, async {
        use tokio::io::AsyncReadExt;
        while matches!(io.read(&mut scratch).await, Ok(n) if n > 0) {}
    })
    .await;
}
