//! The Postgres SQL of every case of the Postgres oracle, compared with `tests/fixtures/pg_sql_snapshot.json`.
//!
//! The cases of `tests/fixtures/oracle/results.json` run in order on a fresh database built from the recorded
//! migration (like `tests/pg_oracle.rs`), and every statement the client sends for them is recorded with
//! [`super::recorder`]: its text and the types of its parameters (never the values, which hold clock times).
//! A change to the engine that alters the SQL sent to Postgres for any of these cases fails here.
//!
//! Regenerate the fixture (only when a change is meant to alter Postgres SQL) with
//! `CORVIS_ORM_UPDATE_PG_SNAPSHOT=1`. Without a reachable server the test is skipped, unless
//! `CORVIS_ORM_PG_REQUIRED=1`.

use serde_json::{Value, json};
use sqlx::{ConnectOptions, Connection, Executor};
use std::str::FromStr;

use super::recorder::{Recorded, capture};
use crate::{Model, ModelMeta, OrmClient};

#[allow(dead_code)]
mod oracle {
    include!("../../tests/generated/oracle.rs");
}

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pg_sql_snapshot.json");

fn meta_of(model: &str) -> &'static ModelMeta {
    match model {
        "user" => oracle::User::meta(),
        "profile" => oracle::Profile::meta(),
        "post" => oracle::Post::meta(),
        "comment" => oracle::Comment::meta(),
        "category" => oracle::Category::meta(),
        "group" => oracle::Group::meta(),
        "wallet" => oracle::Wallet::meta(),
        "pair" => oracle::Pair::meta(),
        "entry" => oracle::Entry::meta(),
        "rate" => oracle::Rate::meta(),
        other => panic!("unknown model {other}"),
    }
}

fn base_url() -> String {
    std::env::var("CORVIS_ORM_PG").unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:55432".to_string())
}

/// Creates a scratch database and returns its name, or `None` when the server is not reachable.
async fn scratch_db(base: &str) -> Option<String> {
    let required = std::env::var("CORVIS_ORM_PG_REQUIRED").is_ok_and(|v| v == "1");
    let admin = sqlx::postgres::PgConnectOptions::from_str(&format!("{base}/postgres")).expect("CORVIS_ORM_PG is a valid URL");
    let mut c = match admin.connect().await {
        Ok(c) => c,
        Err(e) if required => panic!("CORVIS_ORM_PG_REQUIRED=1 but Postgres is not reachable: {e}"),
        Err(_) => return None,
    };
    let db = format!("orm_test_pg_sql_snapshot_{}", std::process::id());
    c.execute(format!("DROP DATABASE IF EXISTS \"{db}\" WITH (FORCE)").as_str()).await.unwrap();
    c.execute(format!("CREATE DATABASE \"{db}\"").as_str()).await.unwrap();
    let _ = c.close().await;
    Some(db)
}

async fn drop_db(base: &str, db: &str) {
    if let Ok(opts) = sqlx::postgres::PgConnectOptions::from_str(&format!("{base}/postgres"))
        && let Ok(mut c) = opts.connect().await
    {
        let _ = c.execute(format!("DROP DATABASE IF EXISTS \"{db}\" WITH (FORCE)").as_str()).await;
        let _ = c.close().await;
    }
}

#[tokio::test]
async fn postgres_sql_of_the_oracle_cases_is_unchanged() {
    let base = base_url();
    let Some(db_name) = scratch_db(&base).await else {
        eprintln!("skipping: Postgres not reachable");
        return;
    };
    let db = OrmClient::new(format!("{base}/{db_name}")).unwrap();
    db.db_execute(include_str!("../../tests/fixtures/oracle/migration.sql")).await.unwrap();
    let cases: Vec<Value> = serde_json::from_str(include_str!("../../tests/fixtures/oracle/results.json")).unwrap();
    let mut snapshot = Vec::new();
    for c in cases.iter().filter(|c| c["model"] != "$raw") {
        let label = c["label"].as_str().unwrap();
        let model = meta_of(c["model"].as_str().unwrap());
        let (_, statements) = capture(db.query_json(model, c["action"].as_str().unwrap(), c["args"].clone())).await;
        snapshot.push(json!({ "label": label, "statements": statements }));
    }
    let _ = db.disconnect().await;
    drop_db(&base, &db_name).await;

    let got = serde_json::to_string_pretty(&Value::Array(snapshot)).unwrap() + "\n";
    if std::env::var("CORVIS_ORM_UPDATE_PG_SNAPSHOT").is_ok_and(|v| v == "1") {
        std::fs::write(FIXTURE, &got).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(FIXTURE).expect("tests/fixtures/pg_sql_snapshot.json is missing");
    let expected: Vec<Value> = serde_json::from_str(&expected).unwrap();
    let got: Vec<Value> = serde_json::from_str(&got).unwrap();
    let mut diffs = Vec::new();
    for (g, x) in got.iter().zip(expected.iter()) {
        if g != x {
            let statements = |v: &Value| serde_json::from_value::<Vec<Recorded>>(v["statements"].clone()).unwrap();
            diffs.push(format!("[{}]\n  expected: {:#?}\n  got:      {:#?}", x["label"], statements(x), statements(g)));
        }
    }
    assert_eq!(got.len(), expected.len(), "the number of oracle cases changed");
    assert!(diffs.is_empty(), "{} oracle cases send different SQL to Postgres:\n{}", diffs.len(), diffs.join("\n"));
}
