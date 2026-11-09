//! The gate under TLS: it reads ciphertext only while the credit lasts when an upgraded connection's write is blocked.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::transport::tls_io::{Gate, Wire};
use super::{ConnState, READ_CREDIT_MAX};

async fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();
    (client, server)
}

/// Writes until the socket stops taking bytes for a moment, so the gate has seen a pending write.
async fn fill(gate: &mut Gate) {
    let chunk = vec![0u8; 64 * 1024];
    let mut total = 0usize;
    while tokio::time::timeout(Duration::from_millis(200), gate.write(&chunk)).await.is_ok() {
        total += chunk.len();
        assert!(total < 512 * 1024 * 1024, "the socket never filled");
    }
}

/// Reads from the gate until a read waits for 200 ms; the bytes read.
async fn read_until_parked(gate: &mut Gate, limit: usize) -> usize {
    let mut buf = vec![0u8; 8 * 1024];
    let mut total = 0usize;
    while total < limit {
        match tokio::time::timeout(Duration::from_millis(200), gate.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(n)) => total += n,
        }
    }
    total
}

fn upgraded() -> Arc<ConnState> {
    let state = Arc::new(ConnState::default());
    state.upgraded.store(true, Ordering::SeqCst);
    state
}

#[tokio::test]
async fn an_upgraded_gate_stops_reading_ciphertext_once_the_credit_is_spent_behind_a_blocked_write() {
    let (mut client, server) = pair().await;
    let wire = Wire::new();
    let mut gate = Gate::new(server, wire.clone(), upgraded());
    fill(&mut gate).await;
    assert!(wire.blocked(), "the pending write was recorded");

    let flood = vec![1u8; 1024 * 1024];
    client.write_all(&flood).await.unwrap();
    let read = read_until_parked(&mut gate, flood.len()).await;
    assert!(read <= READ_CREDIT_MAX, "read {read} bytes of ciphertext while the write was blocked");
    assert!(wire.credit_exhausted());
}

#[tokio::test]
async fn bytes_the_socket_accepts_earn_credit_back() {
    let (mut client, server) = pair().await;
    let wire = Wire::new();
    let mut gate = Gate::new(server, wire.clone(), upgraded());
    fill(&mut gate).await;
    client.write_all(&vec![1u8; 512 * 1024]).await.unwrap();
    let first = read_until_parked(&mut gate, 512 * 1024).await;
    assert!(first <= READ_CREDIT_MAX);

    // The client reads what the server wrote: the socket takes bytes again, which earns credit and lifts the block.
    let drain = tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        while let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(500), client.read(&mut buf)).await {
            if n == 0 {
                break;
            }
        }
    });
    let small = [0u8; 1024];
    loop {
        tokio::time::timeout(Duration::from_secs(5), gate.write_all(&small))
            .await
            .expect("the socket takes bytes once the client reads")
            .unwrap();
        if !wire.blocked() {
            break;
        }
    }
    let rest = read_until_parked(&mut gate, 512 * 1024).await;
    assert_eq!(first + rest, 512 * 1024, "everything the client sent is read once the write is unblocked");
    drain.abort();
}

#[tokio::test]
async fn a_connection_that_is_not_upgraded_reads_freely_behind_a_blocked_write() {
    let (mut client, server) = pair().await;
    let wire = Wire::new();
    let mut gate = Gate::new(server, wire.clone(), Arc::new(ConnState::default()));
    fill(&mut gate).await;
    client.write_all(&vec![1u8; 256 * 1024]).await.unwrap();
    let read = read_until_parked(&mut gate, 256 * 1024).await;
    assert_eq!(read, 256 * 1024);
}
