//! `connection_limit` and `pool_timeout` in the database URL, replayed against the fixture recorded
//! with the original client (`tests/fixtures/pool_url.json`, script `corvis-tests/tmp/orm/pool/cases.js`).
//! The Postgres tests are skipped without a server (see `common/pg.rs`); the SQLite replay always runs.
//!
//! Timing rule: connections are held by a gate or a handshake, not by overlapping sleeps; a duration has a tight
//! lower bound and an upper bound below the nearest wrong value, named in the assertion (see `common/gate.rs`).

#[path = "common/gate.rs"]
mod gate;
#[path = "common/pg.rs"]
mod pg;

use std::time::{Duration, Instant};

use corvis_orm::OrmClientOptions;
use corvis_orm::prelude::*;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/pool_url.json")).unwrap()
}

fn with_query(url: &str, q: &str) -> String {
    if q.is_empty() { url.to_string() } else { format!("{url}?{q}") }
}

/// Runs `n` concurrent gated queries on `db` (the database at `db_url`) and returns the maxima of queries
/// blocked at the server at once and of client backends of the database, as seen by the gate's observer.
/// The queries stay in flight until `expected` of them are blocked and the settle window has passed
/// (see `common/gate.rs`), so the result does not depend on how fast the machine is.
async fn gated_peak(db_url: &str, db: &OrmClient, n: usize, expected: i64) -> (i64, i64) {
    let gate = gate::Gate::close(db_url, 7001).await;
    let sql = gate.sql();
    let mut tasks: Vec<gate::Task> = Vec::new();
    for _ in 0..n {
        let (db, sql) = (db.clone(), sql.clone());
        tasks.push(tokio::spawn(async move { db.query_raw(&sql, vec![]).await }));
    }
    let seen = gate.wait_for(expected, &mut tasks).await;
    gate.open().await;
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    seen
}

#[tokio::test]
async fn recorded_connection_counts_match() {
    let Some(base) = pg::fresh_db("pool_counts").await else { return };
    let fx = fixture();
    for (query, rec) in fx["concurrency"].as_object().unwrap() {
        // The original treats these as no limit; corvis-orm rejects them (see invalid_values_are_rejected_like_the_original).
        if query == "connection_limit=0" || query == "connection_limit=4294967296" {
            continue;
        }
        let n = rec["queries"].as_u64().unwrap() as usize;
        let (expected_active, expected_backends) = if query == "default" {
            // The default is `2 * physical cores + 1`, so it depends on the machine.
            let want = (num_cpus::get_physical() * 2 + 1).min(n) as i64;
            (want, want)
        } else {
            (rec["max_active"].as_i64().unwrap(), rec["max_backends"].as_i64().unwrap())
        };
        let q = if query == "default" { "" } else { query.as_str() };
        let db = OrmClient::new(with_query(&base, q)).unwrap();
        let (active, backends) = gated_peak(&base, &db, n, expected_active).await;
        assert_eq!((active, backends), (expected_active, expected_backends), "URL query `{q}` with {n} queries");
        db.disconnect().await.unwrap();
    }
}

#[tokio::test]
async fn limit_of_two_serves_six_queries() {
    let Some(base) = pg::fresh_db("pool_two").await else { return };
    let db = OrmClient::new(with_query(&base, "connection_limit=2")).unwrap();
    let (active, backends) = gated_peak(&base, &db, 6, 2).await;
    assert_eq!((active, backends), (2, 2), "connection_limit=2 with 6 queries");
    db.disconnect().await.unwrap();
}

#[tokio::test]
async fn builder_wins_over_the_url() {
    let Some(base) = pg::fresh_db("pool_builder").await else { return };
    let db = OrmClient::with_options(OrmClientOptions::new(with_query(&base, "connection_limit=2")).max_connections(4)).unwrap();
    let (active, backends) = gated_peak(&base, &db, 12, 4).await;
    assert_eq!((active, backends), (4, 4), "connection_limit=2 in the URL, max_connections(4), 12 queries");
    db.disconnect().await.unwrap();
}

#[tokio::test]
async fn tls_parameter_is_kept_and_limit_applies() {
    let Some(base) = pg::fresh_db("pool_tls").await else { return };
    // `sslmode=disable` reaches the driver (an unknown pool key would only be logged, but a
    // `sslmode` that was dropped would still connect), and the limit is applied.
    let db = OrmClient::new(with_query(&base, "sslmode=disable&connection_limit=3")).unwrap();
    let (active, backends) = gated_peak(&base, &db, 9, 3).await;
    assert_eq!((active, backends), (3, 3), "sslmode=disable&connection_limit=3 with 9 queries");
    db.disconnect().await.unwrap();
}

/// Holds the only connection with a transaction while a query runs outside it. The holder is released by the
/// test, never by a timer: with `hold: None` after the query has returned (it can only end by timing out);
/// with `hold: Some(h)` after `h`, checking first that the query is still pending. Returns the query's result
/// and how long it waited.
async fn wait_behind_transaction(url: String, hold: Option<Duration>) -> (Result<Vec<Value>, OrmError>, Duration) {
    let db = OrmClient::new(url).unwrap();
    db.connect().await.unwrap();
    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let holder = {
        let db = db.clone();
        tokio::spawn(async move {
            db._transaction_with(TransactionOptions::new().timeout(120_000), |tx| async move {
                tx.query_raw("SELECT 1", vec![]).await?;
                let _ = held_tx.send(());
                let _ = release_rx.await;
                Ok::<_, OrmError>(())
            })
            .await
        })
    };
    held_rx.await.unwrap();
    let query = {
        let db = db.clone();
        tokio::spawn(async move {
            let t = Instant::now();
            let res = db.query_raw("SELECT 1", vec![]).await;
            (res, t.elapsed())
        })
    };
    let (res, waited) = if let Some(hold) = hold {
        tokio::time::sleep(hold).await;
        assert!(!query.is_finished(), "the query ended before the holder was released after {hold:?}");
        let _ = release_tx.send(());
        query.await.unwrap()
    } else {
        let r = query.await.unwrap();
        let _ = release_tx.send(());
        r
    };
    holder.await.unwrap().unwrap();
    db.disconnect().await.unwrap();
    (res, waited)
}

#[tokio::test]
async fn recorded_waits_match() {
    let Some(base) = pg::fresh_db("pool_wait").await else { return };
    let fx = fixture();
    for (key, rec) in fx["wait"].as_object().unwrap() {
        let (query, hold) = key.split_once(" (hold ").unwrap();
        let hold_s: u64 = hold.trim_end_matches("s)").parse().unwrap();
        let want_ms = rec["waited_ms"].as_u64().unwrap();
        if rec["ok"].as_bool().unwrap() {
            // The query is still pending at the recorded hold time (the original waited `want_ms`) and succeeds
            // once the holder lets go. No upper bound: after the release it only needs a free connection.
            let (res, waited) = wait_behind_transaction(with_query(&base, query), Some(Duration::from_secs(hold_s))).await;
            res.unwrap_or_else(|e| panic!("`{query}`: {e}"));
            assert!(waited >= Duration::from_secs(hold_s), "`{query}`: waited {waited:?}, original {want_ms} ms");
        } else {
            // The pool timeout T is in the recorded message. Lower bound tight (timers never fire early); the upper
            // bound `T + 2 s` is below the nearest wrong values: 5 s (the other value of a repeated key) and the
            // 10 s default.
            let msg = rec["error"]["message"].as_str().unwrap();
            let t_s: u64 = msg.split("Current connection pool timeout: ").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
            let (res, waited) = wait_behind_transaction(with_query(&base, query), None).await;
            let e = res.unwrap_err();
            assert_eq!(e.code(), rec["error"]["code"].as_str(), "`{query}`");
            assert!(e.to_string().starts_with("Timed out fetching a new connection from the connection pool."), "{e}");
            assert!(waited >= Duration::from_millis(t_s * 1000 - 50), "`{query}`: waited {waited:?}, original {want_ms} ms");
            assert!(
                waited < Duration::from_millis(t_s * 1000 + 2000),
                "`{query}`: waited {waited:?} (original {want_ms} ms); the other value of a repeated key or the 10 s default would show 5 s or more"
            );
        }
    }
}

#[tokio::test]
async fn gate_holds_queries_until_opened() {
    let Some(base) = pg::fresh_db("pool_gate").await else { return };
    let db = OrmClient::new(with_query(&base, "connection_limit=2")).unwrap();
    let gate = gate::Gate::close(&base, 7002).await;
    let sql = gate.sql();
    let mut tasks: Vec<gate::Task> = Vec::new();
    for _ in 0..5 {
        let (db, sql) = (db.clone(), sql.clone());
        tasks.push(tokio::spawn(async move { db.query_raw(&sql, vec![]).await }));
    }
    assert_eq!(gate.wait_for(2, &mut tasks).await, (2, 2));
    assert!(tasks.iter().all(|t| !t.is_finished()), "a gated query finished before the gate opened");
    gate.open().await;
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    db.disconnect().await.unwrap();
}

#[tokio::test]
async fn migrate_deploy_creates_the_database_with_a_pool_parameter() {
    let Some(base) = pg::fresh_db("pool_ensure").await else { return };
    // `?connection_limit=2` must not reach the driver when `ensure_database` parses the URL.
    let name = "orm_test_pool_ensure";
    let admin_url = format!("{}/postgres", base.rsplit_once('/').unwrap().0);
    let admin = OrmClient::new(admin_url).unwrap();
    admin.execute_raw(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"), vec![]).await.unwrap();

    let dir = std::env::temp_dir().join(format!("corvis-orm-pool-migrations-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("20240101000000_init")).unwrap();
    std::fs::write(dir.join("20240101000000_init/migration.sql"), "CREATE TABLE t (id int PRIMARY KEY);").unwrap();

    let db = OrmClient::new(with_query(&base, "connection_limit=2")).unwrap();
    let applied = db.migrate_deploy(&dir).await.unwrap();
    assert_eq!(applied, vec!["20240101000000_init".to_string()]);
    let rows = admin.query_raw("SELECT count(*)::int8 AS n FROM pg_database WHERE datname = $1", vec![json!(name)]).await.unwrap();
    assert_eq!(rows[0]["n"], json!(1));

    db.disconnect().await.unwrap();
    admin.execute_raw(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"), vec![]).await.unwrap();
    admin.disconnect().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn invalid_values_are_rejected_like_the_original() {
    // The original rejects a non-numeric, negative, fractional or empty value with an
    // initialization error. It accepts `connection_limit=0` and `4294967296` as "no limit"; corvis-orm
    // rejects both (a pool needs at least one connection and a bounded size), see design.md.
    let fx = fixture();
    for (query, rec) in fx["invalid_postgres"].as_object().unwrap() {
        let original_rejects = !rec["ok"].as_bool().unwrap();
        let documented_deviation = query == "connection_limit=0" || query == "connection_limit=4294967296";
        assert!(original_rejects || documented_deviation, "unexpected original behavior for `{query}`");
        let e = OrmClient::new(format!("postgresql://app:secret@127.0.0.1:1/db?{query}")).unwrap_err();
        assert_eq!(e.name(), "InitializationError", "`{query}`");
        // First sentence of the original's message.
        assert!(e.to_string().starts_with("The provided database string is invalid."), "`{query}`: {e}");
        assert!(!e.to_string().contains("secret"), "`{query}`: {e}");
        assert!(e.to_string().contains(query.split('=').next().unwrap()), "`{query}`: {e}");
    }
}

#[tokio::test]
async fn sqlite_urls_are_accepted_like_the_original() {
    let fx = fixture();
    let dir = std::env::temp_dir().join(format!("corvis-orm-pool-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("p.db").to_string_lossy().replace('\\', "/");
    for (key, rec) in fx["sqlite"].as_object().unwrap() {
        let url = key.replace("<db>", &f);
        let documented_deviation = key.ends_with("connection_limit=0") || key.ends_with("connection_limit=4294967296");
        let created = OrmClient::new(&url).map(|_| ());
        let accepted = match &created {
            Ok(()) => {
                let db = OrmClient::new(&url).unwrap();
                let ok = db.query_raw("SELECT 1", vec![]).await.is_ok();
                db.disconnect().await.unwrap();
                ok
            }
            Err(_) => false,
        };
        let original_ok = rec["ok"].as_bool().unwrap();
        if documented_deviation {
            assert!(original_ok && !accepted, "`{key}`");
        } else {
            assert_eq!(accepted, original_ok, "`{key}`");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
