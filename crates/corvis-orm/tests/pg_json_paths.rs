//! JSON `path` filters on Postgres: the path is a bound `text[]` parameter, never SQL text.
//!
//! The tests that run queries use `CORVIS_ORM_PG` and the `orm_test_json_paths_*` databases (skipped
//! when the server is not reachable). `PROPTEST_CASES` overrides the case count of the property
//! (256 by default).

#[path = "common/pg.rs"]
mod pg;

use corvis_orm::OrmClientOptions;
use corvis_orm::prelude::*;
use proptest::prelude::*;
use proptest::test_runner::{FileFailurePersistence, TestCaseError, TestRunner};

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

/// Keys that break string literals, array literals and JSON path syntax.
const KEYS: [&str; 16] = [
    "a'b",
    "a\\b",
    "}",
    "{",
    "a,b",
    "a\"b",
    "{a,b}",
    "\u{65e5}\u{672c}\u{8a9e}",
    "'; DROP TABLE accounts; --",
    "a b",
    "",
    "\\\\",
    "\\\"",
    "$.a",
    "\"",
    ",",
];

/// Document `i` holds `KEYS[i]` at the top level and under `n`, plus a `list` that starts with it.
fn docs() -> Vec<Value> {
    KEYS.iter()
        .enumerate()
        .map(|(i, k)| {
            let mut top = serde_json::Map::new();
            top.insert((*k).to_string(), json!(i));
            top.insert("n".into(), json!({ *k: format!("v{i}") }));
            top.insert("list".into(), json!([*k, i]));
            Value::Object(top)
        })
        .collect()
}

async fn seeded(name: &str) -> Option<OrmClient> {
    let url = pg::fresh_db(&format!("json_paths_{name}")).await?;
    let db = OrmClient::with_options(OrmClientOptions::new(url).max_connections(2)).unwrap();
    db.db_push::<(Account,)>().await.unwrap();
    for (i, d) in docs().into_iter().enumerate() {
        q(&db, "create", json!({ "data": { "email": format!("u{i}@x"), "data": d } })).await.unwrap();
    }
    Some(db)
}

async fn ids(db: &OrmClient, filter: Value) -> Vec<i64> {
    let rows = q(db, "findMany", json!({ "where": filter, "orderBy": { "id": "asc" } })).await.unwrap_or_else(|e| panic!("{filter}: {e}"));
    rows.as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect()
}

/// A client that is never connected: enough to compile filters to SQL.
fn pg_client() -> OrmClient {
    OrmClient::new("postgres://u:p@127.0.0.1:5299/db").unwrap()
}

#[test]
fn generated_sql_binds_the_path_and_never_contains_it() {
    let db = pg_client();
    const OPS: [&str; 12] = [
        "equals",
        "not",
        "lt",
        "lte",
        "gt",
        "gte",
        "string_contains",
        "string_starts_with",
        "string_ends_with",
        "array_contains",
        "array_starts_with",
        "array_ends_with",
    ];
    for key in KEYS.iter().filter(|k| k.len() > 1) {
        for op in OPS {
            let lit = if op.starts_with("string") { json!("x") } else { json!(1) };
            for mode in [None, Some("insensitive")] {
                let mut f = json!({ "path": [key, "other"], op: lit });
                if let Some(m) = mode {
                    f["mode"] = json!(m);
                }
                let sql = corvis_orm::debug_sql(&db, meta(), &json!({ "data": f })).unwrap();
                assert!(!sql.contains(key), "{op} {mode:?}: the path leaked into the SQL: {sql}");
                assert!(sql.contains("::text[]"), "{op} {mode:?}: {sql}");
                assert!(!sql.contains("'{"), "{op} {mode:?}: {sql}");
            }
        }
        // The string form of a path (`$.a.b`) is split into segments and bound like the list form.
        let sql = corvis_orm::debug_sql(&db, meta(), &json!({ "data": { "path": format!("$.{key}.other"), "equals": 1 } })).unwrap();
        assert!(!sql.contains(key), "string path leaked into the SQL: {sql}");
        assert!(sql.contains("#> $1::text[]"), "{sql}");
    }
    let sql = corvis_orm::debug_sql(&db, meta(), &json!({ "data": { "path": ["a'b"], "equals": 1 } })).unwrap();
    assert!(sql.contains("#> $1::text[]"), "{sql}");
}

#[test]
fn a_filter_binds_its_path_once_whatever_the_number_of_operators() {
    let db = pg_client();
    let f = json!({ "path": ["a'b", "c"], "equals": 1, "not": 2, "lt": 3, "gt": 4, "string_contains": "x", "array_contains": 5 });
    let sql = corvis_orm::debug_sql(&db, meta(), &json!({ "data": f })).unwrap();
    // The path is `$1` everywhere; the six values are `$2` to `$7` and nothing else is bound.
    assert!(sql.matches("$1::text[]").count() >= 6, "{sql}");
    assert!(sql.contains("$7") && !sql.contains("$8"), "{sql}");
    assert_eq!(sql.matches("::text[]").count(), sql.matches("$1::text[]").count(), "{sql}");
    // Two filters of one statement each bind their own path.
    let w = json!({ "OR": [{ "data": { "path": ["x"], "equals": 1, "not": 2 } }, { "data": { "path": ["y"], "equals": 3, "not": 4 } }] });
    let sql = corvis_orm::debug_sql(&db, meta(), &w).unwrap();
    assert!(sql.contains("$1::text[]") && sql.contains("$4::text[]") && !sql.contains("$7"), "{sql}");
}

#[test]
fn paths_over_the_segment_cap_are_a_validation_error_on_both_dialects() {
    let clients = [pg_client(), OrmClient::new("sqlite::memory:").unwrap()];
    for db in &clients {
        let at = |n: usize| json!({ "data": { "path": vec![""; n], "equals": 1, "not": 2 } });
        assert!(corvis_orm::debug_sql(db, meta(), &at(128)).is_ok());
        let e = corvis_orm::debug_sql(db, meta(), &at(129)).unwrap_err();
        assert!(e.message().contains("at most 128 path segments"), "{e}");
        let e = corvis_orm::debug_sql(db, meta(), &at(100_000)).unwrap_err();
        assert!(e.message().contains("at most 128 path segments"), "{e}");
        // The string form too: 129 segments of `a`.
        let s = format!("$.{}", vec!["a"; 129].join("."));
        let e = corvis_orm::debug_sql(db, meta(), &json!({ "data": { "path": s, "equals": 1 } })).unwrap_err();
        assert!(e.message().contains("at most 128 path segments"), "{e}");
    }
}

#[tokio::test]
async fn hostile_path_segments_find_exactly_their_row() {
    let Some(db) = seeded("exact").await else { return };
    for (i, k) in KEYS.iter().enumerate() {
        let id = i as i64 + 1;
        assert_eq!(ids(&db, json!({ "data": { "path": [k], "equals": i } })).await, [id], "key {k:?}");
        assert_eq!(ids(&db, json!({ "data": { "path": ["n", k], "equals": format!("v{i}") } })).await, [id], "key {k:?}");
        assert_eq!(ids(&db, json!({ "data": { "path": ["n", k], "string_starts_with": "v" } })).await, [id], "key {k:?}");
        assert!(ids(&db, json!({ "data": { "path": ["list", "0"], "equals": k } })).await.contains(&id), "key {k:?}");
        // The other rows lack the key, so `not` matches nothing (NULL) and never errors.
        assert_eq!(ids(&db, json!({ "data": { "path": [k], "not": i } })).await, Vec::<i64>::new(), "key {k:?}");
    }
}

#[tokio::test]
async fn paths_are_not_parsed_as_array_literals_or_text() {
    let Some(db) = seeded("literal").await else { return };
    // `{a,b}` is one key, not the two keys `a` and `b`.
    assert_eq!(ids(&db, json!({ "data": { "path": ["{a,b}"], "equals": 6 } })).await, [7]);
    assert_eq!(ids(&db, json!({ "data": { "path": ["a", "b"], "not": 0 } })).await, Vec::<i64>::new());
    // A backslash is a backslash, not an escape of the next character.
    assert_eq!(ids(&db, json!({ "data": { "path": ["\\\""], "equals": 12 } })).await, [13]);
    assert_eq!(ids(&db, json!({ "data": { "path": ["\\\\"], "equals": 11 } })).await, [12]);
    // `$.a` is a key, not a JSONPath.
    assert_eq!(ids(&db, json!({ "data": { "path": ["$.a"], "equals": 13 } })).await, [14]);
    // The empty path is the whole document and the empty key is a key.
    assert_eq!(ids(&db, json!({ "data": { "path": [], "not": { "x": 1 } } })).await.len(), KEYS.len());
    assert_eq!(ids(&db, json!({ "data": { "path": [""], "equals": 10 } })).await, [11]);
    // The table is intact after a statement-shaped path.
    assert_eq!(ids(&db, json!({ "data": { "path": ["'; DROP TABLE accounts; --"], "equals": 8 } })).await, [9]);
    assert_eq!(q(&db, "count", json!({})).await.unwrap(), json!(KEYS.len()));
}

#[tokio::test]
async fn a_shared_path_parameter_works_with_every_operator_on_postgres() {
    let Some(db) = seeded("shared").await else { return };
    // Five operators on one path: the one `text[]` parameter is used by all of them.
    let f = json!({ "path": ["n", "a'b"], "not": "zz", "string_starts_with": "v", "string_ends_with": "0", "string_contains": "v", "gte": "v0" });
    assert_eq!(ids(&db, json!({ "data": f })).await, [1]);
    let f = json!({ "path": ["list"], "array_starts_with": "}", "array_contains": "}", "not": [] });
    assert_eq!(ids(&db, json!({ "data": f })).await, [3]);
    // Two filters in one statement bind two paths.
    let w = json!({ "OR": [{ "data": { "path": ["a'b"], "equals": 0, "not": 9 } }, { "data": { "path": ["{"], "equals": 3, "not": 9 } }] });
    assert_eq!(ids(&db, w).await, [1, 4]);
    // Over the cap: a validation error before any query runs.
    let e = q(&db, "findMany", json!({ "where": { "data": { "path": vec![""; 129], "equals": 1 } } })).await.unwrap_err();
    assert!(e.message().contains("at most 128 path segments"), "{e}");
}

fn key() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => prop::sample::select(KEYS.to_vec()).prop_map(String::from),
        2 => prop::sample::select(vec!["n", "list", "0", "1", "missing"]).prop_map(String::from),
        2 => "[a-z'\"\\\\{},$. \\u{65e5}]{0,4}",
    ]
}

/// The value at `path` in a document, with the rules of `#>` (numeric segments index arrays).
fn value_at<'a>(doc: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut cur = doc;
    for seg in path {
        cur = match cur {
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            Value::Object(o) => o.get(seg)?,
            _ => return None,
        };
    }
    Some(cur)
}

#[test]
fn json_path_filters_match_a_rust_evaluation_on_postgres() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let Some(db) = rt.block_on(seeded("property")) else { return };
    let docs = docs();
    let lits: Vec<Value> = (0..KEYS.len())
        .map(|i| json!(i))
        .chain((0..KEYS.len()).map(|i| json!(format!("v{i}"))))
        .chain(KEYS.iter().map(|k| json!(k)))
        .collect();
    let strategy =
        (prop::collection::vec(key(), 0..4), prop::sample::select(vec!["equals", "not"]), prop::sample::select(lits), any::<bool>());
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(256);
    let config = ProptestConfig {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct("proptest-regressions/pg_json_paths.txt"))),
        ..ProptestConfig::default()
    };
    let mut runner = TestRunner::new(config);
    let result = runner.run(&strategy, |(path, op, lit, negate)| {
        let filter = json!({ "data": { "path": path, op: lit } });
        let filter = if negate { json!({ "NOT": filter }) } else { filter };
        let rows = rt
            .block_on(q(&db, "findMany", json!({ "where": filter, "orderBy": { "id": "asc" } })))
            .map_err(|e| TestCaseError::fail(format!("{filter}: {e}")))?;
        let got: Vec<i64> = rows.as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect();
        let want: Vec<i64> = docs
            .iter()
            .enumerate()
            // SQL three-valued logic: a missing path matches neither the filter nor its negation.
            .filter(|(_, d)| value_at(d, &path).map(|v| ((v == &lit) == (op == "equals")) != negate) == Some(true))
            .map(|(i, _)| i as i64 + 1)
            .collect();
        prop_assert_eq!(got, want, "{}", filter);
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}");
    }
    assert_eq!(rt.block_on(q(&db, "count", json!({}))).unwrap(), json!(KEYS.len()));
}
