//! Typed helpers never turn a value into `null`: a value that fails to serialize, or a non-finite float, makes the
//! query fail with a validation error before any statement runs, instead of becoming `IS NULL` / `IS NOT NULL`
//! (or a `NULL` written to the column).

use corvis_orm::prelude::*;
use corvis_orm::serde::ser::Error as _;
use corvis_orm::{SetParam, WhereParam, and, not, or};

/// A value whose `Serialize` always fails (like a map with non-string keys would).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Unserializable(pub f64);

impl Serialize for Unserializable {
    fn serialize<S: corvis_orm::serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(S::Error::custom("this value cannot be serialized"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "readings")]
pub struct Reading {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    #[default("x")]
    pub label: String,
    pub score: Option<f64>,
    #[db_type(Float)]
    pub odd: Option<Unserializable>,
    #[db_type(Float)]
    #[default([])]
    pub odds: Vec<Unserializable>,
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
    pub reading_id: i64,
    #[relation(belongs_to, fields = "reading_id")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading: Option<Box<Reading>>,
}

/// Rows with the scores `1.0`, `2.0` and `null`.
async fn seeded() -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<(Reading, Note)>().await.unwrap();
    for s in [json!(1.0), json!(2.0), Value::Null] {
        db.reading().create(json!({ "data": { "score": s } })).await.unwrap();
    }
    db
}

fn assert_validation<T: std::fmt::Debug>(res: corvis_orm::Result<T>, what: &str) {
    match res {
        Err(e) if e.is_validation() => {}
        other => panic!("{what}: expected a validation error, got {other:?}"),
    }
}

async fn scores(db: &OrmClient) -> Vec<Option<f64>> {
    db.reading().find_many(()).order_by(reading::id::order(SortOrder::Asc)).exec().await.unwrap().into_iter().map(|r| r.score).collect()
}

// ---------------------------------------------------------------------------------------------
// Non-finite floats
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn nan_is_never_a_null_filter() {
    let db = seeded().await;
    for p in [reading::score::equals(f64::NAN), reading::score::equals(f64::INFINITY), reading::score::not(f64::NAN)] {
        let shown = format!("{p:?}");
        let null_filter = p.value().is_null() || p.value() == &json!({ "not": null });
        match db.reading().find_many(vec![p]).exec().await {
            // Value semantics: `NaN` equals no row and every row differs from it; never the `null` row alone.
            Ok(rows) => {
                assert!(!null_filter, "{shown} became a null filter");
                assert!(rows.iter().all(|r| r.score.is_some()), "{shown} matched the null row");
            }
            Err(e) => assert!(e.is_validation(), "{shown}: {e}"),
        }
    }
}

#[tokio::test]
async fn delete_by_not_nan_leaves_the_rows_a_value_comparison_leaves() {
    let db = seeded().await;
    // `score <> NaN` deletes no row on SQLite (a bound NaN is NULL); a refusal deletes nothing either.
    let _ = db.reading().delete_many(vec![reading::score::not(f64::NAN)]).exec().await;
    assert_eq!(scores(&db).await, [Some(1.0), Some(2.0), None]);
}

#[tokio::test]
async fn update_by_nan_changes_no_row() {
    let db = seeded().await;
    let _ = db.reading().update_many(vec![reading::score::equals(f64::NAN)]).data(vec![reading::label::set("changed")]).exec().await;
    let labels: Vec<String> = db.reading().find_many(()).exec().await.unwrap().into_iter().map(|r| r.label).collect();
    assert_eq!(labels, ["x", "x", "x"]);
}

#[tokio::test]
async fn nan_is_never_written_as_null() {
    let db = seeded().await;
    let res = db.reading().update_many(vec![reading::id::equals(1)]).data(vec![reading::score::set(f64::NAN)]).exec().await;
    if res.is_ok() {
        assert_ne!(scores(&db).await[0], None, "NaN was stored as null");
    } else {
        assert_validation(res, "set(NaN)");
        assert_eq!(scores(&db).await[0], Some(1.0));
    }
}

// ---------------------------------------------------------------------------------------------
// Values that fail to serialize: one test per helper family
// ---------------------------------------------------------------------------------------------

const BAD: Unserializable = Unserializable(1.0);

async fn assert_filter_refused(db: &OrmClient, p: WhereParam<Reading>, what: &str) {
    let w = p.clone();
    assert_validation(db.reading().find_many(vec![p]).exec().await, &format!("find_many {what}"));
    assert_validation(
        db.reading().update_many(vec![w.clone()]).data(vec![reading::label::set("changed")]).exec().await,
        &format!("update_many {what}"),
    );
    assert_validation(db.reading().delete_many(vec![w]).exec().await, &format!("delete_many {what}"));
    assert_eq!(scores(db).await.len(), 3, "{what}: rows were deleted");
}

async fn assert_data_refused(db: &OrmClient, p: SetParam<Reading>, what: &str) {
    let res = db.reading().update_many(vec![reading::id::equals(1)]).data(vec![p]).exec().await;
    assert_validation(res, what);
}

#[tokio::test]
async fn equals_and_not_refuse_unserializable_values() {
    let db = seeded().await;
    assert_filter_refused(&db, reading::odd::equals(BAD), "equals").await;
    assert_filter_refused(&db, reading::odd::not(BAD), "not").await;
}

#[tokio::test]
async fn in_lists_refuse_unserializable_values() {
    let db = seeded().await;
    assert_filter_refused(&db, reading::odd::in_vec(vec![BAD]), "in_vec").await;
    assert_filter_refused(&db, reading::odd::not_in_vec(vec![BAD]), "not_in_vec").await;
}

#[tokio::test]
async fn comparisons_refuse_unserializable_values() {
    let db = seeded().await;
    assert_filter_refused(&db, reading::odd::lt(BAD), "lt").await;
    assert_filter_refused(&db, reading::odd::lte(BAD), "lte").await;
    assert_filter_refused(&db, reading::odd::gt(BAD), "gt").await;
    assert_filter_refused(&db, reading::odd::gte(BAD), "gte").await;
}

#[tokio::test]
async fn set_refuses_unserializable_values() {
    let db = seeded().await;
    assert_data_refused(&db, reading::odd::set(BAD), "set").await;
    let res = db.reading().create(vec![reading::odd::set(BAD)]).exec().await;
    assert_validation(res, "create with set");
    assert_eq!(scores(&db).await.len(), 3, "a row was created");
}

#[tokio::test]
async fn arithmetic_refuses_unserializable_values() {
    let db = seeded().await;
    assert_data_refused(&db, reading::odd::increment(BAD), "increment").await;
    assert_data_refused(&db, reading::odd::decrement(BAD), "decrement").await;
    assert_data_refused(&db, reading::odd::multiply(BAD), "multiply").await;
    assert_data_refused(&db, reading::odd::divide(BAD), "divide").await;
}

#[tokio::test]
async fn list_filters_refuse_unserializable_values() {
    let db = seeded().await;
    assert_filter_refused(&db, reading::odds::has(BAD), "has").await;
    assert_filter_refused(&db, reading::odds::has_some(vec![BAD]), "has_some").await;
    assert_filter_refused(&db, reading::odds::has_every(vec![BAD]), "has_every").await;
    assert_filter_refused(&db, reading::odds::equals(vec![BAD]), "list equals").await;
}

#[tokio::test]
async fn list_writes_refuse_unserializable_values() {
    let db = seeded().await;
    assert_data_refused(&db, reading::odds::push(BAD), "push").await;
    assert_data_refused(&db, reading::odds::set(vec![BAD]), "list set").await;
}

#[tokio::test]
async fn combinators_and_relations_carry_the_error() {
    let db = seeded().await;
    let bad = || reading::odd::equals(BAD);
    assert_filter_refused(&db, and(vec![reading::id::gt(0), bad()]), "and").await;
    assert_filter_refused(&db, or(vec![reading::id::gt(0), bad()]), "or").await;
    assert_filter_refused(&db, not(vec![bad()]), "not").await;
    let notes = db.note().find_many(vec![note::reading::is(vec![bad()])]).exec().await;
    assert_validation(notes, "relation is");
    assert_filter_refused(&db, reading::notes::some(vec![note::reading::is(vec![bad()])]), "nested some").await;
    let res = db.reading().find_many(()).filter(bad()).exec().await;
    assert_validation(res, "filter");
}
