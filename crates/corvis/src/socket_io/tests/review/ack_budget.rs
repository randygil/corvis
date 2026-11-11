//! The answers a broadcast with acknowledgements holds: charged to the call, never to the answering connection,
//! bounded by the server-wide answer budget, and by one share per client address.

use super::super::rooms::connect;
use super::super::*;
use crate::socket_io::adapter::AckSink;

/// The answer of a recipient to the broadcast with acknowledgement id `id`.
fn answer(id: u64, text: &str) -> String {
    format!(r#"3{id}["{text}"]"#)
}

#[tokio::test(start_paused = true)]
async fn the_answers_a_broadcast_holds_are_charged_to_the_call_and_never_to_the_answering_connection() {
    let io = server(options());
    let mut conns = Vec::new();
    for _ in 0..3 {
        conns.push(connect(&io, "/").await.0);
    }
    let call = |io: &IoServer, seconds: u64| {
        let io = io.clone();
        tokio::spawn(async move { io.timeout(Duration::from_secs(seconds)).emit_with_ack("q", ()).await })
    };
    let big = "x".repeat(100_000);
    let engine = io.engine().clone();
    let held = || io.main().ack_budget().used();
    assert_eq!(engine.total_unread_bytes(), 0);
    assert_eq!(held(), 0);

    // 1. One answers, one is silent: the answer is held by the call, and the connection's budget, its address's and
    // the server's are free again the moment the call has it.
    let task = call(&io, 5);
    settle().await;
    conns[0].sent().await;
    conns[0].text(&answer(0, &big));
    settle().await;
    assert!(held() >= 100_000, "held by the call: {}", held());
    assert_eq!(engine.total_unread_bytes(), 0, "not on the server-wide unread gauge");
    assert_eq!(engine.address_unread_bytes("127.0.0.1"), 0, "not on the address");
    assert_eq!(conns[0].socket.unread_bytes(), 0, "not on the session");
    advance(5_000).await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::Timeout);
    assert_eq!(error.responses().len(), 1);
    drop(error);
    assert_eq!(held(), 0, "released when the call ended");

    // 2. The answering connection closes while its answer is held: nothing of it remains on the connection side,
    // and the call keeps the answer until it ends.
    let task = call(&io, 120);
    settle().await;
    conns[0].text(&answer(1, &big));
    settle().await;
    assert!(held() >= 100_000);
    conns[0].socket.close();
    advance(31_000).await; // the close timeout of the session: no transport drains it here
    assert_eq!(io.main().socket_count(), 2, "the answering socket is gone");
    assert!(held() >= 100_000, "the call still holds the answer");
    assert_eq!(engine.total_unread_bytes(), 0);
    conns[1].text(&answer(1, "b"));
    conns[2].text(&answer(1, "c"));
    settle().await;
    let answers = task.await.unwrap().unwrap();
    assert_eq!(answers.len(), 3);
    drop(answers);
    settle().await;
    assert_eq!(held(), 0);
    assert_eq!(engine.total_unread_bytes(), 0);

    // 3. A call dropped before its end releases what it holds.
    let (_d, _) = connect(&io, "/").await;
    let task = call(&io, 5);
    settle().await;
    conns[1].text(&answer(2, &big));
    settle().await;
    assert!(held() >= 100_000);
    task.abort();
    let _ = task.await;
    settle().await;
    assert_eq!(held(), 0);
    assert_eq!(engine.total_unread_bytes(), 0);
}

#[test]
fn the_answers_of_all_calls_together_are_bounded_by_the_server_answer_budget_without_waiting() {
    use crate::socket_io::adapter::{AckBudget, AckShared};
    let budget = AckBudget::new(1_000);
    let one = AckShared::new(10_000, budget.clone());
    let two = AckShared::new(10_000, budget.clone());
    let (a, b) = (AckSink::new(one.clone()), AckSink::new(two.clone()));
    let answer = |n: usize| Args::new(vec![IoValue::String("x".repeat(n))]);
    a.respond(answer(300));
    assert!(budget.used() >= 300 && budget.used() <= 1_000);
    // The second call finds no room: nothing waits, it keeps what it had (nothing).
    b.respond(answer(900));
    assert_eq!(two.finish().len(), 0);
    // A call that finished frees its room for the others.
    assert_eq!(one.finish().len(), 1);
    assert_eq!(budget.used(), 0);
    let three = AckShared::new(10_000, budget.clone());
    AckSink::new(three.clone()).respond(answer(600));
    assert_eq!(three.finish().len(), 1);
    assert_eq!(budget.used(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_call_that_finds_no_room_in_the_answer_budget_ends_with_responses_too_large() {
    let io = server(options());
    let (a, _) = connect(&io, "/").await;
    let (_b, _) = connect(&io, "/").await;
    // Another call holds nearly the whole budget.
    let budget = io.main().ack_budget().clone();
    let cap = options().ack_budget();
    let full = crate::socket_io::adapter::AckShared::new(cap, budget.clone());
    AckSink::new(full.clone()).respond(Args::new(vec![IoValue::String("x".repeat(cap - 1_000))]));
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await }
    });
    settle().await;
    a.text(&answer(0, &"y".repeat(10_000)));
    settle().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::ResponsesTooLarge);
    assert_eq!(io.engine().total_unread_bytes(), 0, "the answerer's unread budget is not what ran out");
    drop(error);
    full.finish();
    assert_eq!(budget.used(), 0);
}

#[tokio::test(start_paused = true)]
async fn answers_dropped_by_the_cap_release_their_charge_too() {
    let io = server(options().max_broadcast_ack_bytes(150_000));
    let mut conns = Vec::new();
    for _ in 0..4 {
        conns.push(connect(&io, "/").await.0);
    }
    let task = tokio::spawn({
        let io = io.clone();
        async move { io.timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await }
    });
    settle().await;
    let big = "x".repeat(100_000);
    conns[0].text(&answer(0, &big));
    settle().await;
    conns[1].text(&answer(0, &big));
    settle().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(*error.kind(), BroadcastAckErrorKind::ResponsesTooLarge);
    drop(error);
    settle().await;
    assert_eq!(io.engine().total_unread_bytes(), 0);
    assert_eq!(io.engine().address_unread_bytes("127.0.0.1"), 0);
}

/// The ack id of the first `q` event in what a connection was sent.
fn question_id(sent: &[Out]) -> u64 {
    for out in sent {
        if let Out::T(text) = out
            && let Some(rest) = text.strip_prefix('2')
        {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if rest[digits.len()..].starts_with("[\"q\"") {
                return digits.parse().unwrap();
            }
        }
    }
    panic!("no question in {sent:?}")
}

/// Behind a proxy every request trusts, two connections of one peer send forwarded addresses that do not parse: the
/// session falls back to the peer for its key, and so does the answer budget, so the two share one address share.
#[tokio::test(start_paused = true)]
async fn malformed_forwarded_addresses_from_one_peer_share_one_share_of_the_answer_budget() {
    const MIB: usize = 1 << 20;
    let io = server(
        IoOptions::new()
            .max_decoded_bytes(MIB)
            .max_broadcast_ack_bytes(MIB)
            .engine(EngineOptions::new().ping_interval(DAY).ping_timeout(DAY).max_total_unread_bytes(16 * MIB)),
    );
    let budget = io.main().ack_budget().clone();
    let conns: Vec<Conn> = ["junk-a", "junk-b", "junk-c"]
        .iter()
        .map(|xff| Conn { socket: io.engine().open_test_socket_forwarded("203.0.113.1", Some(xff)) })
        .collect();
    for conn in &conns {
        let id = conn.join("0").await;
        let socket = io.main().socket(&id).unwrap();
        assert_eq!(socket.address_key(), "203.0.113.1", "the key of the session, not the raw forwarded value");
        socket.join("r").unwrap();
    }
    let calls = 40;
    let mut tasks = Vec::new();
    for _ in 0..calls {
        let io = io.clone();
        tasks.push(tokio::spawn(async move { io.to("r").timeout(Duration::from_secs(60)).emit_with_ack("q", ()).await }));
        settle().await;
    }
    let big = "x".repeat(100_000);
    // Two of them answer; the third stays silent, so the calls stay open and hold what was answered.
    for conn in &conns[..2] {
        conn.sent().await;
        for id in 0..calls as u64 {
            conn.text(&answer(id, &big));
            settle().await;
        }
    }
    assert_eq!(budget.address_entries(), 1, "one share for both connections");
    assert!(budget.used() <= budget.address_share(), "{} held, share {}", budget.used(), budget.address_share());
    advance(61_000).await;
    let too_large = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter(|r| matches!(r, Ok(Err(e)) if *e.kind() == BroadcastAckErrorKind::ResponsesTooLarge))
        .count();
    assert!(too_large > 0, "the answers past the share ended their calls with ResponsesTooLarge");
}

#[tokio::test(start_paused = true)]
async fn one_address_cannot_fill_the_answer_budget_and_make_the_calls_of_others_fail() {
    const MIB: usize = 1 << 20;
    let io = server(
        IoOptions::new()
            .max_decoded_bytes(MIB)
            .max_broadcast_ack_bytes(MIB)
            .engine(EngineOptions::new().ping_interval(DAY).ping_timeout(DAY).max_total_unread_bytes(16 * MIB)),
    );
    let budget = io.main().ack_budget().clone();
    // The attacker (one address) has a socket that answers and a silent one in the same room; honest members are on
    // two other addresses.
    let attacker = io.conn_from("203.0.113.1");
    let ia = attacker.join("0").await;
    let silent = io.conn_from("203.0.113.1");
    let isilent = silent.join("0").await;
    let mut honest: Vec<(Conn, String)> = Vec::new();
    for address in ["203.0.113.2", "203.0.113.3"] {
        let c = io.conn_from(address);
        let id = c.join("0").await;
        honest.push((c, id));
    }
    for id in [&ia, &isilent] {
        io.main().socket(id).unwrap().join("r1").unwrap();
    }
    for (_, id) in &honest {
        io.main().socket(id).unwrap().join("r2").unwrap();
    }
    let calls = 40;
    let mut tasks = Vec::new();
    for _ in 0..calls {
        let io = io.clone();
        tasks.push(tokio::spawn(async move { io.to("r1").timeout(Duration::from_secs(60)).emit_with_ack("q", ()).await }));
        settle().await;
    }
    attacker.sent().await;
    let big = "x".repeat(100_000);
    for id in 0..calls as u64 {
        attacker.text(&answer(id, &big));
        settle().await;
    }
    // 40 answers of 100 kB would be 4 MB, the whole budget (4 MiB here): the address share stops it far before.
    assert!(budget.used() <= budget.address_share(), "{} held, share {}", budget.used(), budget.address_share());
    assert_eq!(budget.address_entries(), 1);
    // An honest call, answered from other addresses, completes.
    let honest_call = {
        let io = io.clone();
        tokio::spawn(async move { io.to("r2").timeout(Duration::from_secs(5)).emit_with_ack("q", ()).await })
    };
    settle().await;
    for (conn, _) in &honest {
        let id = question_id(&conn.sent().await);
        conn.text(&answer(id, "ok"));
    }
    settle().await;
    let answers = honest_call.await.unwrap().expect("the honest call completes");
    assert_eq!(answers.len(), 2);
    drop(answers);
    // The calls of the attacker end (some with too much, the others at the deadline) and the map empties.
    advance(61_000).await;
    let mut too_large = 0;
    for task in tasks {
        if let Err(e) = task.await.unwrap()
            && *e.kind() == BroadcastAckErrorKind::ResponsesTooLarge
        {
            too_large += 1;
        }
    }
    assert!(too_large > 0, "the answers over the share ended their calls with ResponsesTooLarge");
    assert_eq!(budget.used(), 0);
    assert_eq!(budget.address_entries(), 0, "an address with nothing held is forgotten");
}
