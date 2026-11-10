//! The memory session store does nothing proportional to the size of a record while it holds its lock: sixteen tasks
//! that keep loading and touching one 15 KiB session must not slow down another user's requests much more than sixteen
//! tasks doing the same with a tiny session do. The comparison is a ratio against a run under the same load, with
//! generous slack, so it asserts on no absolute time and does not depend on the timer. Its own binary: nothing else
//! may compete for the CPU while it measures.
#![cfg(feature = "auth-session")]

mod session_common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use corvis::prelude::*;
use corvis::testing::request;
use session_common::*;

async fn cookie_of(app: &CorvisApplication, path: &str) -> String {
    let r = request(app).post(path).send().await;
    assert_eq!(r.status, 200, "{}", r.text());
    pair(&set_cookies(&r)[0])
}

/// The p99 of a victim's latency (a tiny session) while 16 tasks hammer `attacker_path`'s session, and the victim's
/// requests per second.
async fn victim_p99(attacker_path: &str) -> (Duration, f64) {
    let app = Arc::new(app(options()).await);
    let attacker = cookie_of(&app, attacker_path).await;
    let victim = cookie_of(&app, "/set?k=a&v=1").await;
    let stop = Arc::new(AtomicBool::new(false));
    let mut attackers = Vec::new();
    for _ in 0..16 {
        let (app, cookie, stop) = (app.clone(), attacker.clone(), stop.clone());
        attackers.push(tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                let r = request(&app).get("/who").header("cookie", &cookie).send().await;
                assert_eq!(r.status, 200);
                tokio::task::yield_now().await;
            }
        }));
    }
    let mut latencies = Vec::new();
    let begun = Instant::now();
    while begun.elapsed() < Duration::from_millis(1500) {
        let started = Instant::now();
        let r = request(&app).get("/who").header("cookie", &victim).send().await;
        assert_eq!(r.status, 200);
        latencies.push(started.elapsed());
    }
    stop.store(true, Ordering::Relaxed);
    for attacker in attackers {
        attacker.await.unwrap();
    }
    latencies.sort();
    let p99 = latencies[((latencies.len() as f64 * 0.99) as usize).min(latencies.len() - 1)];
    let rate = latencies.len() as f64 / begun.elapsed().as_secs_f64();
    (p99, rate)
}

// A measurement, not a check that can run everywhere: on a machine with few cores the attackers' own work on their
// 15 KiB records (parsing, outside the lock) takes the CPU from the victim, and the ratio says nothing about the lock
// (2.8 on a CI runner). Run it on a machine with eight cores or more: `cargo test -p corvis --features auth-session
// --test auth_session_contention -- --ignored`.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "timing comparison that needs eight idle cores; run it with --ignored"]
async fn big_sessions_do_not_slow_down_other_users() {
    // Timing on a loaded machine is noisy, so the verdict is the median of three rounds, each of which compares two runs
    // under the same load: how much worse the victim's p99 is and how much less it gets done (requests per second)
    // when the attackers use a 15 KiB session instead of a tiny one.
    let mut slower = Vec::new();
    let mut less = Vec::new();
    for round in 0..3 {
        let (tiny, tiny_rate) = victim_p99("/set?k=a&v=1").await;
        let (big, big_rate) = victim_p99("/fill?n=1900").await;
        let latency_ratio = big.as_secs_f64() / tiny.as_secs_f64().max(0.001);
        let rate_ratio = tiny_rate / big_rate.max(1.0);
        eprintln!(
            "round {round}: tiny attackers p99 {tiny:?} ({tiny_rate:.0}/s), 15 KiB attackers p99 {big:?} ({big_rate:.0}/s), p99 x{latency_ratio:.2}, throughput /{rate_ratio:.2}"
        );
        slower.push(latency_ratio);
        less.push(rate_ratio);
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let (slower, less) = (median(&mut slower), median(&mut less));
    assert!(slower <= 2.0, "a 15 KiB session made the other users' p99 {slower:.1} times worse than a tiny one");
    assert!(less <= 2.0, "a 15 KiB session cut the other users' throughput {less:.1} times more than a tiny one");
}
