//! `connection_limit` and `pool_timeout` in the database URL, replayed against the fixture recorded
//! with the original client (`tests/fixtures/pool_url.json`, script `corvis-tests/tmp/orm/pool/cases.js`).
//! The Postgres tests are skipped without a server (see `common/pg.rs`); the SQLite replay always runs.

#[path = "common/pg.rs"]
mod pg;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, Instant};

use corvis_orm::OrmClientOptions;
use corvis_orm::prelude::*;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/pool_url.json")).unwrap()
}

fn with_query(url: &str, q: &str) -> String {
    if q.is_empty() { url.to_string() } else { format!("{url}?{q}") }
}

/// Runs `n` concurrent `pg_sleep(0.3)` queries on `db` and returns the peak of active sleepers and
/// of server backends of the database (excluding the monitor's own connection).
async fn peak(base: &str, db: &OrmClient, n: usize) -> (i64, i64) {
    let monitor = OrmClient::new(with_query(base, "connection_limit=1")).unwrap();
    let max_active = Arc::new(AtomicI64::new(0));
    let max_backends = Arc::new(AtomicI64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let poll = {
        let (a, b, stop) = (max_active.clone(), max_backends.clone(), stop.clone());
        tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                let rows = monitor
                    .query_raw(
                        "SELECT count(*) FILTER (WHERE query LIKE '%pg_sleep%' AND state = 'active')::int8 AS a, count(*)::int8 AS b \
                         FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()",
                        vec![],
                    )
                    .await
                    .unwrap();
                a.fetch_max(rows[0]["a"].as_i64().unwrap(), Ordering::Relaxed);
                b.fetch_max(rows[0]["b"].as_i64().unwrap(), Ordering::Relaxed);
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            monitor.disconnect().await.unwrap();
        })
    };
    let mut tasks = Vec::new();
    for _ in 0..n {
        let db = db.clone();
        tasks.push(tokio::spawn(async move { db.query_raw("SELECT 1 FROM pg_sleep(0.3)", vec![]).await }));
    }
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    poll.await.unwrap();
    (max_active.load(Ordering::Relaxed), max_backends.load(Ordering::Relaxed))
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
        let (active, backends) = peak(&base, &db, n).await;
        assert_eq!((active, backends), (expected_active, expected_backends), "URL query `{q}`");
        db.disconnect().await.unwrap();
    }
}

#[tokio::test]
async fn limit_of_two_serves_six_queries() {
    let Some(base) = pg::fresh_db("pool_two").await else { return };
    let db = OrmClient::new(with_query(&base, "connection_limit=2")).unwrap();
    let (active, backends) = peak(&base, &db, 6).await;
    assert_eq!((active, backends), (2, 2));
    db.disconnect().await.unwrap();
}

#[tokio::test]
async fn builder_wins_over_the_url() {
    let Some(base) = pg::fresh_db("pool_builder").await else { return };
    let db = OrmClient::with_options(OrmClientOptions::new(with_query(&base, "connection_limit=2")).max_connections(4)).unwrap();
    let (active, backends) = peak(&base, &db, 12).await;
    assert_eq!((active, backends), (4, 4));
    db.disconnect().await.unwrap();
}

#[tokio::test]
async fn tls_parameter_is_kept_and_limit_applies() {
    let Some(base) = pg::fresh_db("pool_tls").await else { return };
    // `sslmode=disable` reaches the driver (an unknown pool key would only be logged, but a
    // `sslmode` that was dropped would still connect), and the limit is applied.
    let db = OrmClient::new(with_query(&base, "sslmode=disable&connection_limit=3")).unwrap();
    let (active, _) = peak(&base, &db, 9).await;
    assert_eq!(active, 3);
    db.disconnect().await.unwrap();
}

/// Holds the only connection with a transaction for `hold` while a query runs outside it.
async fn wait_behind_transaction(url: String, hold: Duration) -> (Result<Vec<Value>, OrmError>, Duration) {
    let db = OrmClient::new(url).unwrap();
    db.connect().await.unwrap();
    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();
    let holder = {
        let db = db.clone();
        tokio::spawn(async move {
            db._transaction(|tx| async move {
                tx.query_raw("SELECT 1", vec![]).await?;
                let _ = held_tx.send(());
                tokio::time::sleep(hold).await;
                Ok::<_, OrmError>(())
            })
            .await
        })
    };
    held_rx.await.unwrap();
    let t = Instant::now();
    let res = db.query_raw("SELECT 1", vec![]).await;
    let waited = t.elapsed();
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
        let (res, waited) = wait_behind_transaction(with_query(&base, query), Duration::from_secs(hold_s)).await;
        let want_ms = rec["waited_ms"].as_u64().unwrap() as i64;
        assert!((waited.as_millis() as i64 - want_ms).abs() < 500, "`{query}`: waited {waited:?}, original {want_ms} ms");
        if rec["ok"].as_bool().unwrap() {
            res.unwrap_or_else(|e| panic!("`{query}`: {e}"));
        } else {
            let e = res.unwrap_err();
            assert_eq!(e.code(), rec["error"]["code"].as_str(), "`{query}`");
            assert!(e.to_string().starts_with("Timed out fetching a new connection from the connection pool."), "{e}");
        }
    }
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
