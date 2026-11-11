//! Statement counts and shapes of engine-built key lists (`tests/parameter_limits.rs` checks the results with lists
//! past the parameter limit). SQLite: at most 500 `OR`ed groups per statement, single-field lists past that bound
//! as one parameter.

use serde_json::{Value, json};

use super::recorder::{Recorded, capture};
use crate::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "items")]
struct Item {
    #[id]
    #[default(autoincrement)]
    id: i64,
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "holders")]
struct Holder {
    #[id]
    #[default(autoincrement)]
    id: i64,
    #[relation(has_many)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pairs: Option<Vec<Pair>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "pairs", id(fields = ["a", "b"]))]
struct Pair {
    a: String,
    b: i64,
    holder_id: Option<i64>,
    #[relation(belongs_to, fields = "holder_id")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    holder: Option<Box<Holder>>,
    #[relation(has_many)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entries: Option<Vec<Entry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "entries")]
struct Entry {
    #[id]
    #[default(autoincrement)]
    id: i64,
    pair_a: String,
    pair_b: i64,
    #[relation(belongs_to, fields = ["pair_a", "pair_b"], references = ["a", "b"])]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pair: Option<Box<Pair>>,
}

const N: usize = 1_200;

async fn db() -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<(Item, Holder, Pair, Entry)>().await.unwrap();
    db
}

async fn seed(db: &OrmClient, table: &str, cols: &str, exprs: &str) {
    let sql = format!(
        "WITH RECURSIVE s(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM s WHERE x < {N}) INSERT INTO \"{table}\" ({cols}) SELECT {exprs} FROM s"
    );
    db.db_execute(&sql).await.unwrap();
}

async fn run(db: &OrmClient, model: &'static crate::ModelMeta, action: &str, args: Value) -> (Value, Vec<Recorded>) {
    let (res, sent) = capture(db.query_json(model, action, args)).await;
    (res.unwrap(), sent)
}

fn starting<'a>(sent: &'a [Recorded], prefix: &str) -> Vec<&'a Recorded> {
    sent.iter().filter(|s| s.sql.starts_with(prefix)).collect()
}

#[tokio::test]
async fn a_long_read_back_is_one_statement_with_one_parameter() {
    let db = db().await;
    let data: Vec<Value> = (0..N).map(|i| json!({ "name": format!("n{i}") })).collect();
    let (rows, sent) = run(&db, Item::meta(), "createManyAndReturn", json!({ "data": data })).await;
    assert_eq!(rows.as_array().unwrap().len(), N);
    let reads = starting(&sent, "SELECT");
    assert_eq!(reads.len(), 1, "{reads:?}");
    assert_eq!(reads[0].params, ["Text"], "{}", reads[0].sql);
    assert!(reads[0].sql.contains("json_each(?)"), "{}", reads[0].sql);
    // A short list keeps the shape it always had.
    let (_, sent) = run(&db, Item::meta(), "createManyAndReturn", json!({ "data": [{ "name": "x" }, { "name": "y" }] })).await;
    let reads = starting(&sent, "SELECT");
    assert!(reads.len() == 1 && reads[0].sql.contains(" OR ") && reads[0].params.len() == 2, "{reads:?}");
}

#[tokio::test]
async fn includes_and_counts_over_a_compound_key_are_split_into_statements() {
    let db = db().await;
    seed(&db, "pairs", "\"a\", \"b\"", "'p' || x, x").await;
    seed(&db, "entries", "\"pair_a\", \"pair_b\"", "'p' || x, x").await;
    let args = json!({ "include": { "entries": true, "_count": { "select": { "entries": true } } } });
    let (pairs, sent) = run(&db, Pair::meta(), "findMany", args).await;
    let pairs = pairs.as_array().unwrap();
    assert_eq!(pairs.len(), N);
    assert!(pairs.iter().all(|p| p["entries"][0]["pair_b"] == p["b"] && p["_count"]["entries"] == 1));
    let children: Vec<usize> =
        starting(&sent, "SELECT").iter().filter(|s| s.sql.contains("FROM \"entries\"")).map(|s| s.params.len()).collect();
    // Three statements for the entries (500 + 500 + 200 keys of two fields) and three for the counts.
    assert_eq!(children, [1000, 1000, 400, 1000, 1000, 400]);
}

#[tokio::test]
async fn a_chunked_update_repeats_its_filter_in_each_statement() {
    let db = db().await;
    run(&db, Holder::meta(), "create", json!({ "data": {} })).await;
    seed(&db, "pairs", "\"a\", \"b\"", "'p' || x, x").await;
    let all: Vec<Value> = (1..=N).map(|i| json!({ "a_b": { "a": format!("p{i}"), "b": i } })).collect();
    let (_, sent) = run(&db, Holder::meta(), "update", json!({ "where": { "id": 1 }, "data": { "pairs": { "set": all } } })).await;
    let updates = starting(&sent, "UPDATE \"pairs\"");
    assert_eq!(updates.len(), 3, "{updates:?}");
    let mut keys = 0;
    for u in &updates {
        // `SET "holder_id" = ?` and, after `WHERE`, the `OR` of this statement's own keys.
        let groups = u.sql.matches("t0.\"a\" = ?").count();
        assert_eq!(u.params.len(), 1 + 2 * groups, "{}", u.sql);
        assert!(u.sql.split(" WHERE ").nth(1).is_some_and(|w| w.contains("t0.\"b\" = ?")), "{}", u.sql);
        keys += groups;
    }
    assert_eq!(keys, N);
    let (n, _) = run(&db, Pair::meta(), "count", json!({ "where": { "holder_id": 1 } })).await;
    assert_eq!(n, json!(N));
}

#[tokio::test]
async fn a_long_nested_delete_many_is_split_into_statements() {
    let db = db().await;
    run(&db, Holder::meta(), "create", json!({ "data": {} })).await;
    seed(&db, "pairs", "\"a\", \"b\", \"holder_id\"", "'p' || x, x, 1").await;
    let items: Vec<Value> = (1..=N).map(|i| json!({ "b": i })).collect();
    let (_, sent) = run(&db, Holder::meta(), "update", json!({ "where": { "id": 1 }, "data": { "pairs": { "deleteMany": items } } })).await;
    assert_eq!(starting(&sent, "DELETE").len(), 3);
    let (n, _) = run(&db, Pair::meta(), "count", json!({})).await;
    assert_eq!(n, json!(0));
}

#[tokio::test]
async fn a_long_insensitive_in_is_one_parameter() {
    let db = db().await;
    seed(&db, "items", "\"name\"", "'Name' || x").await;
    let names: Vec<String> = (1..=N).map(|i| format!("nAME{i}")).collect();
    let (n, sent) = run(&db, Item::meta(), "count", json!({ "where": { "name": { "in": names, "mode": "insensitive" } } })).await;
    assert_eq!(n, json!(N));
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].params, ["Text"]);
    assert!(sent[0].sql.contains("LOWER(t0.\"name\") IN (SELECT LOWER(value) FROM json_each(?))"), "{}", sent[0].sql);
}
