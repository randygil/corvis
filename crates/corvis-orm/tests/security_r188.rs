//! Regression tests of the first security review of #188 (`q-orm-correctness`): the `undefined` marker could be
//! forged from a request; Decimal and Int values that SQLite stores but the ORM cannot read back; non-finite Decimals
//! compared as text; reads that saturate.
//!
//! The Postgres tests use `CORVIS_ORM_PG`, databases dropped at the end of each test and a statement timeout.

#[path = "common/pg.rs"]
mod pg;

use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "accounts")]
pub struct Account {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    pub owner: String,
    pub reset_token: String,
    pub balance: Decimal,
    pub visits: i32,
    #[relation(has_many)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<Note>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "notes")]
pub struct Note {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    pub text: String,
    pub account_id: i64,
    #[relation(belongs_to, fields = "account_id")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Box<Account>>,
}

fn meta() -> &'static corvis_orm::ModelMeta {
    <Account as corvis_orm::Model>::meta()
}

async fn q(db: &OrmClient, action: &str, args: Value) -> corvis_orm::Result<Value> {
    db.query_json(meta(), action, args).await
}

async fn seed(db: &OrmClient) {
    db.db_push::<(Account, Note)>().await.unwrap();
    for (owner, tok, bal) in [("alice", "tok-alice", "100"), ("bob", "tok-bob", "250.75"), ("carol", "tok-carol", "-5")] {
        q(db, "create", json!({ "data": { "owner": owner, "reset_token": tok, "balance": bal, "visits": 1, "notes": { "create": [{ "text": "n" }] } } }))
            .await
            .unwrap();
    }
}

async fn sqlite() -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    seed(&db).await;
    db
}

async fn postgres(name: &str) -> Option<(pg::DropOnExit, OrmClient)> {
    let (url, guard) = pg::fresh_db_bounded(&format!("r188_{name}"), 10).await?;
    let db = OrmClient::new(url).unwrap();
    seed(&db).await;
    Some((guard, db))
}

async fn count(db: &OrmClient) -> i64 {
    q(db, "count", json!({})).await.unwrap().as_i64().unwrap()
}

fn assert_nul_error(res: corvis_orm::Result<Value>, what: &str) {
    match res {
        Err(e) if e.is_validation() && e.message().contains("NUL character") => {}
        other => panic!("{what}: expected the NUL validation error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Finding 1: the text of the `undefined` marker arriving from a request is a NUL string, never "undefined"
// ---------------------------------------------------------------------------------------------

/// What a client can send: the marker's former text, `"\u0000corvis_orm::undefined\u0000"` in JSON.
fn forged() -> String {
    let body: Value = serde_json::from_str(r#"{ "token": "\u0000corvis_orm::undefined\u0000" }"#).unwrap();
    body["token"].as_str().unwrap().to_string()
}

async fn forged_marker_is_a_nul_string(db: &OrmClient) {
    let t = forged();
    assert_nul_error(q(db, "findFirst", json!({ "where": { "reset_token": t } })).await, "findFirst");
    assert_nul_error(q(db, "findMany", json!({ "where": { "reset_token": { "equals": t } } })).await, "equals");
    let typed = db.account().find_first(vec![account::reset_token::equals(t.clone())]).exec_json().await;
    assert_nul_error(typed, "typed equals");
    let r = q(db, "updateMany", json!({ "where": { "owner": t }, "data": { "visits": 99 } })).await;
    assert_nul_error(r, "updateMany");
    assert_nul_error(q(db, "deleteMany", json!({ "where": { "owner": "alice", "reset_token": t } })).await, "deleteMany");
    assert_nul_error(q(db, "deleteMany", json!({ "where": { "owner": t } })).await, "deleteMany, one condition");
    let nested = json!({ "where": { "notes": { "some": { "text": t } } } });
    assert_nul_error(q(db, "findMany", nested).await, "relation filter");
    let nested_write = json!({ "where": { "id": 1 }, "data": { "notes": { "deleteMany": { "text": t } } } });
    assert_nul_error(q(db, "update", nested_write).await, "nested deleteMany");
    assert_eq!(count(db).await, 3, "rows were deleted");
    let visits = q(db, "findMany", json!({ "where": { "visits": 99 } })).await.unwrap();
    assert_eq!(visits, json!([]), "rows were updated");
}

#[tokio::test]
async fn a_forged_undefined_marker_is_refused_sqlite() {
    forged_marker_is_a_nul_string(&sqlite().await).await;
}

#[tokio::test]
async fn a_forged_undefined_marker_is_refused_postgres() {
    let Some((_drop, db)) = postgres("marker").await else { return };
    forged_marker_is_a_nul_string(&db).await;
}

#[tokio::test]
async fn undefined_still_drops_the_key() {
    let db = sqlite().await;
    let r = q(&db, "findMany", json!({ "where": { "owner": corvis_orm::undefined(), "reset_token": "tok-bob" } })).await.unwrap();
    assert_eq!(r.as_array().unwrap().len(), 1);
    let r = q(&db, "count", json!({ "where": { "owner": corvis_orm::or_undefined(None::<String>) } })).await.unwrap();
    assert_eq!(r, json!(3));
}

// ---------------------------------------------------------------------------------------------
// Finding 4: an Int beyond 32 bits is never written on SQLite (it could not be read back)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_int_beyond_32_bits_is_not_written_sqlite() {
    let db = sqlite().await;
    let e = q(&db, "create", json!({ "data": { "owner": "d", "reset_token": "t", "balance": 1, "visits": 3_000_000_000i64 } })).await;
    assert!(e.is_err(), "{e:?}");
    let e = q(&db, "updateMany", json!({ "where": { "id": 1 }, "data": { "visits": { "set": -3_000_000_000i64 } } })).await;
    assert!(e.is_err(), "{e:?}");
    assert_eq!(count(&db).await, 3, "the row was stored");
    // Every list still reads.
    q(&db, "findMany", json!({})).await.unwrap();
    // Filters keep 64 bits, like the reference client (recorded in the SQLite oracle, `err.int_overflow`).
    let r = q(&db, "findMany", json!({ "where": { "visits": 3_000_000_000i64 } })).await.unwrap();
    assert_eq!(r, json!([]));
}

// ---------------------------------------------------------------------------------------------
// Finding 5: arithmetic results beyond the column are refused and rolled back on SQLite
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn decimal_arithmetic_beyond_the_column_is_rolled_back_sqlite() {
    let db = sqlite().await;
    let r = q(&db, "update", json!({ "where": { "id": 2 }, "data": { "balance": { "increment": "1e200" } } })).await;
    assert!(r.is_err(), "{r:?}");
    let r = q(&db, "updateMany", json!({ "where": {}, "data": { "balance": { "multiply": "1e40" } } })).await;
    assert!(r.is_err(), "{r:?}");
    let balances = q(&db, "findMany", json!({ "select": { "balance": true }, "orderBy": { "id": "asc" } })).await.unwrap();
    assert_eq!(balances, json!([{ "balance": "100" }, { "balance": "250.75" }, { "balance": "-5" }]), "a row changed");
    // Within the bound it works.
    let r = q(&db, "update", json!({ "where": { "id": 2 }, "data": { "balance": { "increment": "1" } } })).await.unwrap();
    assert_eq!(r["balance"], "251.75");
    db.account().find_many(()).exec().await.unwrap();
}

#[tokio::test]
async fn int_arithmetic_beyond_32_bits_is_rolled_back_sqlite() {
    let db = sqlite().await;
    for data in [json!({ "visits": { "increment": 2_147_483_647i64 } }), json!({ "visits": { "multiply": 3_000_000_000i64 } })] {
        let r = q(&db, "updateMany", json!({ "where": {}, "data": data.clone() })).await;
        assert!(r.is_err(), "{data}: {r:?}");
        let upsert =
            json!({ "where": { "id": 1 }, "create": { "owner": "x", "reset_token": "x", "balance": 0, "visits": 0 }, "update": data });
        let r = q(&db, "upsert", upsert).await;
        assert!(r.is_err(), "upsert: {r:?}");
    }
    let visits = q(&db, "findMany", json!({ "select": { "visits": true } })).await.unwrap();
    assert_eq!(visits, json!([{ "visits": 1 }, { "visits": 1 }, { "visits": 1 }]));
}

// ---------------------------------------------------------------------------------------------
// Finding 6: non-finite Decimals are refused on SQLite
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn non_finite_decimals_are_refused_sqlite() {
    let db = sqlite().await;
    for (op, v) in [("lt", "-Infinity"), ("lt", "NaN"), ("not", "NaN"), ("equals", "Infinity"), ("in", "NaN")] {
        let v = if op == "in" { json!([v]) } else { json!(v) };
        let r = q(&db, "findMany", json!({ "where": { "balance": { op: v.clone() } } })).await;
        assert!(r.as_ref().is_err_and(|e| e.is_validation()), "{op} {v}: {r:?}");
    }
    let r = q(&db, "updateMany", json!({ "where": { "id": 1 }, "data": { "balance": "Infinity" } })).await;
    assert!(r.as_ref().is_err_and(|e| e.is_validation()), "{r:?}");
    let r = q(&db, "findMany", json!({ "where": { "id": 1 }, "select": { "balance": true } })).await.unwrap();
    assert_eq!(r, json!([{ "balance": "100" }]));
}

// ---------------------------------------------------------------------------------------------
// Finding 7: an Int column that holds something else is P2023, never a saturated or truncated number
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_int_column_holding_a_bigger_or_fractional_number_is_p2023_sqlite() {
    let db = sqlite().await;
    for value in ["9223372036854775807", "1.5", "1e30"] {
        db.execute_raw(&format!("UPDATE \"accounts\" SET \"visits\" = {value} WHERE \"id\" = 1"), vec![]).await.unwrap();
        let e = q(&db, "findMany", json!({ "where": { "id": 1 } })).await.unwrap_err();
        assert_eq!(e.code(), Some("P2023"), "{value}: {e}");
    }
}
