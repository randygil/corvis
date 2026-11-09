//! Postgres runtime features that the oracle cannot record: interactive transaction
//! options (`maxWait`, `timeout`, `isolationLevel`), sequential transactions,
//! typed `$queryRaw` with `sql!` and JS `undefined` in arguments.

#[path = "common/pg.rs"]
mod pg;

#[allow(dead_code)]
mod oracle {
    include!("generated/oracle.rs");
}

use std::time::Duration;

use corvis_orm::prelude::*;
use corvis_orm::{OrmClientOptions, RawSql, or_undefined, undefined};
use oracle::*;

async fn setup(name: &str, max_connections: u32) -> Option<OrmClient> {
    let url = pg::fresh_db(name).await?;
    let db = OrmClient::with_options(OrmClientOptions::new(url).max_connections(max_connections)).unwrap();
    db.db_execute(include_str!("fixtures/oracle/migration.sql")).await.unwrap();
    Some(db)
}

fn category(name: &str) -> Value {
    json!({ "data": { "name": name } })
}

#[tokio::test]
async fn interactive_transaction_timeout_rolls_back() {
    let Some(db) = setup("feat_timeout", 5).await else { return };
    let res: Result<(), OrmError> = db
        ._transaction_with(TransactionOptions::new().timeout(300), |tx| async move {
            tx.category().create(category("a")).await?;
            tokio::time::sleep(Duration::from_millis(600)).await;
            tx.category().create(category("b")).await?;
            Ok(())
        })
        .await;
    let err = res.unwrap_err();
    assert_eq!(err.code(), Some("P2028"), "{err}");
    assert!(
        err.message().contains("A query cannot be executed on an expired transaction. The timeout for this transaction was 300 ms"),
        "{err}"
    );
    assert_eq!(db.category().count(()).await.unwrap(), 0);

    // The commit of an expired transaction fails too.
    let res: Result<(), OrmError> = db
        ._transaction_with(json!({ "timeout": 200 }), |tx| async move {
            tx.category().create(category("c")).await?;
            tokio::time::sleep(Duration::from_millis(400)).await;
            Ok(())
        })
        .await;
    let err = res.unwrap_err();
    assert!(err.message().contains("A commit cannot be executed on an expired transaction"), "{err}");
    assert_eq!(db.category().count(()).await.unwrap(), 0);

    // Within the timeout everything is committed.
    db._transaction_with(TransactionOptions::new().max_wait(5_000).timeout(10_000), |tx| async move {
        tx.category().create(category("d")).await?;
        Ok::<_, OrmError>(())
    })
    .await
    .unwrap();
    assert_eq!(db.category().count(()).await.unwrap(), 1);
}

#[tokio::test]
async fn max_wait_and_isolation_level() {
    let Some(db) = setup("feat_maxwait", 1).await else { return };
    let holder = db.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let hold = tokio::spawn(async move {
        holder
            ._transaction_with(TransactionOptions::new().timeout(3_000), |tx| async move {
                tx.category().create(category("held")).await?;
                let _ = started_tx.send(());
                tokio::time::sleep(Duration::from_millis(800)).await;
                Ok::<_, OrmError>(())
            })
            .await
    });
    started_rx.await.unwrap();
    let err =
        db._transaction_with(TransactionOptions::new().max_wait(100), |tx| async move { tx.category().count(()).await }).await.unwrap_err();
    assert_eq!(err.code(), Some("P2028"));
    assert!(err.message().contains("Unable to start a transaction in the given time."), "{err}");
    hold.await.unwrap().unwrap();

    let rows = db
        ._transaction_with(json!({ "isolationLevel": "Serializable" }), |tx| async move {
            tx._query_raw(sql!("SHOW transaction_isolation")).await
        })
        .await
        .unwrap();
    assert_eq!(rows, vec![json!({ "transaction_isolation": "serializable" })]);
    let rows = db
        ._transaction_with(TransactionOptions::new().isolation_level(IsolationLevel::RepeatableRead), |tx| async move {
            tx._query_raw(sql!("SHOW transaction_isolation")).await
        })
        .await
        .unwrap();
    assert_eq!(rows[0]["transaction_isolation"], "repeatable read");
}

#[tokio::test]
async fn sequential_transactions() {
    let Some(db) = setup("feat_batch", 5).await else { return };
    let (a, b, n) = db
        ._transaction_batch((db.category().create(category("a")), db.category().create(category("b")), db.category().count(())))
        .await
        .unwrap();
    assert_eq!((a.name.as_str(), b.name.as_str(), n), ("a", "b", 2));

    // A failing query rolls back the whole batch.
    let err = db._transaction_batch(vec![db.category().create(category("c")), db.category().create(category("a"))]).await.unwrap_err();
    assert_eq!(err.code(), Some("P2002"));
    assert_eq!(db.category().count(()).await.unwrap(), 2);

    let updated = db
        ._transaction_batch(
            ["a", "b"]
                .iter()
                .map(|n| db.category().update(json!({ "where": { "name": n }, "data": { "name": format!("{n}2") } })))
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    assert_eq!(updated.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["a2", "b2"]);
}

#[tokio::test]
async fn typed_raw_queries() {
    let Some(db) = setup("feat_raw", 5).await else { return };
    for name in ["tech", "news", "life"] {
        db.category().create(category(name)).await.unwrap();
    }
    let names = vec!["tech".to_string(), "life".to_string()];
    let order = RawSql::raw("ORDER BY name");
    let rows = db._query_raw(sql!("SELECT id, name FROM \"Category\" WHERE name = ANY({names}) {order}")).await.unwrap();
    assert_eq!(rows.iter().map(|r| r["name"].as_str().unwrap()).collect::<Vec<_>>(), ["life", "tech"]);

    let ids = RawSql::join([1, 3], ",", "(", ")");
    let missing: Option<String> = None;
    let cond = if names.is_empty() { RawSql::empty() } else { sql!("AND name <> {\"x\"}") };
    let n: i64 =
        db._query_raw(sql!("SELECT count(*) AS n FROM \"Category\" WHERE id IN {ids} AND {missing}::text IS NULL {cond}")).await.unwrap()
            [0]["n"]
            .as_i64()
            .unwrap();
    assert_eq!(n, 2);

    #[derive(Deserialize)]
    struct Row {
        n: Decimal,
        d: DateTime,
        b: i64,
    }
    let price = Decimal::from("10.25");
    let when = DateTime::parse("2024-05-06T10:00:00.000Z").unwrap();
    let rows: Vec<Row> = db._query_raw_as(sql!("SELECT {price} * 2 AS n, {when} AS d, {7}::bigint AS b")).await.unwrap();
    assert_eq!((rows[0].n.to_string(), rows[0].d.to_iso_string(), rows[0].b), ("20.5".into(), "2024-05-06T10:00:00.000Z".into(), 7));

    // Inside a transaction, with row locks; braces are escaped with `{{` / `}}`.
    let affected = db
        ._transaction(|tx| async move {
            let locked = tx._query_raw(sql!("SELECT id FROM \"Category\" WHERE name = {\"tech\"} FOR UPDATE")).await?;
            assert_eq!(locked.len(), 1);
            let arr = tx._query_raw(sql!("SELECT '{{1,2}}'::int[] AS a")).await?;
            assert_eq!(arr[0]["a"], json!([1, 2]));
            tx._execute_raw(sql!("UPDATE \"Category\" SET name = name || '!' WHERE name IN ({\"news\"}, {\"life\"})")).await
        })
        .await
        .unwrap();
    assert_eq!(affected, 2);

    let err = db._execute_raw(sql!("INSERT INTO \"Category\" (name) VALUES ({\"forbidden\"})")).await.unwrap_err();
    assert_eq!(err.code(), Some("P2010"));
    assert_eq!(err.meta().unwrap()["code"], "P0001");
    // The same trigger in a model query is an unknown request error carrying the SQLSTATE.
    let err = db.category().create(category("forbidden")).await.unwrap_err();
    assert_eq!(err.name(), "UnknownRequestError");
    assert!(err.message().contains("code: \"P0001\"") && err.message().contains("CATEGORY_FORBIDDEN"), "{err}");
}

#[tokio::test]
async fn undefined_arguments_are_dropped() {
    let Some(db) = setup("feat_undefined", 5).await else { return };
    for name in ["a", "b"] {
        db.category().create(category(name)).await.unwrap();
    }
    let from: Option<i32> = None;
    let all: Vec<Category> = db
        .category()
        .find_many(json!({ "where": { "id": { "gte": or_undefined(from), "lte": undefined() }, "name": undefined() }, "orderBy": { "id": "asc" } }))
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    let some: Vec<Category> = db
        .category()
        .find_many(json!({ "where": { "OR": [{ "name": "a" }, undefined()], "id": { "gte": or_undefined(Some(1)) } } }))
        .await
        .unwrap();
    assert_eq!(some.len(), 1);
    // `undefined` in data leaves the column untouched; `null` would write NULL.
    let u: User = db.user().create(json!({ "data": { "email": "x@y.z", "name": "X" } })).await.unwrap();
    let u: User = db.user().update(json!({ "where": { "id": u.id }, "data": { "name": undefined(), "active": false } })).await.unwrap();
    assert_eq!((u.name.as_deref(), u.active), (Some("X"), false));
}

/// A `NULL` parameter is bound with a guessed type (`text`, `timestamp`, ...). The same SQL
/// text later executed with a typed value (`uuid`, `date`) must not reuse the prepared
/// statement of the `NULL` call: Postgres would read the binary value with the old type
/// ("invalid byte sequence for encoding UTF8", "insufficient data left in message").
#[tokio::test]
async fn null_and_typed_parameters_do_not_share_a_prepared_statement() {
    let Some(db) = setup("feat_null_params", 1).await else { return };
    let day = chrono::NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
    let id = uuid::Uuid::parse_str("b91cbacc-3326-4ba0-b32c-c41b96a370b2").unwrap();
    for _ in 0..2 {
        for (d, u) in [(None, None), (Some(day), Some(id)), (None, Some(id)), (Some(day), None)] {
            let rows = db._query_raw(corvis_orm::sql!("SELECT {d}::date::text AS d, {u}::uuid::text AS u")).await.unwrap();
            assert_eq!(rows[0]["d"], d.map_or(Value::Null, |_| json!("2026-08-15")));
            assert_eq!(rows[0]["u"], u.map_or(Value::Null, |u| json!(u.to_string())));
        }
    }
}

#[tokio::test]
async fn raw_parameters_of_different_types_do_not_share_a_prepared_statement() {
    let Some(db) = setup("feat_param_types", 1).await else { return };
    for _ in 0..2 {
        for v in [json!(1), json!("x"), json!(1.5), json!(true), json!({ "a": 1 }), json!(null)] {
            let rows = db.query_raw("SELECT $1::text AS v", vec![v.clone()]).await.unwrap();
            let expected = match &v {
                Value::Null => Value::Null,
                Value::String(s) => json!(s),
                Value::Object(_) => json!("{\"a\": 1}"),
                other => json!(other.to_string()),
            };
            assert_eq!(rows[0]["v"], expected, "{v}");
        }
    }
    // The type comment is appended safely after a `;` or a trailing line comment.
    for sql in ["SELECT $1::int AS v;", "SELECT $1::int AS v -- note"] {
        assert_eq!(db.query_raw(sql, vec![json!(7)]).await.unwrap()[0]["v"], json!(7), "{sql}");
    }
}

// ---------------------------------------------------------------------------------------------
// The helper itself: `CORVIS_ORM_PG_REQUIRED=1` (set by CI) turns a skip into a failure
// ---------------------------------------------------------------------------------------------

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(s) => *s,
        Err(p) => p.downcast_ref::<&str>().map(|s| s.to_string()).unwrap_or_default(),
    }
}

/// Runs `fresh_db_at` with the variable's policy set to "required" and returns the panic message.
async fn required_failure(base: &'static str) -> String {
    let err = tokio::spawn(pg::fresh_db_at(base, true, "required_check")).await.expect_err("a required server that is down must panic");
    panic_message(err.into_panic())
}

#[tokio::test]
async fn required_postgres_fails_instead_of_skipping() {
    let base = "postgres://app:p4ss-w0rd@127.0.0.1:5299";
    // Without the variable: skipped, as in a local run without a server.
    assert!(pg::fresh_db_at(base, false, "required_check").await.is_none());

    let msg = required_failure(base).await;
    assert!(msg.contains("CORVIS_ORM_PG_REQUIRED=1") && msg.contains("CORVIS_ORM_PG)"), "{msg}");
    assert!(msg.contains("127.0.0.1:5299") && msg.contains("is not reachable"), "{msg}");
    assert!(!msg.contains("p4ss-w0rd") && !msg.contains("app:"), "{msg}");

    let msg = required_failure("postgres://app:p4ss-w0rd@127.0.0.1:not-a-port").await;
    assert!(msg.contains("has an invalid URL"), "{msg}");
    assert!(!msg.contains("p4ss-w0rd") && !msg.contains("app:"), "{msg}");
}

#[test]
fn server_host_never_includes_credentials() {
    assert_eq!(pg::host("postgres://postgres:postgres@127.0.0.1:55432"), "127.0.0.1:55432");
    assert_eq!(pg::host("postgres://u:p@ss@db.internal:5432/app?sslmode=require"), "db.internal:5432");
    assert_eq!(pg::host("postgres://db.internal"), "db.internal");
}

#[derive(serde::Serialize)]
struct NewUser {
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    balance: String,
}

#[tokio::test]
async fn typed_operations_data_and_counts_on_postgres() {
    let Some(db) = setup("feat_typed_api", 5).await else { return };

    // `createManyAndReturn` / `updateManyAndReturn` equal the untyped results.
    let created = db.user().create_many_and_return(json!({ "data": [{ "email": "a@x.io" }, { "email": "b@x.io" }] })).await.unwrap();
    assert_eq!(created.len(), 2);
    assert!(created.iter().all(|u| u.id > 0));
    let raw = db
        .query_json(User::meta(), "createManyAndReturn", json!({ "data": [{ "email": "c@x.io" }, { "email": "d@x.io" }] }))
        .await
        .unwrap();
    let typed =
        db.user().create_many_and_return(json!({ "data": [{ "email": "e@x.io" }, { "email": "f@x.io" }] })).exec_json().await.unwrap();
    assert_eq!(raw.as_array().unwrap().len(), typed.as_array().unwrap().len());
    let updated = db
        .user()
        .update_many_and_return(json!({ "where": { "email": { "endsWith": "@x.io" } }, "data": { "name": "bulk" } }))
        .await
        .unwrap();
    assert_eq!(updated.len(), 6);
    assert!(updated.iter().all(|u| u.name.as_deref() == Some("bulk")));

    // `Data` creates from a DTO and skips the absent fields.
    let dto = NewUser { email: "ana@x.io".into(), name: None, balance: "12.50".into() };
    let ana = db.user().create(corvis_orm::Data(&dto)).await.unwrap();
    assert_eq!(ana.name, None);
    assert_eq!(ana.balance.to_string(), "12.5");

    // Relation counts: two counts merge and the typed result serializes like `.exec_json()`.
    db.post().create(json!({ "data": { "title": "p1", "authorId": ana.id } })).await.unwrap();
    db.post().create(json!({ "data": { "title": "p2", "authorId": ana.id } })).await.unwrap();
    db.wallet()
        .create(json!({ "data": { "id": "6f1c2b0e-7a53-4e0c-9d57-1f2b0c3a4d5e", "userId": ana.id, "currency": "USD", "amount": "1.5" } }))
        .await
        .unwrap();
    let build = || {
        db.user()
            .find_many(json!({ "where": { "email": { "in": ["ana@x.io", "a@x.io"] } }, "orderBy": { "id": "asc" } }))
            .include(user::posts::count())
            .include(user::wallets::count())
    };
    let json = build().exec_json().await.unwrap();
    let rows = build().exec_as::<Vec<user::WithCount>>().await.unwrap();
    assert_eq!(rows.len(), 2);
    let ana_row = rows.iter().find(|r| r.model.email == "ana@x.io").unwrap();
    assert_eq!(ana_row.count.posts, Some(2));
    assert_eq!(ana_row.count.wallets, Some(1));
    assert_eq!(ana_row.model.balance.to_string(), "12.5");
    assert_eq!(serde_json::to_string(&rows).unwrap(), serde_json::to_string(&json).unwrap());
}

#[tokio::test]
async fn throwaway_failing_postgres_assertion() {
    let Some(_db) = setup("feat_throwaway", 1).await else { return };
    assert_eq!(1 + 1, 3, "throwaway: deliberate failure");
}
