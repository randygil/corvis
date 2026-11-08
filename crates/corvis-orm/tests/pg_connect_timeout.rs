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

#[tokio::test]
async fn the_parameter_is_removed_from_the_driver_url() {
    let Some(url) = pg::fresh_db("ct_param").await else { return };
    let db = OrmClient::new(format!("{url}?sslmode=disable&connect_timeout=3&schema=s")).unwrap();
    assert_eq!(db.connect().await.ok(), Some(()));
    let rows = db.query_raw("SELECT current_setting('search_path') AS p", vec![]).await.unwrap();
    assert_eq!(rows[0]["p"], "s");
}
