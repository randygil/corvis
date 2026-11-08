//! Regression tests of the security review of corvis-orm: hostile query arguments (decimals,
//! nesting, unique filters, in-lists, numbers out of range), atomic writes under concurrency,
//! transaction expiry, connection errors and TLS.
//!
//! The Postgres tests use `CORVIS_ORM_PG` and `orm_test_sec_*` databases (skipped when the
//! server is not reachable).

#[path = "common/pg.rs"]
mod pg;

use std::time::{Duration, Instant};

use corvis_orm::prelude::*;
use corvis_orm::{OrmClientOptions, TransactionOptions};

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

async fn sqlite() -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<(Account,)>().await.unwrap();
    db
}

async fn postgres(name: &str, pool: u32) -> Option<OrmClient> {
    let url = pg::fresh_db(&format!("sec_{name}")).await?;
    let db = OrmClient::with_options(OrmClientOptions::new(url).max_connections(pool)).unwrap();
    db.db_push::<(Account,)>().await.unwrap();
    Some(db)
}

async fn seed(db: &OrmClient) {
    for (i, (email, tok, name)) in [("victim@x", "s3cr3t-token", "alice"), ("other@x", "zzz-other", "bob")].iter().enumerate() {
        q(
            db,
            "create",
            json!({ "data": {
            "email": email, "reset_token": tok, "status": "PENDING", "balance": "1", "data": { "name": name, "n": i }
        } }),
        )
        .await
        .unwrap();
    }
}

/// Runs `f` on a thread with the 2 MiB stack of a tokio worker.
fn on_small_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(f).unwrap().join().expect("the thread must not crash");
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

// ---------------------------------------------------------------------------------------------
// Decimal
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn decimal_with_a_huge_exponent_is_an_error_not_an_abort() {
    let db = sqlite().await;
    seed(&db).await;
    // Used to try to allocate 10^14 bytes (`to_fixed_plain`) and abort the process.
    for args in [json!({ "where": { "balance": { "gt": "1e100000000000000" } } }), json!({ "where": { "balance": "-1e140000" } })] {
        let e = q(&db, "findMany", args).await.unwrap_err();
        assert!(e.message().contains("Couldn't convert an integer (possible overflow)."), "{e}");
    }
    // Written values are also checked against the Decimal(65, 30) column first.
    let e = q(&db, "create", json!({ "data": { "email": "n@x", "status": "x", "balance": "1e100000000000000" } })).await.unwrap_err();
    assert!(e.is_validation(), "{e}");
}

#[tokio::test]
async fn decimal_exponent_overflow_does_not_panic() {
    let db = sqlite().await;
    seed(&db).await;
    // `exp - frac_len` used to overflow (panic in debug, a wrapped exponent in release). Like
    // decimal.js, an exponent below `minE` (-9e15) is zero and one above `maxE` is Infinity.
    let r = q(&db, "count", json!({ "where": { "balance": "0.11e-9223372036854775807" } })).await.unwrap();
    assert_eq!(r, json!(0));
    assert!(Decimal::parse("0.11e-9223372036854775807").unwrap().is_zero());
    assert!(!Decimal::parse("1e9223372036854775807").unwrap().is_finite());
    assert!(!Decimal::parse("1e99999999999999999999999999").unwrap().is_finite());
}

#[tokio::test]
async fn decimal_trailing_zeros_are_parsed_in_linear_time() {
    let db = sqlite().await;
    let s = format!("1{}", "0".repeat(99_000));
    let t = Instant::now();
    let d = Decimal::parse(&s).unwrap();
    assert_eq!(d.to_string(), "1e+99000");
    // A run of zeros produced by arithmetic is normalized quickly too.
    let big = Decimal::parse(&format!("123{}", "0".repeat(50_000))).unwrap().times(1);
    assert_eq!(big.to_string(), "1.23e+50002");
    let r = q(&db, "count", json!({ "where": { "balance": { "equals": s } } })).await.unwrap();
    assert_eq!(r, json!(0));
    // Was ~1 s in release (one BigUint division per zero), much more in debug.
    assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
}

#[tokio::test]
async fn sqlite_decimal_columns_are_bounded_like_decimal_65_30() {
    let db = sqlite().await;
    // "1e300000" (8 bytes) used to store 300001 digits, and every read re-parsed them (seconds).
    let e = q(&db, "create", json!({ "data": { "email": "a@x", "status": "x", "balance": "1e300000" } })).await.unwrap_err();
    assert!(e.is_validation(), "{e}");
    assert!(e.message().contains("Decimal(65, 30)"), "{e}");
    let e = q(&db, "create", json!({ "data": { "email": "a@x", "status": "x", "balance": "1e35" } })).await.unwrap_err();
    assert!(e.is_validation(), "{e}");
    // Postgres rounds to the scale of the column; so does the SQLite column.
    let r = q(&db, "create", json!({ "data": { "email": "b@x", "status": "x", "balance": "1e-300000" } })).await.unwrap();
    assert_eq!(r["balance"], json!("0"));
    let r = q(&db, "create", json!({ "data": { "email": "c@x", "status": "x", "balance": "0.0000000000000000000000000000015" } }))
        .await
        .unwrap();
    assert_eq!(r["balance"], json!("2e-30"));
    let r = q(&db, "update", json!({ "where": { "id": 2 }, "data": { "balance": { "set": "99999999999999999999999999999999999.4" } } }))
        .await
        .unwrap();
    assert_eq!(r["balance"], json!("9.99999999999999999999999999999999994e+34"));
    let raw = db.query_raw("SELECT max(length(balance)) AS n FROM accounts", vec![]).await.unwrap();
    assert!(raw[0]["n"].as_i64().unwrap() < 80, "{raw:?}");
}

#[tokio::test]
async fn postgres_decimal_is_never_truncated() {
    let Some(db) = postgres("decimal", 2).await else { return };
    // `db push` creates `DECIMAL(65,30)` (it used to be `TEXT`).
    let ty = db
        .query_raw("SELECT data_type, numeric_precision, numeric_scale FROM information_schema.columns WHERE table_name = 'accounts' AND column_name = 'balance'", vec![])
        .await
        .unwrap();
    assert_eq!(ty[0], json!({ "data_type": "numeric", "numeric_precision": 65, "numeric_scale": 30 }));
    // The 16-bit weight / scale used to wrap: 9999e-262144 was stored as 9999, 5e262144 as 5.
    for v in ["9999e-262144", "5e262144", "1e140000"] {
        let e = q(&db, "create", json!({ "data": { "email": format!("{v}@x"), "status": "x", "balance": v } })).await.unwrap_err();
        assert!(e.message().contains("Couldn't convert an integer (possible overflow)."), "{v}: {e}");
        let e = q(&db, "count", json!({ "where": { "balance": { "lte": v } } })).await.unwrap_err();
        assert!(e.message().contains("ToSql(0)") && e.message().contains("Couldn't convert"), "{v}: {e}");
    }
    assert_eq!(q(&db, "count", json!({})).await.unwrap(), json!(0));
    // Within the NUMERIC range, values go through unchanged (Postgres applies the column type).
    q(&db, "create", json!({ "data": { "email": "a@x", "status": "x", "balance": "12.5" } })).await.unwrap();
    assert_eq!(q(&db, "count", json!({ "where": { "balance": { "lt": "1e131071" } } })).await.unwrap(), json!(1));
    assert_eq!(q(&db, "count", json!({ "where": { "balance": { "gt": "1e-16383" } } })).await.unwrap(), json!(1));
    let e = q(&db, "create", json!({ "data": { "email": "b@x", "status": "x", "balance": "1e40" } })).await.unwrap_err();
    assert!(e.message().contains("numeric field overflow"), "{e}");
}

// ---------------------------------------------------------------------------------------------
// Nesting
// ---------------------------------------------------------------------------------------------

/// `{ key: v }` without `json!` (which would serialize `v` recursively).
fn wrap(key: &str, v: Value) -> Value {
    let mut m = serde_json::Map::new();
    m.insert(key.to_string(), v);
    Value::Object(m)
}

fn nested_not(depth: usize) -> Value {
    let mut w = json!({});
    for _ in 0..depth {
        w = wrap("NOT", w);
    }
    w
}

/// Drops a value without recursion (serde_json's `Drop` recurses once per level).
fn drop_flat(v: Value) {
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(m) => stack.extend(m.into_iter().map(|(_, x)| x)),
            Value::Array(a) => stack.extend(a),
            _ => {}
        }
    }
}

#[test]
fn deep_where_is_rejected_before_it_overflows_the_stack() {
    on_small_stack(|| {
        runtime().block_on(async {
            let db = sqlite().await;
            seed(&db).await;
            // `{ where: <d levels of NOT around {}> }` has depth d + 2: 123 levels give 125, the
            // limit of the reference query engine (accepted), 124 give 126 (rejected there too).
            assert_eq!(q(&db, "count", json!({ "where": nested_not(123) })).await.unwrap(), json!(0));
            assert_eq!(q(&db, "count", json!({ "where": nested_not(122) })).await.unwrap(), json!(2));
            let mut and = json!({ "status": { "not": { "equals": "X" } } });
            for _ in 0..60 {
                and = json!({ "AND": [and], "OR": [{ "id": { "gt": 0 } }] });
            }
            assert_eq!(q(&db, "count", json!({ "where": and })).await.unwrap(), json!(2));
            let e = q(&db, "count", json!({ "where": nested_not(124) })).await.unwrap_err();
            assert!(e.is_validation() && e.message().contains("nested too deeply"), "{e}");
            // 10000 levels used to overflow the stack (STATUS_STACK_OVERFLOW, process abort).
            let e = q(&db, "findMany", wrap("where", nested_not(10_000))).await.unwrap_err();
            assert!(e.is_validation(), "{e}");
            let mut having = json!({ "status": "PENDING" });
            for _ in 0..10_000 {
                having = wrap("NOT", Value::Array(vec![having]));
            }
            let mut args = wrap("having", having);
            args.as_object_mut().unwrap().insert("by".into(), json!(["status"]));
            let e = q(&db, "groupBy", args).await.unwrap_err();
            assert!(e.is_validation(), "{e}");
            let deep = nested_not(10_000);
            let e = corvis_orm::debug_sql(&db, meta(), &deep).unwrap_err();
            assert!(e.is_validation(), "{e}");
            drop_flat(deep);
            // The limit is configurable.
            let strict = OrmClient::with_options(OrmClientOptions::new("sqlite::memory:").max_query_depth(10)).unwrap();
            strict.db_push::<(Account,)>().await.unwrap();
            assert!(q(&strict, "count", json!({ "where": nested_not(10) })).await.is_err());
            assert!(q(&strict, "count", json!({ "where": nested_not(8) })).await.is_ok());
        })
    });
}

#[test]
fn psl_deep_nesting_is_a_parse_error() {
    on_small_stack(|| {
        let deep = format!("model U {{\n id Int @id @default({}1{})\n}}\n", "[".repeat(50_000), "]".repeat(50_000));
        let e = corvis_orm::psl::parse(&deep).unwrap_err();
        assert!(e.to_string().contains("nested too deeply"), "{e}");
        let ok = format!("model U {{\n id Int @id @default({}1{})\n}}\n", "[".repeat(10), "]".repeat(10));
        assert!(corvis_orm::psl::parse(&ok).is_ok());
    });
}

// ---------------------------------------------------------------------------------------------
// WhereUniqueInput
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn where_unique_only_accepts_values_on_unique_fields() {
    let db = sqlite().await;
    seed(&db).await;
    // A password-reset lookup by two secrets must not become a lookup by one.
    for (action, args) in [
        ("findUnique", json!({ "where": { "email": "victim@x", "reset_token": { "not": "guess" } } })),
        ("findUniqueOrThrow", json!({ "where": { "email": "victim@x", "reset_token": { "startsWith": "" } } })),
        ("update", json!({ "where": { "email": "victim@x", "reset_token": { "startsWith": "" } }, "data": { "status": "PWNED" } })),
        ("delete", json!({ "where": { "id": 1, "email": { "contains": "@" } } })),
        (
            "upsert",
            json!({ "where": { "email": { "equals": "victim@x" } }, "create": { "email": "z@x", "status": "x", "balance": "1" }, "update": {} }),
        ),
        ("findUnique", json!({ "where": { "id": { "equals": 1 } } })),
        ("findUnique", json!({ "where": { "id": 1, "email": null } })),
        ("findMany", json!({ "cursor": { "id": { "gt": 0 } } })),
    ] {
        let e = q(&db, action, args.clone()).await.unwrap_err();
        assert!(e.is_validation(), "{action} {args}: {e}");
    }
    let e = q(&db, "findUnique", json!({ "where": { "email": "victim@x", "reset_token": { "not": "guess" } } })).await.unwrap_err();
    assert!(e.message().ends_with("Argument `reset_token`: Invalid value provided. Expected String, provided Object."), "{e}");
    let row = q(&db, "findUnique", json!({ "where": { "id": 1 } })).await.unwrap();
    assert_eq!(row["status"], json!("PENDING"));
    // Non-unique fields keep accepting filters.
    let r = q(&db, "findUnique", json!({ "where": { "id": 1, "status": { "not": "PAID" } } })).await.unwrap();
    assert_eq!(r["email"], json!("victim@x"));
}

// ---------------------------------------------------------------------------------------------
// Atomic writes
// ---------------------------------------------------------------------------------------------

/// 20 concurrent "pay the order while it is still PENDING": exactly one may win.
async fn race_update(db: &OrmClient) -> (usize, usize, Value) {
    let created = q(db, "create", json!({ "data": { "email": "race@x", "status": "PENDING", "balance": "0" } })).await.unwrap();
    let id = created["id"].clone();
    let mut hs = Vec::new();
    for _ in 0..20 {
        let db = db.clone();
        let id = id.clone();
        hs.push(tokio::spawn(async move {
            q(
                &db,
                "update",
                json!({ "where": { "id": id, "status": "PENDING" }, "data": { "status": "PAID", "version": { "increment": 1 } } }),
            )
            .await
        }));
    }
    let (mut ok, mut not_found) = (0, 0);
    for h in hs {
        match h.await.unwrap() {
            Ok(_) => ok += 1,
            Err(e) if e.is_not_found() => {
                assert_eq!(e.meta().unwrap()["cause"], json!("No record was found for an update."));
                not_found += 1
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
    let row = q(db, "findUnique", json!({ "where": { "id": id } })).await.unwrap();
    (ok, not_found, row["version"].clone())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn update_applies_its_filter_atomically_sqlite() {
    let db = sqlite().await;
    let (ok, not_found, version) = race_update(&db).await;
    assert_eq!((ok, not_found, version), (1, 19, json!(1)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn update_applies_its_filter_atomically_postgres() {
    let Some(db) = postgres("race_update", 10).await else { return };
    let (ok, not_found, version) = race_update(&db).await;
    assert_eq!((ok, not_found, version), (1, 19, json!(1)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn update_many_optimistic_concurrency_has_one_winner_postgres() {
    let Some(db) = postgres("race_occ", 10).await else { return };
    q(&db, "create", json!({ "data": { "email": "b@x", "status": "PENDING", "balance": "0" } })).await.unwrap();
    let mut winners = Vec::new();
    for _ in 0..10 {
        q(&db, "updateMany", json!({ "where": { "id": 1 }, "data": { "version": 0 } })).await.unwrap();
        let mut hs = Vec::new();
        for _ in 0..20 {
            let db = db.clone();
            hs.push(tokio::spawn(async move {
                q(&db, "updateMany", json!({ "where": { "id": 1, "version": 0 }, "data": { "version": { "increment": 1 } } })).await
            }));
        }
        let mut total = 0;
        for h in hs {
            total += h.await.unwrap().unwrap()["count"].as_i64().unwrap();
        }
        winners.push(total);
    }
    assert_eq!(winners, vec![1; 10]);
    // Same with `limit` and with `updateManyAndReturn`.
    q(&db, "updateMany", json!({ "where": { "id": 1 }, "data": { "version": 0 } })).await.unwrap();
    let mut hs = Vec::new();
    for i in 0..20 {
        let db = db.clone();
        hs.push(tokio::spawn(async move {
            if i % 2 == 0 {
                let r =
                    q(&db, "updateMany", json!({ "where": { "version": 0 }, "data": { "version": { "increment": 1 } }, "limit": 1 })).await;
                r.unwrap()["count"].as_i64().unwrap()
            } else {
                let r =
                    q(&db, "updateManyAndReturn", json!({ "where": { "version": 0 }, "data": { "version": { "increment": 1 } } })).await;
                r.unwrap().as_array().unwrap().len() as i64
            }
        }));
    }
    let mut total = 0;
    for h in hs {
        total += h.await.unwrap();
    }
    assert_eq!(total, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn delete_and_upsert_apply_their_filter_atomically_postgres() {
    let Some(db) = postgres("race_delete", 10).await else { return };
    q(&db, "create", json!({ "data": { "email": "d@x", "status": "PENDING", "balance": "0" } })).await.unwrap();
    let mut hs = Vec::new();
    for i in 0..20 {
        let db = db.clone();
        hs.push(tokio::spawn(async move {
            if i % 2 == 0 {
                q(&db, "delete", json!({ "where": { "id": 1, "status": "PENDING" } })).await.map(|_| "delete")
            } else {
                // Non-native upsert (the `where` has a non-unique filter): update only while PENDING.
                q(&db, "upsert", json!({ "where": { "id": 1, "status": "PENDING" }, "create": { "email": "e@x", "status": "NEW", "balance": "0" }, "update": { "status": "DONE" } }))
                    .await
                    .map(|_| "upsert")
            }
        }));
    }
    let mut results = Vec::new();
    for h in hs {
        results.push(h.await.unwrap());
    }
    let deletes = results.iter().filter(|r| matches!(r, Ok("delete"))).count();
    let updates = q(&db, "count", json!({ "where": { "status": "DONE" } })).await.unwrap().as_i64().unwrap();
    // Either the row was deleted once, or updated once and then nobody could delete it.
    assert!(deletes + updates as usize <= 1, "deletes={deletes} updates={updates} {results:?}");
    for r in &results {
        if let Err(e) = r {
            assert!(e.is_not_found() || e.is_unique_violation(), "{e}");
        }
    }
}

#[tokio::test]
async fn delete_with_a_stale_filter_is_not_found() {
    let db = sqlite().await;
    seed(&db).await;
    let e = q(&db, "delete", json!({ "where": { "id": 1, "status": "PAID" } })).await.unwrap_err();
    assert_eq!(e.meta().unwrap()["cause"], json!("No record was found for a delete."));
    let e = q(&db, "update", json!({ "where": { "id": 1, "status": "PAID" }, "data": { "status": "X" } })).await.unwrap_err();
    assert!(e.is_not_found());
    assert_eq!(q(&db, "count", json!({ "where": { "status": "PENDING" } })).await.unwrap(), json!(2));
}

// ---------------------------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn expired_transaction_never_reports_success() {
    let db = sqlite().await;
    let mut silent = 0;
    for i in 0..1500u64 {
        let email = format!("u{i}@x");
        let e2 = email.clone();
        let res: Result<(), OrmError> = db
            ._transaction_with(TransactionOptions::new().timeout(3), |tx| async move {
                q(&tx, "create", json!({ "data": { "email": e2, "status": "x", "balance": "1" } })).await?;
                tokio::time::sleep(Duration::from_micros(2_000 + (i % 17) * 60)).await;
                Ok(())
            })
            .await;
        let exists = q(&db, "count", json!({ "where": { "email": email } })).await.unwrap() == json!(1);
        if res.is_ok() && !exists {
            silent += 1;
        }
    }
    assert_eq!(silent, 0, "transactions reported Ok although they were rolled back");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_nested_writes_do_not_leak_timer_tasks() {
    let Some(db) = postgres("tx_leak", 5).await else { return };
    let m = tokio::runtime::Handle::current().metrics();
    q(&db, "count", json!({})).await.unwrap();
    let before = m.num_alive_tasks();
    let mut cancelled = 0;
    for i in 0..300 {
        // `createMany` runs in a transaction with a one-year timeout; drop it half-way.
        let fut = q(
            &db,
            "createMany",
            json!({ "data": [
            { "email": format!("a{i}@x"), "status": "x", "balance": "1" },
            { "email": format!("b{i}@x"), "status": "x", "balance": "1" }
        ] }),
        );
        if tokio::time::timeout(Duration::from_micros(500 + (i as u64 * 37) % 5000), fut).await.is_err() {
            cancelled += 1;
        }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after = m.num_alive_tasks();
    assert!(cancelled > 0);
    assert!(after <= before + 2, "alive tasks {before} -> {after} after {cancelled} cancellations");
    assert!(q(&db, "count", json!({})).await.is_ok());
}

// ---------------------------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn connection_errors_do_not_show_credentials() {
    for url in [
        "postgres://app:p@ssw0rd@127.0.0.1:5299/db",
        "postgres://127.0.0.1:5299/db?user=app&password=SuperSecreta",
        "postgresql:/app:SuperSecreta@127.0.0.1:5299/db",
        "mysql://app:SuperSecreta@127.0.0.1/db",
    ] {
        let mut opts = OrmClientOptions::new(url);
        opts.acquire_timeout = Some(Duration::from_secs(2));
        let msg = match OrmClient::with_options(opts) {
            Ok(c) => c.connect().await.unwrap_err().message().to_string(),
            Err(e) => e.message().to_string(),
        };
        assert!(!msg.contains("ssw0rd") && !msg.contains("SuperSecreta") && !msg.contains("app:"), "{url} -> {msg}");
    }
    let mut opts = OrmClientOptions::new("postgres://app:p@ssw0rd@127.0.0.1:5299/db");
    opts.acquire_timeout = Some(Duration::from_secs(2));
    let e = OrmClient::with_options(opts).unwrap().connect().await.unwrap_err();
    assert_eq!(e.code(), Some("P1001"));
    assert!(e.message().starts_with("Can't reach database server at `127.0.0.1:5299`"), "{e}");
}

#[tokio::test]
async fn postgres_supports_tls() {
    if pg::fresh_db("sec_tls").await.is_none() {
        return;
    }
    // Before: "TLS upgrade required by connect options but SQLx was built without TLS support".
    let url = format!("{}/orm_test_sec_tls?sslmode=require", pg::base_url());
    let r = OrmClient::new(url).unwrap().connect().await;
    if let Err(e) = &r {
        assert!(!e.message().contains("without TLS support"), "{e}");
    }
    // `prefer` falls back to plain text when the server has no TLS.
    let url = format!("{}/orm_test_sec_tls?sslmode=prefer", pg::base_url());
    OrmClient::new(url).unwrap().connect().await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// Low-severity inputs
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn json_string_filters_are_literal_on_sqlite() {
    let db = sqlite().await;
    seed(&db).await;
    for (op, needle) in [
        ("string_contains", "*"),
        ("string_contains", "%"),
        ("string_starts_with", "?"),
        ("string_starts_with", "[a-z]"),
        ("string_ends_with", "_"),
    ] {
        let n = q(&db, "count", json!({ "where": { "data": { "path": ["name"], op: needle } } })).await.unwrap();
        assert_eq!(n, json!(0), "{op} {needle}");
    }
    for (op, needle, expected) in
        [("string_contains", "lic", 1), ("string_starts_with", "bo", 1), ("string_ends_with", "ce", 1), ("string_ends_with", "", 2)]
    {
        let n = q(&db, "count", json!({ "where": { "data": { "path": ["name"], op: needle } } })).await.unwrap();
        assert_eq!(n, json!(expected), "{op} {needle}");
    }
}

#[tokio::test]
async fn take_and_limit_are_bounded() {
    let db = sqlite().await;
    seed(&db).await;
    // `take: i64::MIN` used to panic (`abs` overflow) in debug.
    let r = q(&db, "findMany", json!({ "take": i64::MIN })).await.unwrap();
    assert_eq!(r.as_array().unwrap().len(), 2);
    let r = q(&db, "aggregate", json!({ "take": i64::MIN, "_count": true })).await.unwrap();
    assert_eq!(r["_count"], json!(2));
    // A negative `limit` used to mean "no limit" on SQLite.
    for action in ["deleteMany", "updateMany"] {
        let mut args = json!({ "where": {}, "limit": -1 });
        if action == "updateMany" {
            args["data"] = json!({ "status": "X" });
        }
        let e = q(&db, action, args).await.unwrap_err();
        assert_eq!(e.code(), Some("P2019"));
        assert_eq!(e.meta().unwrap()["details"], json!("Provided limit (-1) must be a positive integer."));
    }
    assert_eq!(q(&db, "count", json!({ "where": { "status": "PENDING" } })).await.unwrap(), json!(2));
}

#[tokio::test]
async fn bigint_and_bytes_out_of_range_are_rejected() {
    let db = sqlite().await;
    for body in [
        r#"{"email":"a@x","status":"x","balance":"1","big":18446744073709551615}"#,
        r#"{"email":"b@x","status":"x","balance":"1","big":-1e30}"#,
        r#"{"email":"c@x","status":"x","balance":"1","blob":[1,256]}"#,
    ] {
        let data: Value = serde_json::from_str(body).unwrap();
        let e = q(&db, "create", json!({ "data": data })).await.unwrap_err();
        assert!(e.is_validation(), "{body}: {e}");
    }
    assert_eq!(q(&db, "count", json!({})).await.unwrap(), json!(0));
    let r = q(
        &db,
        "create",
        json!({ "data": { "email": "d@x", "status": "x", "balance": "1", "big": 9007199254740991i64, "blob": [0, 255] } }),
    )
    .await
    .unwrap();
    assert_eq!((r["big"].clone(), r["blob"].clone()), (json!(9007199254740991i64), json!([0, 255])));
    // Raw arguments: an u64 beyond i64 is bound as a number, not wrapped to a negative one.
    let max = u64::MAX;
    let rows = db._query_raw(corvis_orm::sql!("SELECT {max} AS v")).await.unwrap();
    assert_eq!(rows[0]["v"].to_string().trim_matches('"'), "18446744073709551615");
}

#[tokio::test]
async fn long_in_lists_are_sent_as_one_parameter_sqlite() {
    let db = sqlite().await;
    seed(&db).await;
    let ids: Vec<i64> = (0..40_000).collect();
    // Used to fail with "too many SQL variables".
    assert_eq!(q(&db, "count", json!({ "where": { "id": { "in": ids } } })).await.unwrap(), json!(2));
    assert_eq!(q(&db, "count", json!({ "where": { "id": { "notIn": ids } } })).await.unwrap(), json!(0));
    let emails: Vec<String> = (0..5_000).map(|i| format!("u{i}@x")).chain(["victim@x".to_string()]).collect();
    assert_eq!(q(&db, "count", json!({ "where": { "email": { "in": emails } } })).await.unwrap(), json!(1));
}

#[tokio::test]
async fn long_in_lists_are_sent_as_one_parameter_postgres() {
    let Some(db) = postgres("in_list", 2).await else { return };
    seed(&db).await;
    for n in [65_535i64, 70_000, 131_072] {
        let ids: Vec<i64> = (0..n).collect();
        // Used to fail with "too many arguments for query".
        assert_eq!(q(&db, "count", json!({ "where": { "id": { "in": ids } } })).await.unwrap(), json!(2), "{n}");
        assert_eq!(q(&db, "count", json!({ "where": { "id": { "notIn": ids } } })).await.unwrap(), json!(0), "{n}");
    }
    let tokens: Vec<String> = (0..5_000).map(|i| format!("t{i}")).chain(["zzz-other".to_string()]).collect();
    assert_eq!(q(&db, "count", json!({ "where": { "reset_token": { "in": tokens } } })).await.unwrap(), json!(1));
    // An Int out of range inside a long list still fails like a single value.
    let mut ids: Vec<i64> = (0..2_000).collect();
    ids.push(3_000_000_000);
    let e = q(&db, "count", json!({ "where": { "version": { "in": ids } } })).await.unwrap_err();
    assert!(e.message().contains("Unable to fit integer value '3000000000' into an INT4"), "{e}");
}
