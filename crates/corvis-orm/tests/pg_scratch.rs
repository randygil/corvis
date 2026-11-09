//! The naming and clean-up rules of the scratch databases and roles of the Postgres tests (see `common/pg.rs`):
//! per-run names within Postgres' identifier limit, and the sweep of stale leftovers.

#[path = "common/pg.rs"]
mod pg;

use corvis_orm::prelude::*;

#[test]
fn scratch_names_are_unique_per_run_and_fit_the_identifier_limit() {
    let long = "x".repeat(100);
    for name in [pg::scratch_name("orm_test_", &long), pg::scratch_name("orm_", &long)] {
        assert!(name.len() <= 63, "{name} is {} bytes", name.len());
        assert!(name.ends_with(pg::run_id()), "{name}");
    }
    let db = pg::scratch_name("orm_test_", "feat_x");
    assert_eq!(db, format!("orm_test_feat_x_{}", pg::run_id()));
    assert!(db.contains(&format!("_{}_", std::process::id())), "{db}");
    assert_eq!(pg::db_of("postgres://u:p@h:1/some_db?schema=a"), "some_db");
}

async fn present(admin: &OrmClient, catalog: &str, column: &str, name: &str) -> bool {
    let rows = admin.query_raw(&format!("SELECT count(*)::int8 AS n FROM {catalog} WHERE {column} = $1"), vec![json!(name)]).await.unwrap();
    rows[0]["n"] == json!(1)
}

#[tokio::test]
async fn the_sweep_drops_only_stale_scratch_objects_of_dead_runs() {
    let Some(base) = pg::fresh_db("scratch_sweep").await else { return };
    let server = base.rsplit_once('/').unwrap().0.to_string();
    let admin = OrmClient::new(format!("{server}/postgres")).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let old = now - pg::STALE_AFTER_SECS - 60;
    let stale_db = format!("orm_test_sweep_old_{old}_1_abcd");
    let stale_role = format!("orm_sweep_old_{old}_1_abcd");
    let recent_db = format!("orm_test_sweep_new_{}_1_abcd", now - 60);
    let plain_db = "orm_test_sweep_plain".to_string();
    let cleanup = [
        format!("DROP DATABASE IF EXISTS {stale_db}"),
        format!("DROP ROLE IF EXISTS {stale_role}"),
        format!("DROP DATABASE IF EXISTS {recent_db}"),
        format!("DROP DATABASE IF EXISTS {plain_db}"),
    ];
    for sql in &cleanup {
        admin.execute_raw(sql, vec![]).await.unwrap();
    }
    for sql in [
        format!("CREATE DATABASE {stale_db}"),
        format!("CREATE ROLE {stale_role}"),
        format!("CREATE DATABASE {recent_db}"),
        format!("CREATE DATABASE {plain_db}"),
    ] {
        admin.execute_raw(&sql, vec![]).await.unwrap();
    }

    let opts = <sqlx::postgres::PgConnectOptions as std::str::FromStr>::from_str(&format!("{server}/postgres")).unwrap();
    let mut c = sqlx::ConnectOptions::connect(&opts).await.unwrap();
    pg::sweep_stale(&mut c).await;

    let results = (
        present(&admin, "pg_database", "datname", &stale_db).await,
        present(&admin, "pg_roles", "rolname", &stale_role).await,
        present(&admin, "pg_database", "datname", &recent_db).await,
        present(&admin, "pg_database", "datname", &plain_db).await,
    );
    for sql in &cleanup {
        let _ = admin.execute_raw(sql, vec![]).await;
    }
    assert_eq!(results, (false, false, true, true), "(stale db, stale role, recent db, name without a run suffix)");
}
