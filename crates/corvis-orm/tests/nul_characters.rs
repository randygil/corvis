//! A NUL character (U+0000) in a `where` is a validation error on every dialect, before any SQL runs.
//!
//! PostgreSQL cannot hold NUL in `text` or `jsonb` and used to fail these queries with a raw
//! database error (`22021` / `22P05`); SQLite cut JSON paths short at the NUL (`bad JSON path`).
//! The Postgres test uses `CORVIS_ORM_PG` and the `orm_test_nul_characters` database (skipped when
//! the server is not reachable).

#[path = "common/pg.rs"]
mod pg;

use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "accounts")]
pub struct Account {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    pub email: String,
    pub data: Option<serde_json::Value>,
}

fn meta() -> &'static corvis_orm::ModelMeta {
    <Account as corvis_orm::Model>::meta()
}

#[allow(clippy::result_large_err)]
async fn q(db: &OrmClient, action: &str, args: Value) -> corvis_orm::Result<Value> {
    db.query_json(meta(), action, args).await
}

async fn seeded(url: &str) -> OrmClient {
    let db = OrmClient::new(url).unwrap();
    db.db_push::<(Account,)>().await.unwrap();
    q(&db, "create", json!({ "data": { "email": "a@x", "data": { "k": "v" } } })).await.unwrap();
    db
}

/// Every place a NUL can sit in a `where`.
fn nul_filters() -> Vec<(&'static str, Value)> {
    let n = "a\u{0}b";
    vec![
        ("equals shorthand", json!({ "email": n })),
        ("equals", json!({ "email": { "equals": n } })),
        ("not", json!({ "email": { "not": n } })),
        ("contains", json!({ "email": { "contains": n } })),
        ("startsWith", json!({ "email": { "startsWith": "\u{0}" } })),
        ("endsWith", json!({ "email": { "endsWith": "\u{0}" } })),
        ("in", json!({ "email": { "in": ["x", n] } })),
        ("notIn", json!({ "email": { "notIn": [n] } })),
        ("lt", json!({ "email": { "lt": n } })),
        ("json path segment", json!({ "data": { "path": [n], "equals": 1 } })),
        ("json path string form", json!({ "data": { "path": "$.a\u{0}b", "equals": 1 } })),
        ("json equals string", json!({ "data": { "path": ["k"], "equals": n } })),
        ("json equals nested", json!({ "data": { "equals": { "k": n } } })),
        ("json key", json!({ "data": { "equals": { n: 1 } } })),
        ("json string_contains", json!({ "data": { "path": ["k"], "string_contains": n } })),
        ("json array_contains", json!({ "data": { "path": ["k"], "array_contains": [n] } })),
        ("AND", json!({ "AND": [{ "email": "a@x" }, { "email": n }] })),
        ("OR", json!({ "OR": [{ "email": n }] })),
        ("NOT", json!({ "NOT": { "email": n } })),
    ]
}

async fn check_all_rejected(db: &OrmClient, dialect: &str) {
    for (name, w) in nul_filters() {
        for action in ["findMany", "count", "findFirst", "deleteMany", "updateMany"] {
            let args = match action {
                "updateMany" => json!({ "where": w, "data": { "email": "z@x" } }),
                _ => json!({ "where": w }),
            };
            let e = q(db, action, args).await.unwrap_err();
            assert!(e.is_validation(), "{dialect} {name} {action}: expected a validation error, got {e}");
            assert!(e.message().contains("NUL character"), "{dialect} {name} {action}: {e}");
        }
    }
    // Nothing ran: the row is intact and an ordinary filter still works.
    let rows = q(db, "findMany", json!({ "where": { "email": "a@x", "data": { "path": ["k"], "equals": "v" } } })).await.unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1, "{dialect}");
    assert_eq!(q(db, "count", json!({})).await.unwrap(), json!(1), "{dialect}");
}

#[tokio::test]
async fn nul_in_a_filter_is_a_validation_error_on_sqlite() {
    let db = seeded("sqlite::memory:").await;
    check_all_rejected(&db, "sqlite").await;
}

#[tokio::test]
async fn nul_in_a_filter_is_a_validation_error_on_postgres() {
    let Some(url) = pg::fresh_db("nul_characters").await else { return };
    let db = seeded(&url).await;
    check_all_rejected(&db, "postgres").await;
}

/// The check runs before any SQL is built, so a client that never connects reports it too.
#[test]
fn nul_is_rejected_when_compiling_sql() {
    for url in ["postgres://u:p@127.0.0.1:5299/db", "sqlite::memory:"] {
        let db = OrmClient::new(url).unwrap();
        for (name, w) in nul_filters() {
            let e = corvis_orm::debug_sql(&db, meta(), &w).unwrap_err();
            assert!(e.is_validation(), "{url} {name}: {e}");
        }
        // Other control and non-ASCII characters are still ordinary values.
        let w = json!({ "data": { "path": ["a\u{e9}\u{1}"], "equals": "\u{1}" }, "email": { "contains": "\u{2028}" } });
        assert!(corvis_orm::debug_sql(&db, meta(), &w).is_ok());
    }
}
