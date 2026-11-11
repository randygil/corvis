//! `Decimal` columns on SQLite compare, sort and aggregate as numbers, on every table layout an app can have:
//! a table created by `db push`, one created with a `DECIMAL` column (the original client's migration), and one
//! with the `TEXT` column that `db push` created before corvis-orm declared `DECIMAL` (earlier pushed databases).

use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "amounts")]
pub struct Amount {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    pub label: String,
    pub amount: Decimal,
}

fn meta() -> &'static corvis_orm::ModelMeta {
    <Amount as corvis_orm::Model>::meta()
}

async fn q(db: &OrmClient, action: &str, args: Value) -> Value {
    db.query_json(meta(), action, args).await.unwrap()
}

/// `99.5` and `100` labelled `a`, `1000` labelled `b`, `-5` labelled `c`.
async fn seed(db: &OrmClient) {
    for (label, amount) in [("a", "99.5"), ("a", "100"), ("b", "1000"), ("c", "-5")] {
        q(db, "create", json!({ "data": { "label": label, "amount": amount } })).await;
    }
}

async fn pushed() -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<Amount>().await.unwrap();
    seed(&db).await;
    db
}

async fn created_with(column_type: &str) -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_execute(&format!(
        "CREATE TABLE \"amounts\" (\"id\" INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT, \"label\" TEXT NOT NULL, \"amount\" {column_type} NOT NULL)"
    ))
    .await
    .unwrap();
    seed(&db).await;
    db
}

fn amounts(rows: &Value) -> Vec<String> {
    rows.as_array().unwrap().iter().map(|r| r["amount"].as_str().unwrap().to_string()).collect()
}

async fn find(db: &OrmClient, args: Value) -> Vec<String> {
    amounts(&q(db, "findMany", args).await)
}

/// Every comparison, ordering and aggregate of the spec, on one database.
async fn check(db: &OrmClient) {
    let by_id = json!({ "id": "asc" });
    assert_eq!(find(db, json!({ "where": { "amount": { "gte": 100 } }, "orderBy": by_id })).await, ["100", "1000"], "gte 100");
    assert_eq!(find(db, json!({ "where": { "amount": { "gte": "100" } }, "orderBy": by_id })).await, ["100", "1000"], "gte \"100\"");
    assert_eq!(find(db, json!({ "where": { "amount": { "lt": 0 } }, "orderBy": by_id })).await, ["-5"], "lt 0");
    assert_eq!(find(db, json!({ "where": { "amount": { "gt": "99.5" } }, "orderBy": by_id })).await, ["100", "1000"], "gt 99.5");
    assert_eq!(find(db, json!({ "where": { "amount": { "in": ["100.0", 1000] } }, "orderBy": by_id })).await, ["100", "1000"], "in");
    assert_eq!(find(db, json!({ "where": { "amount": { "notIn": ["100.0", 1000] } }, "orderBy": by_id })).await, ["99.5", "-5"], "notIn");
    assert_eq!(find(db, json!({ "where": { "amount": "1e3" } })).await, ["1000"], "equals 1e3");
    assert_eq!(find(db, json!({ "orderBy": { "amount": "asc" } })).await, ["-5", "99.5", "100", "1000"], "orderBy asc");
    assert_eq!(find(db, json!({ "orderBy": { "amount": "desc" } })).await, ["1000", "100", "99.5", "-5"], "orderBy desc");
    // The cursor condition compares in the same (numeric) order.
    let page = find(db, json!({ "orderBy": { "amount": "asc" }, "cursor": { "id": 1 }, "take": 2 })).await;
    assert_eq!(page, ["99.5", "100"], "cursor");

    let agg = q(db, "aggregate", json!({ "_min": { "amount": true }, "_max": { "amount": true }, "_sum": { "amount": true } })).await;
    assert_eq!(agg, json!({ "_min": { "amount": "-5" }, "_max": { "amount": "1000" }, "_sum": { "amount": "1194.5" } }), "aggregate");

    let groups = |having: Value| {
        let db = db.clone();
        async move {
            let rows =
                q(&db, "groupBy", json!({ "by": ["label"], "having": having, "orderBy": { "label": "asc" }, "_max": { "amount": true } }))
                    .await;
            rows.as_array().unwrap().iter().map(|r| r["label"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        }
    };
    assert_eq!(groups(json!({ "amount": { "_sum": { "gt": 150 } } })).await, ["a", "b"], "having _sum");
    assert_eq!(groups(json!({ "amount": { "_max": { "gt": "500" } } })).await, ["b"], "having _max");
    assert_eq!(groups(json!({ "amount": { "_min": { "lt": 0 } } })).await, ["c"], "having _min");
    let max = q(db, "groupBy", json!({ "by": ["label"], "_max": { "amount": true }, "orderBy": { "_max": { "amount": "desc" } } })).await;
    let labels: Vec<&str> = max.as_array().unwrap().iter().map(|r| r["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["b", "a", "c"], "groupBy ordered by _max");

    // Arithmetic on the column reads it as a number too.
    q(db, "updateMany", json!({ "where": { "label": "c" }, "data": { "amount": { "increment": "0.5" } } })).await;
    assert_eq!(find(db, json!({ "where": { "label": "c" } })).await, ["-4.5"], "increment");
}

#[tokio::test]
async fn decimal_compares_as_a_number_on_a_pushed_table() {
    check(&pushed().await).await;
}

#[tokio::test]
async fn decimal_compares_as_a_number_on_a_decimal_column() {
    check(&created_with("DECIMAL").await).await;
}

#[tokio::test]
async fn decimal_compares_as_a_number_on_a_text_column_of_an_earlier_push() {
    check(&created_with("TEXT").await).await;
}

#[tokio::test]
async fn db_push_declares_decimal_columns_as_decimal() {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    let ddl = db.schema_sql::<Amount>().unwrap().join("\n");
    assert!(ddl.contains("\"amount\" DECIMAL NOT NULL"), "{ddl}");
}
