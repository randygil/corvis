//! Postgres helpers for the integration tests.
//!
//! Tests use `CORVIS_ORM_PG` (default `postgres://postgres:postgres@127.0.0.1:55432`)
//! and databases named `orm_test_*`. When the server is not reachable they are skipped,
//! unless `CORVIS_ORM_PG_REQUIRED=1` is set (CI sets it): then the test fails instead, so
//! a green run means the Postgres tests really ran. Failure messages name the server's
//! host and port, never the password.
#![allow(dead_code)]

use sqlx::{ConnectOptions, Connection, Executor};
use std::str::FromStr;

pub fn base_url() -> String {
    std::env::var("CORVIS_ORM_PG").unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:55432".to_string())
}

/// Whether `CORVIS_ORM_PG_REQUIRED=1` turns a missing server into a test failure.
pub fn required() -> bool {
    std::env::var("CORVIS_ORM_PG_REQUIRED").is_ok_and(|v| v == "1")
}

/// Drops and recreates `orm_test_<name>`; returns its URL, or `None` when Postgres is down
/// (a panic instead when `CORVIS_ORM_PG_REQUIRED=1`).
pub async fn fresh_db(name: &str) -> Option<String> {
    fresh_db_at(&base_url(), required(), name).await
}

/// [`fresh_db`] against an explicit server URL and policy.
pub async fn fresh_db_at(base: &str, required: bool, name: &str) -> Option<String> {
    let db = format!("orm_test_{name}");
    let admin = match sqlx::postgres::PgConnectOptions::from_str(&format!("{base}/postgres")) {
        Ok(o) => o,
        Err(e) => return unavailable(base, required, "has an invalid URL", e),
    };
    let mut c = match admin.connect().await {
        Ok(c) => c,
        Err(e) => return unavailable(base, required, "is not reachable", e),
    };
    if let Err(e) = c.execute(format!("DROP DATABASE IF EXISTS \"{db}\" WITH (FORCE)").as_str()).await {
        return unavailable(base, required, &format!("could not drop database {db}"), e);
    }
    if let Err(e) = c.execute(format!("CREATE DATABASE \"{db}\"").as_str()).await {
        return unavailable(base, required, &format!("could not create database {db}"), e);
    }
    let _ = c.close().await;
    Some(format!("{base}/{db}"))
}

/// Skips (returns `None`) or, when `required`, panics with the host and the error.
fn unavailable(base: &str, required: bool, what: &str, e: impl std::fmt::Display) -> Option<String> {
    let e = redact(base, &e.to_string());
    if required {
        panic!("CORVIS_ORM_PG_REQUIRED=1 but the Postgres server at {} (CORVIS_ORM_PG) {what}: {e}", host(base));
    }
    eprintln!("skipping: Postgres not reachable ({} {what}: {e})", host(base));
    None
}

/// The `user:password@` part of a `postgres://` URL, split at the last `@` before the path.
fn userinfo(base: &str) -> Option<&str> {
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let authority = rest.split(['/', '?']).next().unwrap_or(rest);
    authority.rsplit_once('@').map(|(u, _)| u)
}

/// `host:port` of a server URL, without credentials.
pub fn host(base: &str) -> &str {
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let rest = match userinfo(base) {
        Some(u) => &rest[u.len() + 1..],
        None => rest,
    };
    rest.split(['/', '?']).next().unwrap_or(rest)
}

/// Removes the URL's password from `msg`, in case an error message quotes it.
fn redact(base: &str, msg: &str) -> String {
    match userinfo(base).and_then(|u| u.split_once(':')).map(|(_, p)| p) {
        Some(p) if !p.is_empty() => msg.replace(p, "***"),
        _ => msg.to_string(),
    }
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
