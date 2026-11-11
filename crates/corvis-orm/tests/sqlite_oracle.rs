//! Differential test against the reference JavaScript client on SQLite.
//!
//! `tests/fixtures/oracle_sqlite/results.json` was recorded by running every case with the reference client
//! (`corvis-tests/tmp/orm/oracle/sqlite/cases.js`: the Postgres oracle's cases that apply to SQLite, plus SQLite
//! cases) on `tests/fixtures/oracle_sqlite/schema.prisma`, on a database built from `migration.sql`, the reference
//! client's own migration for that schema. The cases run here twice, in order, through the models generated from that
//! schema (`tests/generated/oracle_sqlite.rs`): on a database built from that migration (an app migrated from the
//! reference client) and on one built by `db push` (the examples' path). Both must give the recorded results, compared
//! like the Postgres oracle (`tests/common/oracle.rs`), except the cases of [`KNOWN_DIFFERENCES`].

#[path = "common/oracle.rs"]
mod compare;

#[allow(dead_code)]
mod oracle {
    include!("generated/oracle_sqlite.rs");
}

use corvis_orm::{Model, ModelMeta, OrmClient};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Db {
    Migrated,
    Pushed,
}

/// Cases that differ from the recording on purpose: (label, databases, reason). An entry that no longer differs
/// fails the test, so the table cannot go stale.
const KNOWN_DIFFERENCES: &[(&str, &[Db], &str)] = &[
    (
        "sqlite.contains_wildcards",
        &[Db::Migrated, Db::Pushed],
        "the reference client does not escape `%` in `contains` on SQLite, so `contains: \"%\"` matches every row; \
         Corvis matches it literally, as the reference client does on Postgres",
    ),
    ("sqlite.contains_underscore", &[Db::Migrated, Db::Pushed], "the same for `_`, which `LIKE` reads as any character"),
    (
        "sec.decimal_write_overflow",
        &[Db::Migrated, Db::Pushed],
        "the security bound of the hardening review: a `Decimal` written on SQLite must fit `Decimal(65, 30)` (or its \
         `@db.Decimal(p, s)`), so that a short input cannot store a huge value; the reference client stores `1e100`",
    ),
    (
        "err.int_overflow_create",
        &[Db::Migrated, Db::Pushed],
        "security review of #188: an `Int` beyond 32 bits is never written on SQLite (Corvis fails like Postgres does, \
         before any SQL runs); the reference client stores it and then fails every read of the row with `P2023`, so \
         one such value breaks every list of the table",
    ),
    (
        "arith.increment_overflow",
        &[Db::Migrated, Db::Pushed],
        "the same for an arithmetic update whose result leaves 32 bits: Corvis fails the statement, which SQLite undoes, \
         with the error Postgres gives (`integer out of range`); the reference client stores 2147483648",
    ),
    (
        "arith.after_overflow",
        &[Db::Migrated, Db::Pushed],
        "follows from `arith.increment_overflow`: the row keeps 2147483647 and reads, where the reference client fails \
         with `P2023`",
    ),
    (
        "json.nested_jsonnull_create",
        &[Db::Migrated, Db::Pushed],
        "security review of #188, round 2: a `DbNull` / `JsonNull` / `AnyNull` inside a Json value (not its whole value) \
         is a validation error in Corvis; the reference client stores its name (`{\"a\": \"JsonNull\"}`)",
    ),
    ("json.nested_jsonnull_create_many", &[Db::Migrated, Db::Pushed], "the same, in `createMany`"),
    (
        "json.nested_anynull_equals",
        &[Db::Migrated, Db::Pushed],
        "the same in a filter, where the reference client compares with `{\"a\": \"AnyNull\"}`",
    ),
    ("json.nested_after", &[Db::Migrated, Db::Pushed], "follows from `json.nested_jsonnull_create*`: Corvis created neither row"),
    (
        "json.nested_cleanup",
        &[Db::Migrated, Db::Pushed],
        "the same: Corvis deletes 0 rows where the reference client deletes 2, and then both have the same rows",
    ),
    (
        "arith.big_increment_2",
        &[Db::Migrated, Db::Pushed],
        "a `BigInt` arithmetic update past 64 bits fails like on Postgres (`bigint out of range`) and SQLite undoes it; the \
         reference client stores a float there and then fails every read of the row with `P2023`",
    ),
    ("arith.big_after", &[Db::Migrated, Db::Pushed], "follows from `arith.big_increment_2`: the row keeps its value and reads"),
    (
        "arith.ratio_multiply_overflow",
        &[Db::Migrated, Db::Pushed],
        "a `Float` arithmetic update to an infinite value fails like on Postgres (`value out of range: overflow`); the \
         reference client stores `inf` and then fails every read of the row with `P2023`",
    ),
    ("arith.ratio_after", &[Db::Migrated, Db::Pushed], "follows from `arith.ratio_multiply_overflow`"),
];

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

fn recording() -> Value {
    serde_json::from_str(include_str!("fixtures/oracle_sqlite/results.json")).unwrap()
}

async fn run(db: &OrmClient, which: Db) {
    let doc = recording();
    let cases = doc["cases"].as_array().unwrap();
    let only = std::env::var("ORACLE_ONLY").ok();
    let mut failures = Vec::new();
    let mut known_seen = Vec::new();
    for c in cases {
        let label = c["label"].as_str().unwrap();
        let mut args = c["args"].clone();
        compare::revive(&mut args);
        let res = db.query_json(meta_of(c["model"].as_str().unwrap()), c["action"].as_str().unwrap(), args).await;
        if only.as_deref().is_some_and(|o| !label.starts_with(o)) {
            continue;
        }
        let known = KNOWN_DIFFERENCES.iter().find(|(l, dbs, _)| *l == label && dbs.contains(&which));
        match (compare::compare(c, res), known) {
            (Some(diff), None) => failures.push(diff),
            (Some(_), Some(_)) => known_seen.push(label),
            (None, Some((_, _, reason))) => {
                failures.push(format!("[{label}] is listed in KNOWN_DIFFERENCES ({reason}) but no longer differs: remove the entry"))
            }
            (None, None) => {}
        }
    }
    if !failures.is_empty() {
        panic!("{which:?}: {} of {} cases differ from the reference client:\n{}", failures.len(), cases.len(), failures.join("\n"));
    }
}

#[test]
fn the_recording_names_the_client_and_the_script() {
    let doc = recording();
    assert!(doc["recorded_with"].as_str().is_some_and(|v| v.starts_with("@prisma/client 6.")), "{}", doc["recorded_with"]);
    assert_eq!(doc["script"], "corvis-tests/tmp/orm/oracle/sqlite/cases.js");
}

#[tokio::test]
async fn matches_reference_client_on_the_reference_migration() {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_execute(include_str!("fixtures/oracle_sqlite/migration.sql")).await.unwrap();
    run(&db, Db::Migrated).await;
}

#[tokio::test]
async fn matches_reference_client_on_a_pushed_database() {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<(
        oracle::User,
        oracle::Profile,
        oracle::Post,
        oracle::Comment,
        oracle::Category,
        oracle::Group,
        oracle::Wallet,
        oracle::Pair,
        oracle::Entry,
        oracle::Rate,
    )>()
    .await
    .unwrap();
    run(&db, Db::Pushed).await;
}
