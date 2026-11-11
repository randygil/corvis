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
    pub score: f64,
    pub big: i64,
    pub meta: Option<Value>,
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
    pub extra: Option<Value>,
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
        let data = json!({ "owner": owner, "reset_token": tok, "balance": bal, "visits": 1, "score": 1.5, "big": 1, "notes": { "create": [{ "text": "n" }] } });
        q(db, "create", json!({ "data": data })).await.unwrap();
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
    let data = json!({ "owner": "d", "reset_token": "t", "balance": 1, "visits": 3_000_000_000i64, "score": 1, "big": 1 });
    let e = q(&db, "create", json!({ "data": data })).await;
    assert!(e.as_ref().is_err_and(|e| e.message().contains("INT4")), "{e:?}");
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
        let upsert = json!({ "where": { "id": 1 }, "create": { "owner": "x", "reset_token": "x", "balance": 0, "visits": 0, "score": 0, "big": 0 }, "update": data });
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

// =============================================================================================
// Round 2
// =============================================================================================

/// The secret parts of the markers: the `$type` tags of the Json nulls and the text of `undefined()` without its NULs.
fn marker_texts() -> Vec<String> {
    let mut out: Vec<String> = [corvis_orm::db_null(), corvis_orm::json_null(), corvis_orm::any_null()]
        .iter()
        .map(|m| m["$type"].as_str().unwrap().to_string())
        .collect();
    out.push(corvis_orm::undefined().as_str().unwrap().trim_matches('\0').to_string());
    out
}

fn assert_no_marker(text: &str, what: &str) {
    for m in marker_texts() {
        assert!(!text.contains(&m), "{what}: the marker text leaked: {text}");
    }
}

fn assert_null_marker_refused(res: corvis_orm::Result<Value>, what: &str) {
    match res {
        Err(e) if e.is_validation() && e.message().contains("whole value") => assert_no_marker(e.message(), what),
        other => panic!("{what}: expected the whole-value validation error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Finding A: a null marker is only ever the whole value of a Json field (or of `equals` / `not`)
// ---------------------------------------------------------------------------------------------

async fn null_markers_are_whole_values_only(db: &OrmClient) {
    let (dn, jn, an) = (corvis_orm::db_null(), corvis_orm::json_null(), corvis_orm::any_null());
    let refused = [
        ("update, object", "update", json!({ "where": { "id": 1 }, "data": { "meta": { "deletedAt": jn.clone() } } })),
        ("update, list", "update", json!({ "where": { "id": 1 }, "data": { "meta": [an.clone()] } })),
        ("update, deep", "updateMany", json!({ "where": {}, "data": { "meta": { "a": [{ "b": dn.clone() }] } } })),
        ("update, set", "update", json!({ "where": { "id": 1 }, "data": { "meta": { "set": { "a": dn.clone() } } } })),
        (
            "create",
            "create",
            json!({ "data": { "owner": "e", "reset_token": "e", "balance": 1, "visits": 1, "score": 1, "big": 1, "meta": { "a": jn.clone() } } }),
        ),
        (
            "createMany",
            "createMany",
            json!({ "data": [{ "owner": "e", "reset_token": "e", "balance": 1, "visits": 1, "score": 1, "big": 1, "meta": [dn.clone()] }] }),
        ),
        (
            "nested create",
            "update",
            json!({ "where": { "id": 1 }, "data": { "notes": { "create": [{ "text": "x", "extra": { "a": dn.clone() } }] } } }),
        ),
        ("filter equals, nested", "findMany", json!({ "where": { "meta": { "equals": { "a": an.clone() } } } })),
        ("filter not, nested", "findMany", json!({ "where": { "meta": { "not": [jn.clone()] } } })),
        ("filter path equals, nested", "findMany", json!({ "where": { "meta": { "path": ["a"], "equals": { "b": an.clone() } } } })),
        ("filter array_contains", "findMany", json!({ "where": { "meta": { "array_contains": [jn.clone()] } } })),
        ("filter string_contains", "findMany", json!({ "where": { "meta": { "path": ["a"], "string_contains": dn.clone() } } })),
        ("relation filter", "findMany", json!({ "where": { "notes": { "some": { "extra": { "equals": { "a": dn.clone() } } } } } })),
    ];
    for (what, action, args) in refused {
        assert_null_marker_refused(q(db, action, args).await, what);
    }
    // As the whole value they keep working.
    q(db, "update", json!({ "where": { "id": 1 }, "data": { "meta": jn.clone() } })).await.unwrap();
    q(db, "update", json!({ "where": { "id": 2 }, "data": { "meta": { "a": 1 } } })).await.unwrap();
    let r = q(db, "findMany", json!({ "where": { "meta": { "equals": jn.clone() } }, "select": { "id": true } })).await.unwrap();
    assert_eq!(r, json!([{ "id": 1 }]));
    let r = q(db, "count", json!({ "where": { "meta": { "equals": an.clone() } } })).await.unwrap();
    assert_eq!(r, json!(2));
    let r = q(db, "count", json!({ "where": { "meta": { "path": ["a"], "equals": jn.clone() } } })).await.unwrap();
    assert_eq!(r, json!(0));
    q(db, "update", json!({ "where": { "id": 1 }, "data": { "meta": dn.clone() } })).await.unwrap();
    // Nothing stored holds a marker.
    let all = q(db, "findMany", json!({ "include": { "notes": true } })).await.unwrap();
    assert_no_marker(&all.to_string(), "stored rows");
}

#[tokio::test]
async fn null_markers_are_whole_values_only_sqlite() {
    null_markers_are_whole_values_only(&sqlite().await).await;
}

#[tokio::test]
async fn null_markers_are_whole_values_only_postgres() {
    let Some((_drop, db)) = postgres("nulls").await else { return };
    null_markers_are_whole_values_only(&db).await;
}

/// Every input that is quoted back in an error carries a marker here; no error shows its text.
async fn marker_text_never_reaches_an_error(db: &OrmClient) {
    let dn = corvis_orm::db_null();
    let un = corvis_orm::undefined();
    let tag = dn["$type"].as_str().unwrap().to_string();
    let cases = [
        ("create", json!({ "data": { "owner": dn, "reset_token": "t", "balance": 1, "visits": 1, "score": 1, "big": 1 } })),
        ("update", json!({ "where": { "id": 1 }, "data": { "visits": dn } })),
        ("findMany", json!({ "orderBy": { "owner": dn } })),
        ("findMany", json!({ "take": dn })),
        ("findMany", json!({ "select": { "owner": dn } })),
        ("findMany", json!({ "distinct": [dn] })),
        ("findMany", json!({ "cursor": { "id": dn } })),
        ("findMany", json!({ "where": { "owner": { "contains": "a", "mode": dn } } })),
        ("findMany", json!({ "where": { "owner": { "contains": dn } } })),
        ("findMany", json!({ "where": { "meta": { "path": dn, "equals": 1 } } })),
        ("findMany", json!({ "where": { tag.clone(): 1 } })),
        ("findMany", json!({ "orderBy": { tag.clone(): "asc" } })),
        ("groupBy", json!({ "by": [dn] })),
        ("groupBy", json!({ "by": [tag.clone()] })),
        ("groupBy", json!({ "by": ["owner"], "having": { "visits": { "_sum": { "gt": dn } } } })),
        ("findMany", json!({ "where": { "balance": dn } })),
        ("findMany", json!({ "where": un })),
        ("findMany", json!({ "take": un })),
        ("update", json!({ "where": { "id": 1 }, "data": { "visits": { "increment": dn } } })),
        ("findMany", json!({ "wherex": dn })),
        ("findMany", json!({ "where": { "owner": { tag.clone(): 1 } } })),
    ];
    for (action, args) in cases {
        let what = format!("{action} {args}");
        let what = what.replace(&tag, "<tag>");
        match q(db, action, args).await {
            Err(e) => {
                assert_no_marker(e.message(), &what);
                assert_no_marker(&e.to_string(), &what);
                assert_no_marker(&format!("{e:?}"), &what);
            }
            Ok(v) => assert_no_marker(&v.to_string(), &what),
        }
    }
    let p = corvis_orm::WhereParam::<Account>::new("meta", json!({ "equals": corvis_orm::any_null(), "x": corvis_orm::undefined() }));
    assert_no_marker(&format!("{p:?}"), "Debug of a typed parameter");
}

#[tokio::test]
async fn marker_text_never_reaches_an_error_sqlite() {
    marker_text_never_reaches_an_error(&sqlite().await).await;
}

#[tokio::test]
async fn marker_text_never_reaches_an_error_postgres() {
    let Some((_drop, db)) = postgres("echo").await else { return };
    marker_text_never_reaches_an_error(&db).await;
}

// ---------------------------------------------------------------------------------------------
// Finding B: BigInt and Float arithmetic beyond the column on SQLite
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn bigint_and_float_arithmetic_beyond_range_is_rolled_back_sqlite() {
    let db = sqlite().await;
    q(&db, "updateMany", json!({ "where": {}, "data": { "big": i64::MAX, "score": 1.5e308 } })).await.unwrap();
    for (data, message) in [
        (json!({ "big": { "increment": 1 } }), "bigint out of range"),
        (json!({ "big": { "multiply": 2 } }), "bigint out of range"),
        (json!({ "score": { "multiply": 10 } }), "value out of range: overflow"),
        (json!({ "score": { "increment": 1.5e308 } }), "value out of range: overflow"),
    ] {
        let r = q(&db, "updateMany", json!({ "where": {}, "data": data.clone() })).await;
        assert!(r.as_ref().is_err_and(|e| e.message().contains(message)), "{data}: {r:?}");
    }
    let rows = q(&db, "findMany", json!({ "select": { "big": true, "score": true } })).await.unwrap();
    for row in rows.as_array().unwrap() {
        assert!(row["big"] == json!(i64::MAX) || row["big"] == json!(i64::MAX.to_string()), "{row}");
        assert_eq!(row["score"], json!(1.5e308), "{row}");
    }
    // A Float is never written as text (recorded: `fin.ratio_text_infinity`, a validation error).
    let r = q(&db, "update", json!({ "where": { "id": 1 }, "data": { "score": "Infinity" } })).await;
    assert!(r.as_ref().is_err_and(|e| e.is_validation()), "{r:?}");
    db.account().find_many(()).exec().await.unwrap();
}

#[tokio::test]
async fn a_bigint_or_float_column_holding_something_else_is_p2023_sqlite() {
    let db = sqlite().await;
    for (column, value, message) in [
        ("big", "1.5", "Could not convert from `BigDecimal(1.5)` to `BigInt`"),
        ("big", "1e30", "Could not convert from `BigDecimal(1000000000000000000000000000000)` to `BigInt`"),
        ("score", "9e999", "Could not convert value inf of the field `score` to type `Float`."),
        ("score", "-9e999", "Could not convert value -inf of the field `score` to type `Float`."),
    ] {
        db.execute_raw(&format!("UPDATE \"accounts\" SET \"{column}\" = {value} WHERE \"id\" = 1"), vec![]).await.unwrap();
        let e = q(&db, "findMany", json!({ "where": { "id": 1 } })).await.unwrap_err();
        assert_eq!((e.code(), e.message()), (Some("P2023"), format!("Inconsistent column data: {message}").as_str()), "{column} = {value}");
        db.execute_raw(&format!("UPDATE \"accounts\" SET \"{column}\" = 1 WHERE \"id\" = 1"), vec![]).await.unwrap();
    }
}
