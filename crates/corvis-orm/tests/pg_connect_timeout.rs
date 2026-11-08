//! `connect_timeout` in the database URL against a real Postgres (skipped without a server, see
//! `common/pg.rs`). The cases that need no server (a silent listener, a refused port) are unit tests of
//! the crate. Scratch objects: the roles `orm_ct_role_a` and `orm_ct_role_b` and the database `orm_test_ct_latin1`, dropped
//! before and after each test that uses them.

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

#[tokio::test]
async fn waiting_for_a_slot_is_not_cut_by_the_connect_timeout() {
    let Some(url) = pg::fresh_db("ct_slot").await else { return };
    let db = OrmClient::new(format!("{url}?connection_limit=1&pool_timeout=0&connect_timeout=1")).unwrap();
    db.connect().await.unwrap();
    let holder = {
        let d = db.clone();
        tokio::spawn(async move {
            d._transaction(|tx| async move {
                tx.query_raw("SELECT 1", vec![]).await?;
                tokio::time::sleep(Duration::from_secs(3)).await;
                Ok::<_, OrmError>(())
            })
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t = Instant::now();
    let rows = db.query_raw("SELECT 1 AS one", vec![]).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(t.elapsed() >= Duration::from_millis(2000), "{:?}", t.elapsed());
    holder.await.unwrap().unwrap();
}

#[tokio::test]
async fn pool_timeout_still_bounds_the_wait_for_a_slot() {
    let Some(url) = pg::fresh_db("ct_slot2").await else { return };
    let db = OrmClient::new(format!("{url}?connection_limit=1&pool_timeout=1&connect_timeout=30")).unwrap();
    db.connect().await.unwrap();
    let holder = {
        let d = db.clone();
        tokio::spawn(async move {
            d._transaction(|tx| async move {
                tx.query_raw("SELECT 1", vec![]).await?;
                tokio::time::sleep(Duration::from_secs(3)).await;
                Ok::<_, OrmError>(())
            })
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t = Instant::now();
    let e = db.query_raw("SELECT 1", vec![]).await.unwrap_err();
    assert_eq!(e.code(), Some("P2024"), "{e}");
    assert!(t.elapsed() >= Duration::from_millis(900) && t.elapsed() < Duration::from_millis(2500), "{:?}", t.elapsed());
    holder.await.unwrap().unwrap();
    // The slot came back: the pool works again.
    db.query_raw("SELECT 1", vec![]).await.unwrap();
}

#[tokio::test]
async fn role_at_its_connection_limit_fails_after_the_connect_timeout() {
    let Some(url) = pg::fresh_db("ct_role").await else { return };
    let db_name = "orm_test_ct_role";
    let admin = admin().await;
    reset_role(&admin, "orm_ct_role_a").await;
    admin.db_execute("CREATE ROLE orm_ct_role_a LOGIN PASSWORD 'ctpw' CONNECTION LIMIT 0").await.unwrap();
    let _ = url;
    let db = OrmClient::new(role_url("orm_ct_role_a", db_name, "connect_timeout=1&pool_timeout=0")).unwrap();
    let t = Instant::now();
    let e = db.connect().await.unwrap_err();
    assert!(t.elapsed() >= Duration::from_millis(900) && t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert_eq!(e.code(), Some("P1001"), "{e}");
    assert!(e.message().starts_with("Can't reach database server at `"), "{e}");
    assert!(!e.message().contains("ctpw"), "{e}");
    reset_role(&admin, "orm_ct_role_a").await;
}

#[tokio::test]
async fn server_lost_after_start_fails_the_next_query_after_the_connect_timeout() {
    let Some(_) = pg::fresh_db("ct_lost").await else { return };
    let db_name = "orm_test_ct_lost";
    let admin = admin().await;
    reset_role(&admin, "orm_ct_role_b").await;
    admin.db_execute("CREATE ROLE orm_ct_role_b LOGIN PASSWORD 'ctpw'").await.unwrap();
    let db = OrmClient::new(role_url("orm_ct_role_b", db_name, "connection_limit=2&pool_timeout=0&connect_timeout=1")).unwrap();
    db.connect().await.unwrap();
    db.query_raw("SELECT 1", vec![]).await.unwrap();
    // New connections of the role now get 53300, and the open ones are gone.
    admin.db_execute("ALTER ROLE orm_ct_role_b CONNECTION LIMIT 0").await.unwrap();
    admin.db_execute("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = 'orm_ct_role_b'").await.unwrap();
    let t = Instant::now();
    let e = db.query_raw("SELECT 1", vec![]).await.unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
    assert_eq!(e.code(), Some("P1001"), "{e}");
    // Lifting the limit lets the same client recover: the slot of the failed request was released.
    admin.db_execute("ALTER ROLE orm_ct_role_b CONNECTION LIMIT -1").await.unwrap();
    db.query_raw("SELECT 1", vec![]).await.unwrap();
    db.disconnect().await.unwrap();
    reset_role(&admin, "orm_ct_role_b").await;
}

#[tokio::test]
async fn a_schema_the_database_cannot_encode_is_reported_at_once() {
    let admin = admin().await;
    // Skip without a server.
    if pg::fresh_db("ct_probe").await.is_none() {
        return;
    }
    admin.db_execute("DROP DATABASE IF EXISTS orm_test_ct_latin1 WITH (FORCE)").await.unwrap();
    admin.db_execute("CREATE DATABASE orm_test_ct_latin1 ENCODING 'LATIN1' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0").await.unwrap();
    let url = format!("{}/orm_test_ct_latin1?schema=%E2%82%AC&pool_timeout=0", pg::base_url());

    let t = Instant::now();
    let e = OrmClient::new(&url).unwrap().connect().await.unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(e.code(), Some("P1001"), "{e}");
    assert!(e.message().starts_with("Can't reach database server at `"), "{e}");
    assert!(e.message().contains("has no equivalent in encoding"), "{e}");

    let t = Instant::now();
    let e = OrmClient::new(&url).unwrap().query_raw("SELECT 1", vec![]).await.unwrap_err();
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(e.code(), Some("P1001"), "{e}");
    assert!(e.message().contains("has no equivalent in encoding"), "{e}");

    admin.db_execute("DROP DATABASE IF EXISTS orm_test_ct_latin1 WITH (FORCE)").await.unwrap();
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
    // Every slot comes back: two queries at once still run, well inside the pool timeout.
    let t = Instant::now();
    let (a, b) = tokio::join!(db.query_raw("SELECT 1", vec![]), db.query_raw("SELECT 2", vec![]));
    a.unwrap();
    b.unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
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
        assert!((got - s).abs() < 1.0, "{q}: original {s} s, corvis-orm {got} s");
    }
}

#[tokio::test]
async fn fixture_refused_port() {
    let fx = fixture();
    let base = "postgresql://u:p@127.0.0.1:1/x";
    for (q, case) in fx["refused"].as_object().unwrap() {
        assert_eq!(case["error"]["name"], "PrismaClientInitializationError", "{q}");
        assert!(case["error"]["message"].as_str().unwrap().contains("Can't reach database server at `127.0.0.1:1`"), "{q}");
    }
    // Difference (design decision 6): the original fails after about 2 s whatever the parameters say; corvis-orm retries
    // the refused connection inside `connect_timeout` and then fails with the same `P1001`.
    for (q, limit) in [("connect_timeout=1", 1.0), ("default", 5.0), ("pool_timeout=0&connect_timeout=2", 2.0)] {
        let (r, took) = run(with_q(base, q)).await;
        let e = r.unwrap_err();
        assert_eq!(e.code(), Some("P1001"), "{q}");
        assert!(e.message().starts_with("Can't reach database server at `127.0.0.1:1`"), "{q}: {e}");
        assert!((took.as_secs_f64() - limit).abs() < 1.0, "{q}: {took:?}");
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

    let Some(_) = pg::fresh_db("ct_fixture_role").await else { return };
    let admin = admin().await;
    reset_role(&admin, "orm_ct_role_c").await;
    admin.db_execute("CREATE ROLE orm_ct_role_c LOGIN PASSWORD 'ctpw' CONNECTION LIMIT 0").await.unwrap();
    let (r, took) = run(role_url("orm_ct_role_c", "orm_test_ct_fixture_role", "connect_timeout=1&pool_timeout=0")).await;
    reset_role(&admin, "orm_ct_role_c").await;
    let e = r.unwrap_err();
    assert_eq!(e.code(), Some("P1001"), "{e}");
    assert!(took >= Duration::from_millis(900) && took < Duration::from_secs(3), "{took:?}");
}

#[tokio::test]
async fn a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back() {
    let Some(url) = pg::fresh_db("ct_tx_cancel").await else { return };
    for q in ["connection_limit=1&pool_timeout=0&connect_timeout=1", "connection_limit=1&pool_timeout=0&connect_timeout=0"] {
        let db = OrmClient::new(format!("{url}?{q}")).unwrap();
        db.connect().await.unwrap();
        // Dropped while a long query runs: sqlx rolls back and pings only after that query ends (about 3 s).
        let cancelled = tokio::time::timeout(
            Duration::from_millis(200),
            db._transaction(|tx| async move { tx.query_raw("SELECT pg_sleep(3)", vec![]).await }),
        )
        .await;
        assert!(cancelled.is_err());
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
