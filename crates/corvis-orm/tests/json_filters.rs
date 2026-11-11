//! Regression tests of JSON `path` filters on SQLite: `equals`, `not` and ordering compare values of
//! the same JSON type (they used to compare `json_extract` output with quoted JSON text, so `equals`
//! never matched and `not` / `lt` matched every row).

use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "accounts")]
pub struct Account {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    #[unique]
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

async fn seeded() -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<(Account,)>().await.unwrap();
    let docs = [
        json!({ "n": 0, "role": "admin", "ok": true, "tags": ["a", "b"] }),
        json!({ "n": 1, "role": "user", "ok": false, "tags": ["b"] }),
        json!({ "n": 2.5, "role": "5", "ok": null, "tags": [] }),
        json!({ "n": "1", "role": "user" }),
        json!({ "m": 1 }),
    ];
    for (i, d) in docs.iter().enumerate() {
        q(&db, "create", json!({ "data": { "email": format!("u{i}@x"), "data": d } })).await.unwrap();
    }
    db
}

async fn ids(db: &OrmClient, filter: Value) -> Vec<i64> {
    let rows = q(db, "findMany", json!({ "where": filter, "orderBy": { "id": "asc" } })).await.unwrap();
    rows.as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect()
}

fn path(p: &[&str], op: &str, v: Value) -> Value {
    json!({ "data": { "path": p, op: v } })
}

#[tokio::test]
async fn equals_matches_numbers_strings_and_booleans() {
    let db = seeded().await;
    assert_eq!(ids(&db, path(&["n"], "equals", json!(1))).await, [2]);
    assert_eq!(ids(&db, path(&["n"], "equals", json!(2.5))).await, [3]);
    assert_eq!(ids(&db, path(&["n"], "equals", json!("1"))).await, [4]);
    assert_eq!(ids(&db, path(&["role"], "equals", json!("admin"))).await, [1]);
    assert_eq!(ids(&db, path(&["role"], "equals", json!("5"))).await, [3]);
    assert_eq!(ids(&db, path(&["role"], "equals", json!(5))).await, Vec::<i64>::new());
    // `true` is not `1`.
    assert_eq!(ids(&db, path(&["ok"], "equals", json!(true))).await, [1]);
    assert_eq!(ids(&db, path(&["ok"], "equals", json!(1))).await, Vec::<i64>::new());
    assert_eq!(ids(&db, path(&["tags", "0"], "equals", json!("b"))).await, [2]);
}

#[tokio::test]
async fn not_excludes_equal_values_and_missing_paths_only() {
    let db = seeded().await;
    // Rows without the path are neither equal nor different (SQL NULL).
    assert_eq!(ids(&db, path(&["role"], "not", json!("admin"))).await, [2, 3, 4]);
    assert_eq!(ids(&db, path(&["n"], "not", json!(1))).await, [1, 3, 4]);
    assert_eq!(ids(&db, json!({ "NOT": path(&["role"], "equals", json!("admin")) })).await, [2, 3, 4]);
}

#[tokio::test]
async fn ordering_compares_values_of_the_same_type() {
    let db = seeded().await;
    assert_eq!(ids(&db, path(&["n"], "lt", json!(-1000))).await, Vec::<i64>::new());
    assert_eq!(ids(&db, path(&["n"], "lt", json!(1))).await, [1]);
    assert_eq!(ids(&db, path(&["n"], "lte", json!(1))).await, [1, 2]);
    assert_eq!(ids(&db, path(&["n"], "gt", json!(1))).await, [3]);
    assert_eq!(ids(&db, path(&["n"], "gte", json!(1))).await, [2, 3]);
    // A string is never ordered against a number.
    assert_eq!(ids(&db, path(&["n"], "gt", json!("0"))).await, [4]);
    assert_eq!(ids(&db, path(&["n"], "gt", json!(100))).await, Vec::<i64>::new());
}

#[tokio::test]
async fn string_and_array_filters_check_the_json_type() {
    let db = seeded().await;
    // `n: 2.5` and `n: 1` hold numbers, not strings.
    assert_eq!(ids(&db, path(&["n"], "string_contains", json!("1"))).await, [4]);
    assert_eq!(ids(&db, path(&["role"], "string_starts_with", json!("ad"))).await, [1]);
    assert_eq!(ids(&db, path(&["tags"], "array_contains", json!("b"))).await, [1, 2]);
    assert_eq!(ids(&db, path(&["tags"], "array_contains", json!(["a", "b"]))).await, [1]);
    assert_eq!(ids(&db, path(&["tags"], "array_contains", json!([]))).await, [1, 2, 3]);
    // A scalar is not a one-element array, and `"1"` is not `1`.
    assert_eq!(ids(&db, path(&["role"], "array_contains", json!("admin"))).await, Vec::<i64>::new());
    assert_eq!(ids(&db, path(&["tags"], "array_contains", json!(1))).await, Vec::<i64>::new());
}

#[tokio::test]
async fn json_null_is_distinct_from_a_missing_path() {
    let db = seeded().await;
    let json_null = corvis_orm::json_null();
    assert_eq!(ids(&db, path(&["ok"], "equals", json_null.clone())).await, [3]);
    assert_eq!(ids(&db, path(&["ok"], "not", json_null)).await, [1, 2]);
}
