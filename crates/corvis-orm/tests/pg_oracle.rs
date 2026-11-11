//! Differential test against the reference JavaScript client (PostgreSQL).
//!
//! `tests/fixtures/oracle/results.json` was recorded by running every case with the reference
//! client (`corvis-tests/tmp/orm/oracle/full/cases.js`) on the schema
//! `tests/fixtures/oracle/schema.prisma`. Here the same cases run, in the same order,
//! through corvis-orm with the models generated from that schema, and every result
//! (or error code / meta / message) must be identical.

#[path = "common/pg.rs"]
mod pg;

#[path = "common/oracle.rs"]
mod compare;

#[allow(dead_code)]
mod oracle {
    include!("generated/oracle.rs");
}

use corvis_orm::{DateTime, Decimal, Model, ModelMeta, OrmClient, RawSql, ToSqlArg};
use serde_json::Value;

fn meta_of(model: &str) -> &'static ModelMeta {
    match model {
        "user" => oracle::User::meta(),
        "profile" => oracle::Profile::meta(),
        "post" => oracle::Post::meta(),
        "comment" => oracle::Comment::meta(),
        "category" => oracle::Category::meta(),
        "group" => oracle::Group::meta(),
        "wallet" => oracle::Wallet::meta(),
        "pair" => oracle::Pair::meta(),
        "entry" => oracle::Entry::meta(),
        "rate" => oracle::Rate::meta(),
        other => panic!("unknown model {other}"),
    }
}

/// `[strings, ...values]` of a tagged `sql` case; `{ $date }`, `{ $decimal }` and `{ $json }` mark typed values.
fn typed_sql(args: &[Value]) -> RawSql {
    let strings: Vec<String> = args[0].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
    let values = args[1..]
        .iter()
        .map(|v| match v {
            Value::Object(o) if o.contains_key("$date") => DateTime::parse(o["$date"].as_str().unwrap()).unwrap().to_sql_arg(),
            Value::Object(o) if o.contains_key("$decimal") => Decimal::parse(o["$decimal"].as_str().unwrap()).unwrap().to_sql_arg(),
            Value::Object(o) if o.contains_key("$json") => o["$json"].to_sql_arg(),
            Value::Array(a) => a.iter().map(|x| x.as_i64().unwrap()).collect::<Vec<i64>>().to_sql_arg(),
            Value::Number(n) if n.is_i64() => n.as_i64().unwrap().to_sql_arg(),
            Value::Number(n) => n.as_f64().unwrap().to_sql_arg(),
            other => other.to_sql_arg(),
        })
        .collect();
    RawSql::new(strings, values)
}

#[tokio::test]
async fn matches_reference_client() {
    let Some(url) = pg::fresh_db("oracle_rs").await else { return };
    let db = OrmClient::new(url).unwrap();
    db.db_execute(include_str!("fixtures/oracle/migration.sql")).await.unwrap();
    run_cases(&db).await;
}

/// The same cases on a database built by `db push` of the generated models, with the oracle's block of extras (a
/// `CHECK` and a trigger written by hand after the migration) applied after the push.
#[tokio::test]
async fn matches_reference_client_on_a_pushed_database() {
    let Some((url, _drop)) = pg::fresh_db_bounded("oracle_rs_pushed", 30).await else { return };
    let db = OrmClient::new(url).unwrap();
    db.db_push::<(
        oracle::User,
        oracle::Profile,
        oracle::Post,
        oracle::Comment,
        oracle::Category,
        oracle::Group,
        oracle::Wallet,
        oracle::Pair,
        oracle::Entry,
        oracle::Rate,
    )>()
    .await
    .unwrap();
    let migration = include_str!("fixtures/oracle/migration.sql");
    let extras = &migration[migration.find("-- Oracle extras").expect("the migration has its block of extras")..];
    db.db_execute(extras).await.unwrap();
    run_cases(&db).await;
}

async fn run_cases(db: &OrmClient) {
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/oracle/results.json")).unwrap();
    let only = std::env::var("ORACLE_ONLY").ok();
    let mut failures = Vec::new();
    for c in &cases {
        let label = c["label"].as_str().unwrap();
        let model = c["model"].as_str().unwrap();
        let action = c["action"].as_str().unwrap();
        let res = if model == "$raw" && matches!(action, "queryRaw" | "executeRaw") {
            let sql = typed_sql(c["args"].as_array().unwrap());
            if action == "queryRaw" { db._query_raw(sql).await.map(Value::from) } else { db._execute_raw(sql).await.map(Value::from) }
        } else if model == "$raw" {
            let a = c["args"].as_array().unwrap();
            let sql = a[0].as_str().unwrap();
            let params = a[1..].to_vec();
            if action == "queryRawUnsafe" {
                db._query_raw_unsafe(sql, params).await.map(Value::from)
            } else {
                db._execute_raw_unsafe(sql, params).await.map(Value::from)
            }
        } else {
            let mut args = c["args"].clone();
            compare::revive(&mut args);
            db.query_json(meta_of(model), action, args).await
        };
        if only.as_deref().is_some_and(|o| !label.starts_with(o)) {
            continue;
        }
        failures.extend(compare::compare(c, res));
    }
    if !failures.is_empty() {
        panic!("{} of {} cases differ from the reference client:\n{}", failures.len(), cases.len(), failures.join("\n"));
    }
}
