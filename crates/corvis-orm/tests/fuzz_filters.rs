//! Property-based fuzzing of `where` filters on in-memory SQLite: generated hostile filters on a seeded
//! table must produce results or validation errors, never database syntax errors, panics or changed data.
//!
//! `PROPTEST_CASES` overrides the case count (2,000 by default).

use corvis_orm::prelude::*;
use proptest::prelude::*;
use proptest::test_runner::{FileFailurePersistence, TestCaseError, TestRunner};

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "accounts")]
pub struct Account {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    #[unique]
    pub email: String,
    #[unique]
    pub reset_token: Option<String>,
    pub status: String,
    #[default(0)]
    pub version: i32,
    pub balance: Decimal,
    pub data: Option<serde_json::Value>,
    #[default(0)]
    pub big: i64,
    pub blob: Option<Vec<u8>>,
}

fn meta() -> &'static corvis_orm::ModelMeta {
    <Account as corvis_orm::Model>::meta()
}

async fn q(db: &OrmClient, action: &str, args: Value) -> corvis_orm::Result<Value> {
    db.query_json(meta(), action, args).await
}

/// Emails of the seeded rows: plain, quote-shaped and wildcard-shaped on purpose.
const SEED_EMAILS: [&str; 7] =
    ["victim@x", "other@x", "a'b", "x\" OR 1=1 --", "100%", "_", "'; DROP TABLE accounts; --"];

async fn seeded() -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<(Account,)>().await.unwrap();
    for (i, email) in SEED_EMAILS.iter().enumerate() {
        q(
            &db,
            "create",
            json!({ "data": {
                "email": email, "reset_token": format!("tok-{i}"), "status": if i % 2 == 0 { "PENDING" } else { "DONE" },
                "balance": "1.5", "version": i, "data": { "n": i }
            } }),
        )
        .await
        .unwrap();
    }
    db
}

async fn snapshot(db: &OrmClient) -> Value {
    q(db, "findMany", json!({ "orderBy": { "id": "asc" } })).await.unwrap()
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(2000)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

fn hostile_string() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(vec![
            "'", "\"", ";", "--", "/*", "*/", "DROP TABLE accounts", "' OR '1'='1", "'; DROP TABLE accounts; --", "\\", "%", "_",
            "\0", "a\0b", "1=1", ") OR (1=1", "`", "[", "]", "\n", "\u{2028}", "\u{e9}", "NULL", "$1", "?", ":x", "@p",
        ])
        .prop_map(String::from),
        2 => prop::sample::select(SEED_EMAILS.to_vec()).prop_map(String::from),
        2 => "\\PC{0,12}",
    ]
}

fn field_name() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => prop::sample::select(vec!["id", "email", "reset_token", "status", "version", "balance", "data", "big", "blob"])
            .prop_map(String::from),
        2 => prop::sample::select(vec![
            "nope", "", "__proto__", "\"email\"", "email; DROP TABLE accounts", "email--", "Email", "1", "accounts.email", "ROWID",
        ])
        .prop_map(String::from),
        1 => hostile_string(),
    ]
}

fn operator() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => prop::sample::select(vec![
            "equals", "not", "in", "notIn", "contains", "startsWith", "endsWith", "lt", "lte", "gt", "gte", "mode", "path",
            "string_contains", "has", "some", "is", "isNot", "AND", "OR", "NOT",
        ])
        .prop_map(String::from),
        2 => prop::sample::select(vec!["like", "regex", "equals; --", "$ne", "'", "", "EQUALS", "="]).prop_map(String::from),
    ]
}

fn scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        4 => hostile_string().prop_map(Value::String),
        2 => any::<i64>().prop_map(|n| json!(n)),
        1 => any::<f64>().prop_map(|n| json!(n)),
        1 => prop::sample::select(vec![json!(null), json!(true), json!(false), json!("insensitive"), json!("default"), json!(1e300)]),
    ]
}

/// A value of a field filter: a scalar, a list or an operator object, nested a few levels.
fn filter_value() -> impl Strategy<Value = Value> {
    scalar().prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            2 => prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            4 => prop::collection::vec((operator(), inner.clone()), 0..3).prop_map(|kv| Value::Object(kv.into_iter().collect())),
            1 => prop::collection::vec((field_name(), inner), 0..2).prop_map(|kv| Value::Object(kv.into_iter().collect())),
        ]
    })
}

fn where_arg() -> impl Strategy<Value = Value> {
    let leaf = prop::collection::vec((field_name(), filter_value()), 0..4).prop_map(|kv| Value::Object(kv.into_iter().collect()));
    leaf.prop_recursive(3, 24, 3, |inner| {
        prop_oneof![
            2 => prop::collection::vec(
                ("AND|OR|NOT", prop_oneof![inner.clone(), prop::collection::vec(inner.clone(), 0..3).prop_map(Value::Array)]),
                1..3
            )
            .prop_map(|kv| Value::Object(kv.into_iter().collect())),
            1 => inner,
        ]
    })
}

fn check_error(e: &corvis_orm::OrmError) -> Result<(), String> {
    let m = e.message().to_lowercase();
    if m.contains("syntax error") || m.contains("unrecognized token") || m.contains("unterminated") {
        return Err(format!("database syntax error: {e}"));
    }
    match e {
        corvis_orm::OrmError::Validation(_) | corvis_orm::OrmError::KnownRequest(_) => Ok(()),
        // Out-of-range integers fail before the database sees them, with the original's message.
        corvis_orm::OrmError::UnknownRequest(_) if m.contains("unable to fit integer value") => Ok(()),
        other => Err(format!("unexpected error kind {}: {other}", other.name())),
    }
}

#[test]
fn filters_never_break_the_query() {
    let rt = runtime();
    let db = rt.block_on(seeded());
    let before = rt.block_on(snapshot(&db));
    let mut runner = TestRunner::new(config(cases()));
    let strategy = (where_arg(), prop::sample::select(vec!["findMany", "count", "findFirst"]));
    let result = runner.run(&strategy, |(w, action)| {
        let args = if action == "count" { json!({ "where": w }) } else { json!({ "where": w, "take": 50 }) };
        match rt.block_on(q(&db, action, args.clone())) {
            Ok(_) => Ok(()),
            Err(e) => check_error(&e).map_err(|m| TestCaseError::fail(format!("{action} {args}: {m}"))),
        }
    });
    if let Err(e) = result {
        panic!("{e}");
    }
    assert_eq!(rt.block_on(snapshot(&db)), before, "read queries changed the table");
}

#[test]
fn string_equals_matches_exactly() {
    let rt = runtime();
    let db = rt.block_on(seeded());
    let before = rt.block_on(snapshot(&db));
    let mut runner = TestRunner::new(config(cases()));
    let result = runner.run(&(hostile_string(), any::<bool>()), |(s, long_form)| {
        let filter = if long_form { json!({ "email": { "equals": s } }) } else { json!({ "email": s }) };
        let rows = rt
            .block_on(q(&db, "findMany", json!({ "where": filter })))
            .map_err(|e| TestCaseError::fail(format!("{s:?}: {e}")))?;
        let rows = rows.as_array().unwrap();
        let expected = SEED_EMAILS.iter().filter(|e| **e == s).count();
        prop_assert_eq!(rows.len(), expected, "email = {:?}", s);
        for row in rows {
            prop_assert_eq!(row["email"].as_str(), Some(s.as_str()));
        }
        let n = rt.block_on(q(&db, "count", json!({ "where": filter }))).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(n.as_u64(), Some(expected as u64));
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}");
    }
    assert_eq!(rt.block_on(snapshot(&db)), before, "read queries changed the table");
}

/// Fixed injection-shaped and unknown-key filters, so a regression shows up without the generator.
#[test]
fn known_hostile_filters() {
    let rt = runtime();
    rt.block_on(async {
        let db = seeded().await;
        let before = snapshot(&db).await;
        for v in ["'", "\"", ";", "--", "/*", "x'; DROP TABLE accounts; --", "' OR '1'='1"] {
            for f in [
                json!({ "email": v }),
                json!({ "email": { "contains": v } }),
                json!({ "email": { "startsWith": v, "endsWith": v } }),
                json!({ "status": { "in": [v, "PENDING"] } }),
                json!({ "status": { "notIn": [v] } }),
                json!({ "NOT": { "OR": [{ "email": v }, { "status": { "not": v } }] } }),
            ] {
                if let Err(e) = q(&db, "findMany", json!({ "where": f })).await {
                    check_error(&e).unwrap_or_else(|m| panic!("{f}: {m}"));
                }
            }
        }
        let e = q(&db, "findMany", json!({ "where": { "nope": 1 } })).await.unwrap_err();
        assert!(e.is_validation(), "{e}");
        let e = q(&db, "findMany", json!({ "where": { "email": { "like": "%" } } })).await.unwrap_err();
        assert!(e.is_validation(), "{e}");
        assert_eq!(snapshot(&db).await, before);
    });
}

/// Failing seeds go to `proptest-regressions/fuzz_filters.txt` (committed) and are replayed on every run.
fn config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct("proptest-regressions/fuzz_filters.txt"))),
        ..ProptestConfig::default()
    }
}
