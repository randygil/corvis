//! Postgres helpers for the integration tests.
//!
//! Tests use `CORVIS_ORM_PG` (default `postgres://postgres:postgres@127.0.0.1:55432`)
//! and databases named `orm_test_*`. When the server is not reachable they are skipped.
#![allow(dead_code)]

use sqlx::{ConnectOptions, Connection, Executor};
use std::str::FromStr;

pub fn base_url() -> String {
    std::env::var("CORVIS_ORM_PG").unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:55432".to_string())
}

/// Drops and recreates `orm_test_<name>`; returns its URL, or `None` when Postgres is down.
pub async fn fresh_db(name: &str) -> Option<String> {
    let db = format!("orm_test_{name}");
    let admin = sqlx::postgres::PgConnectOptions::from_str(&format!("{}/postgres", base_url())).ok()?;
    let mut c = match admin.connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping: Postgres not reachable ({e})");
            return None;
        }
    };
    c.execute(format!("DROP DATABASE IF EXISTS \"{db}\" WITH (FORCE)").as_str()).await.ok()?;
    c.execute(format!("CREATE DATABASE \"{db}\"").as_str()).await.ok()?;
    let _ = c.close().await;
    Some(format!("{}/{db}", base_url()))
}

/// The folder of an original reference application (`corvis-tests/nest/<app>`, outside
/// this repository), when it is present.
pub fn app_dir(rel: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var("CORVIS_TESTS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../corvis-tests"));
    let p = base.join("nest").join(rel);
    if p.exists() { Some(p) } else { None }
}
