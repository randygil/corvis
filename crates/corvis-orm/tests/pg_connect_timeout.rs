//! `connect_timeout` in the database URL against a real Postgres (skipped without a server, see
//! `common/pg.rs`). The cases that need no server (a silent listener, a refused port) are unit tests of
//! the crate. Scratch objects: the roles `orm_ct_role_*_<run>` and the database `orm_test_ct_latin1_<run>` (per-run names,
//! see `common/pg.rs`), dropped before and after each test that uses them and again when the process exits.
//!
//! Timing rule: a holder is in place (handshake) before the query under test starts; a duration has a tight lower
//! bound and an upper bound below the nearest wrong value, named in the assertion (see `common/gate.rs`).

#[path = "common/gate.rs"]
mod gate;
#[path = "common/pg.rs"]
mod pg;

use std::time::{Duration, Instant};

use corvis_orm::prelude::*;

/// A client on the server's `postgres` database, to create and drop scratch objects.
async fn admin() -> OrmClient {
    OrmClient::new(format!("{}/postgres", pg::base_url())).unwrap()
}

fn role_url(role: &str, db: &str, q: &str) -> String {
    let url = format!("postgres://{role}:ctpw@{}/{db}", pg::host(&pg::base_url()));
    if q.is_empty() { url } else { format!("{url}?{q}") }
}

async fn reset_role(admin: &OrmClient, role: &str) {
    admin.db_execute(&format!("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = '{role}'")).await.unwrap();
    admin.db_execute(&format!("DROP ROLE IF EXISTS {role}")).await.unwrap();
}

/// Holds the only connection of `db` with a transaction until the returned sender fires; the future resolves
/// once the holder has the connection (a handshake, not a delay).
async fn hold_connection(db: &OrmClient) -> (tokio::sync::oneshot::Sender<()>, tokio::task::JoinHandle<std::result::Result<(), OrmError>>) {
    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let d = db.clone();
    let holder = tokio::spawn(async move {
        d._transaction_with(TransactionOptions::new().timeout(120_000), |tx| async move {
            tx.query_raw("SELECT 1", vec![]).await?;
            let _ = held_tx.send(());
            let _ = release_rx.await;
            Ok::<_, OrmError>(())
        })
        .await
    });
    held_rx.await.unwrap();
    (release_tx, holder)
}

#[tokio::test]
async fn waiting_for_a_slot_is_not_cut_by_the_connect_timeout() {
    let Some(url) = pg::fresh_db("ct_slot").await else { return };
    let db = OrmClient::new(format!("{url}?connection_limit=1&pool_timeout=0&connect_timeout=1")).unwrap();
    db.connect().await.unwrap();
    let (release, holder) = hold_connection(&db).await;
    let query = {
        let d = db.clone();
        tokio::spawn(async move { d.query_raw("SELECT 1 AS one", vec![]).await })
    };
    // More than twice `connect_timeout=1`: a wait cut by the connect timeout would have failed by now.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(!query.is_finished(), "the query ended while the only connection was held (connect_timeout=1 cut the wait?)");
    release.send(()).unwrap();
    let rows = query.await.unwrap().unwrap();
    assert_eq!(rows.len(), 1);
    holder.await.unwrap().unwrap();
}

#[tokio::test]
async fn pool_timeout_still_bounds_the_wait_for_a_slot() {
    let Some(url) = pg::fresh_db("ct_slot2").await else { return };
    let db = OrmClient::new(format!("{url}?connection_limit=1&pool_timeout=1&connect_timeout=30")).unwrap();
    db.connect().await.unwrap();
    let (release, holder) = hold_connection(&db).await;
    // The holder is released only after the query has failed, so the query can only end by timing out.
    let t = Instant::now();
    let e = db.query_raw("SELECT 1", vec![]).await.unwrap_err();
    let waited = t.elapsed();
    release.send(()).unwrap();
    assert_eq!(e.code(), Some("P2024"), "{e}");
    assert!(waited >= Duration::from_millis(950), "{waited:?}");
    assert!(waited < Duration::from_secs(3), "{waited:?}: connect_timeout=30 or the 10 s default pool timeout would show 10 s or more");
    holder.await.unwrap().unwrap();
    // The slot came back: the pool works again.
    db.query_raw("SELECT 1", vec![]).await.unwrap();
}

#[tokio::test]
async fn role_at_its_connection_limit_fails_after_the_connect_timeout() {
    let Some(url) = pg::fresh_db("ct_role").await else { return };
    let db_name = pg::db_of(&url).to_string();
    let role = pg::scratch_role("ct_role_a");
    let admin = admin().await;
    reset_role(&admin, &role).await;
    admin.db_execute(&format!("CREATE ROLE {role} LOGIN PASSWORD 'ctpw' CONNECTION LIMIT 0")).await.unwrap();
    let db = OrmClient::new(role_url(&role, &db_name, "connect_timeout=1&pool_timeout=0")).unwrap();
    let t = Instant::now();
    let e = db.connect().await.unwrap_err();
    assert!(t.elapsed() >= Duration::from_millis(950), "{:?}", t.elapsed());
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}: the 5 s default connect_timeout would show 5 s or more", t.elapsed());
    assert_eq!(e.code(), Some("P1001"), "{e}");
    assert!(e.message().starts_with("Can't reach database server at `"), "{e}");
    assert!(!e.message().contains("ctpw"), "{e}");
    reset_role(&admin, &role).await;
}

#[tokio::test]
async fn server_lost_after_start_fails_the_next_query_after_the_connect_timeout() {
    let Some(url) = pg::fresh_db("ct_lost").await else { return };
    let db_name = pg::db_of(&url).to_string();
    let role = pg::scratch_role("ct_role_b");
    let admin = admin().await;
    reset_role(&admin, &role).await;
    admin.db_execute(&format!("CREATE ROLE {role} LOGIN PASSWORD 'ctpw'")).await.unwrap();
    let db = OrmClient::new(role_url(&role, &db_name, "connection_limit=2&pool_timeout=0&connect_timeout=1")).unwrap();
    db.connect().await.unwrap();
    db.query_raw("SELECT 1", vec![]).await.unwrap();
    // New connections of the role now get 53300, and the open ones are gone.
    admin.db_execute(&format!("ALTER ROLE {role} CONNECTION LIMIT 0")).await.unwrap();
    admin.db_execute(&format!("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = '{role}'")).await.unwrap();
    let t = Instant::now();
    // The wrong behavior is waiting forever (`pool_timeout=0`); the guard turns a hang into a failure.
    let e = tokio::time::timeout(Duration::from_secs(10), db.query_raw("SELECT 1", vec![]))
        .await
        .expect("the query still waited after 10 s: connect_timeout=1 did not bound it")
        .unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}: 3 s above connect_timeout=1", t.elapsed());
    assert_eq!(e.code(), Some("P1001"), "{e}");
    // Lifting the limit lets the same client recover: the slot of the failed request was released.
    admin.db_execute(&format!("ALTER ROLE {role} CONNECTION LIMIT -1")).await.unwrap();
    db.query_raw("SELECT 1", vec![]).await.unwrap();
    db.disconnect().await.unwrap();
    reset_role(&admin, &role).await;
}

#[tokio::test]
async fn a_schema_the_database_cannot_encode_is_reported_at_once() {
    let admin = admin().await;
    // Skip without a server.
    let latin1 = pg::scratch_db("ct_latin1");
    if pg::fresh_db("ct_probe").await.is_none() {
        return;
    }
    admin.db_execute(&format!("DROP DATABASE IF EXISTS {latin1} WITH (FORCE)")).await.unwrap();
    admin.db_execute(&format!("CREATE DATABASE {latin1} ENCODING 'LATIN1' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0")).await.unwrap();
    // `connect_timeout=30`: retrying until the connect timeout would take 30 s, far above the 10 s bound, so the
    // bound tells "reported at once" from "retried" however slow the machine is.
    let url = format!("{}/{latin1}?schema=%E2%82%AC&pool_timeout=0&connect_timeout=30", pg::base_url());

    let t = Instant::now();
    let e = OrmClient::new(&url).unwrap().connect().await.unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}: retried until connect_timeout=30?", t.elapsed());
    assert_eq!(e.code(), Some("P1001"), "{e}");
    assert!(e.message().starts_with("Can't reach database server at `"), "{e}");
    assert!(e.message().contains("has no equivalent in encoding"), "{e}");

    let t = Instant::now();
    let e = OrmClient::new(&url).unwrap().query_raw("SELECT 1", vec![]).await.unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}: retried until connect_timeout=30?", t.elapsed());
    assert_eq!(e.code(), Some("P1001"), "{e}");
    assert!(e.message().contains("has no equivalent in encoding"), "{e}");

    admin.db_execute(&format!("DROP DATABASE IF EXISTS {latin1} WITH (FORCE)")).await.unwrap();
}

#[tokio::test]
async fn cancelled_queries_and_scripts_do_not_leak_slots() {
    let Some(url) = pg::fresh_db("ct_cancel").await else { return };
    let db = OrmClient::new(format!("{url}?connection_limit=2&pool_timeout=3&connect_timeout=3")).unwrap();
    db.connect().await.unwrap();
    for i in 0..300u64 {
        let q = db.query_raw("SELECT pg_sleep(0.01)", vec![]);
        let _ = tokio::time::timeout(Duration::from_micros(200 + (i * 53) % 3000), q).await;
        if i % 10 == 0 {
            let s = db.db_execute("SELECT pg_sleep(0.01)");
            let _ = tokio::time::timeout(Duration::from_micros(300 + (i * 31) % 2000), s).await;
        }
    }
    // Every slot comes back: two gated queries must both be blocked at the server at once. A leaked slot leaves one
    // blocked query while the other waits for a slot and fails with P2024 after `pool_timeout=3`; `wait_for` stops
    // as soon as one of the queries has finished before both are blocked.
    let gate = gate::Gate::close(&url, 7003).await;
    let sql = gate.sql();
    let mut tasks: Vec<gate::Task> = Vec::new();
    for _ in 0..2 {
        let (db, sql) = (db.clone(), sql.clone());
        tasks.push(tokio::spawn(async move { db.query_raw(&sql, vec![]).await }));
    }
    assert_eq!(gate.wait_for(2, &mut tasks).await, (2, 2), "a slot leaked");
    gate.open().await;
    for t in tasks {
        t.await.unwrap().unwrap();
    }
}

// ---------------------------------------------------------------------------------------------
// Replay of the fixture recorded with the original client (`tests/fixtures/connect_timeout.json`,
// script `corvis-tests/tmp/orm/connect/cases.js`).
// ---------------------------------------------------------------------------------------------

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/connect_timeout.json")).unwrap()
}

fn with_q(url: &str, q: &str) -> String {
    match (q.is_empty() || q == "default", url.contains('?')) {
        (true, _) => url.to_string(),
        (false, true) => format!("{url}&{q}"),
        (false, false) => format!("{url}?{q}"),
    }
}

/// Loopback listener that accepts sockets and never writes.
async fn silent_listener() -> (u16, tokio::task::JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let h = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = l.accept().await {
            held.push(s);
        }
    });
    (port, h)
}

/// Seconds the original took, rounded: `None` when it was still pending.
fn secs(case: &Value) -> Option<f64> {
    if case["pending"] == true { None } else { Some((case["elapsed_ms"].as_f64().unwrap() / 1000.0).round()) }
}

async fn run(url: String) -> (std::result::Result<Vec<Value>, OrmError>, Duration) {
    let t = Instant::now();
    let r = OrmClient::new(url).unwrap().query_raw("SELECT 1", vec![]).await;
    (r, t.elapsed())
}

#[tokio::test]
async fn fixture_silent_server_timings_match() {
    let fx = fixture();
    let (port, _srv) = silent_listener().await;
    let base = format!("postgresql://u:p@127.0.0.1:{port}/x");
    let mut runs = Vec::new();
    for (q, case) in fx["silent"].as_object().unwrap() {
        let url = with_q(&base, q);
        match secs(case) {
            // `connect_timeout=0&pool_timeout=0`: still waiting.
            None => {
                let c = OrmClient::new(url).unwrap();
                assert!(tokio::time::timeout(Duration::from_secs(3), c.connect()).await.is_err(), "{q}");
            }
            Some(s) => {
                assert_eq!(case["ok"], false, "{q}");
                runs.push((q.clone(), s, tokio::spawn(run(url))));
            }
        }
    }
    for (q, s, h) in runs {
        let (r, took) = h.await.unwrap();
        let e = r.unwrap_err();
        assert_eq!(e.code(), Some("P1001"), "{q}: {e}");
        assert!(e.message().starts_with("Can't reach database server at `127.0.0.1:"), "{q}: {e}");
        let got = took.as_secs_f64();
        // Tight lower bound; the upper bound is below the 5 s default (the value for `connect_timeout=1&connect_timeout=3`).
        assert!(got >= s - 0.05, "{q}: original {s} s, corvis-orm {got} s");
        assert!(got < s + 1.5, "{q}: original {s} s, corvis-orm {got} s; the 5 s default would show 5 s or more");
    }
}

#[tokio::test]
async fn fixture_refused_port() {
    let fx = fixture();
    let base = "postgresql://u:p@127.0.0.1:1/x";
    for (q, case) in fx["refused"].as_object().unwrap() {
        // The original reports an initialization error (its class name ends in `ClientInitializationError`).
        assert!(case["error"]["name"].as_str().unwrap().ends_with("ClientInitializationError"), "{q}");
        assert!(case["error"]["message"].as_str().unwrap().contains("Can't reach database server at `127.0.0.1:1`"), "{q}");
    }
    // Difference (design decision 6): the original fails after about 2 s whatever the parameters say; corvis-orm retries
    // the refused connection inside `connect_timeout` and then fails with the same `P1001`.
    for (q, limit) in [("connect_timeout=1", 1.0), ("default", 5.0), ("pool_timeout=0&connect_timeout=2", 2.0)] {
        let (r, took) = run(with_q(base, q)).await;
        let e = r.unwrap_err();
        assert_eq!(e.code(), Some("P1001"), "{q}");
        assert!(e.message().starts_with("Can't reach database server at `127.0.0.1:1`"), "{q}: {e}");
        // The retry loop may give up one backoff before the deadline; load only adds time on the upper side.
        assert!(took.as_secs_f64() >= limit - 1.0, "{q}: {took:?}");
        assert!(took.as_secs_f64() < limit + 1.5, "{q}: {took:?}; the original's fixed 2 s or an ignored limit would differ");
    }
    // `connect_timeout=0`: the original still fails at once; corvis-orm keeps retrying (no limit was asked for).
    assert_eq!(fx["refused"]["connect_timeout=0"]["ok"], false);
    let c = OrmClient::new(with_q(base, "connect_timeout=0")).unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(3), c.connect()).await.is_err());
}

#[tokio::test]
async fn fixture_busy_pool_is_not_cut_by_connect_timeout() {
    let fx = fixture();
    let case = &fx["busy_pool"]["connection_limit=1&pool_timeout=0&connect_timeout=1 (hold 3s)"];
    assert_eq!(case["ok"], true);
    assert!((case["waited_ms"].as_f64().unwrap() - 2700.0).abs() < 600.0);
    // corvis-orm: `waiting_for_a_slot_is_not_cut_by_the_connect_timeout` runs the same scenario.
}

#[tokio::test]
async fn fixture_invalid_values_and_sqlite() {
    let fx = fixture();
    let Some(url) = pg::fresh_db("ct_fixture").await else { return };
    for (key, case) in fx["invalid_postgres"].as_object().unwrap() {
        // `pool_timeout` overflow is `pool_timeout`'s own rule (no limit in corvis-orm, an error in the original).
        if !key.starts_with("connect_timeout=") {
            continue;
        }
        let r = OrmClient::new(format!("{url}?{key}"));
        match case["ok"].as_bool().unwrap() {
            false => {
                let e = r.unwrap_err();
                assert_eq!(e.code(), Some("P1013"), "{key}");
                assert!(case["error"]["message"].as_str().unwrap().contains("The provided database string is invalid."), "{key}");
                assert!(e.message().starts_with("The provided database string is invalid."), "{key}: {e}");
                assert!(e.message().contains("`connect_timeout`"), "{key}: {e}");
            }
            true => {
                let c = r.unwrap_or_else(|e| panic!("{key}: {e}"));
                c.query_raw("SELECT 1", vec![]).await.unwrap_or_else(|e| panic!("{key}: {e}"));
            }
        }
    }
    let c = OrmClient::new(format!("{url}?connect_timeout=3")).unwrap();
    assert_eq!(fx["valid_postgres"]["connect_timeout=3"]["ok"], true);
    c.query_raw("SELECT 1", vec![]).await.unwrap();

    let dir = std::env::temp_dir().join(format!("corvis-ct-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.to_string_lossy().replace('\\', "/");
    for (key, case) in fx["sqlite"].as_object().unwrap() {
        assert_eq!(case["ok"], true, "{key}");
        let c = OrmClient::new(key.replace("<db>", &format!("{f}/p.db"))).unwrap();
        c.query_raw("SELECT 1", vec![]).await.unwrap_or_else(|e| panic!("{key}: {e}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn fixture_database_side_failures() {
    let fx = fixture();
    // Differences (reported by the fixture): the original connects to the LATIN1 database with `schema=%E2%82%AC`
    // (it does not run `SET search_path` per connection) and reports a role at its limit at once ("Too many database
    // connections opened"). corvis-orm fails the first with `P1001` and the database's message (see
    // `a_schema_the_database_cannot_encode_is_reported_at_once`) and the second with `P1001` after `connect_timeout`.
    assert_eq!(fx["setup_rejected"]["$connect"]["ok"], true);
    assert_eq!(fx["setup_rejected"]["$queryRaw"]["ok"], true);
    let case = &fx["role_at_limit"]["default"];
    assert_eq!(case["ok"], false);
    assert!(case["error"]["message"].as_str().unwrap().contains("too many connections for role"));
    assert!(case["elapsed_ms"].as_f64().unwrap() < 1000.0);

    let Some(url) = pg::fresh_db("ct_fixture_role").await else { return };
    let role = pg::scratch_role("ct_role_c");
    let admin = admin().await;
    reset_role(&admin, &role).await;
    admin.db_execute(&format!("CREATE ROLE {role} LOGIN PASSWORD 'ctpw' CONNECTION LIMIT 0")).await.unwrap();
    let (r, took) = run(role_url(&role, pg::db_of(&url), "connect_timeout=1&pool_timeout=0")).await;
    reset_role(&admin, &role).await;
    let e = r.unwrap_err();
    assert_eq!(e.code(), Some("P1001"), "{e}");
    assert!(took >= Duration::from_millis(950), "{took:?}");
    assert!(took < Duration::from_secs(4), "{took:?}: the 5 s default connect_timeout would show 5 s or more");
}

#[tokio::test]
async fn a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back() {
    let Some(url) = pg::fresh_db("ct_tx_cancel").await else { return };
    let observer = OrmClient::new(format!("{url}?connection_limit=1")).unwrap();
    for q in ["connection_limit=1&pool_timeout=0&connect_timeout=1", "connection_limit=1&pool_timeout=0&connect_timeout=0"] {
        let db = OrmClient::new(format!("{url}?{q}")).unwrap();
        db.connect().await.unwrap();
        // Dropped once the observer sees the long query running: sqlx rolls back and pings only after that query
        // ends (about 3 s).
        let running = async {
            loop {
                let rows = observer
                    .query_raw(
                        "SELECT count(*)::int8 AS n FROM pg_stat_activity WHERE datname = current_database() AND state = 'active' \
                         AND query LIKE '%pg_sleep(3)%' AND pid <> pg_backend_pid()",
                        vec![],
                    )
                    .await
                    .unwrap();
                if rows[0]["n"].as_i64().unwrap() > 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        let tx = db._transaction(|tx| async move { tx.query_raw("SELECT pg_sleep(3)", vec![]).await });
        tokio::select! {
            r = tx => panic!("{q}: the transaction ended before it was cancelled: {r:?}"),
            r = tokio::time::timeout(Duration::from_secs(10), running) => r.expect("the long query never showed as active within 10 s"),
        }
        // The next request waits for the slot (no limit) instead of failing with P1001 after the connect timeout.
        let t = Instant::now();
        let rows = db.query_raw("SELECT 1 AS one", vec![]).await.unwrap_or_else(|e| panic!("{q}: {e}"));
        assert_eq!(rows.len(), 1);
        assert!(t.elapsed() >= Duration::from_millis(1500), "{q}: {:?}", t.elapsed());
        // The transaction was rolled back and the connection is clean.
        let n = db
            .query_raw(
                "SELECT count(*) AS n FROM pg_stat_activity WHERE state = 'idle in transaction' AND datname = current_database()",
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(n[0]["n"], 0, "{q}");
        db.disconnect().await.unwrap();
    }
}

#[tokio::test]
async fn the_parameter_is_removed_from_the_driver_url() {
    let Some(url) = pg::fresh_db("ct_param").await else { return };
    let db = OrmClient::new(format!("{url}?sslmode=disable&connect_timeout=3&schema=s")).unwrap();
    assert_eq!(db.connect().await.ok(), Some(()));
    let rows = db.query_raw("SELECT current_setting('search_path') AS p", vec![]).await.unwrap();
    assert_eq!(rows[0]["p"], "s");
}
