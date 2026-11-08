//! The `schema` parameter of the database URL, replayed against the fixture recorded with the
//! original client (`tests/fixtures/schema_url.json`, script `corvis-tests/tmp/orm/schema/cases.js`).
//! The Postgres tests are skipped without a server (see `common/pg.rs`).

#[path = "common/pg.rs"]
mod pg;

use std::path::PathBuf;

use corvis_orm::prelude::*;

/// A model in an explicit schema: it keeps `"audit"."events"` whatever the URL schema is.
#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "events", schema = "audit")]
pub struct Event {
    #[id]
    pub id: i64,
    pub label: String,
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/schema_url.json")).unwrap()
}

fn with_query(url: &str, q: &str) -> String {
    if q.is_empty() { url.to_string() } else { format!("{url}?{q}") }
}

fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schema_migrations")
}

/// The database of the test, with `public."User"` and `other."User"` holding different rows.
async fn two_schemas(name: &str) -> Option<String> {
    let base = pg::fresh_db(name).await?;
    let admin = OrmClient::new(&base).unwrap();
    for sql in [
        "CREATE SCHEMA other",
        r#"CREATE TABLE public."User" (id int primary key, email text not null)"#,
        r#"CREATE TABLE other."User" (id int primary key, email text not null)"#,
        r#"INSERT INTO public."User" VALUES (1, 'a@x')"#,
        r#"INSERT INTO other."User" VALUES (2, 'b@x')"#,
    ] {
        admin.execute_raw(sql, vec![]).await.unwrap();
    }
    admin.disconnect().await.unwrap();
    Some(base)
}

async fn first_string(db: &OrmClient, sql: &str, key: &str) -> Value {
    db.query_raw(sql, vec![]).await.unwrap()[0][key].clone()
}

async fn emails(db: &OrmClient) -> Vec<String> {
    db.query_raw(r#"SELECT email FROM "User" ORDER BY id"#, vec![])
        .await
        .unwrap()
        .iter()
        .map(|r| r["email"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn queries_resolve_in_the_url_schema_like_the_original() {
    let Some(base) = two_schemas("schema_queries").await else { return };
    let fx = fixture();
    for (key, rec) in fx["queries"].as_object().unwrap() {
        let q = if key == "none" { String::new() } else { key.clone() };
        let db = OrmClient::new(with_query(&base, &q)).unwrap();
        let got = emails(&db).await;
        let want: Vec<String> = rec["raw_unqualified"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(got, want, "`{key}`: unqualified raw SQL");
        let p = db.query_raw("SELECT current_schema() AS cs, current_setting('search_path') AS sp", vec![]).await.unwrap();
        assert_eq!(p[0]["cs"], rec["current_schema"], "`{key}`");
        // The original reports `"other"` where the server prints `other` (quotes only when needed): same path.
        let unquoted = |v: &Value| v.as_str().unwrap().replace('"', "");
        assert_eq!(unquoted(&p[0]["sp"]), unquoted(&rec["search_path"]), "`{key}`: the path is exactly the one schema (or the server's)");
        db.disconnect().await.unwrap();
    }
    // The recorded rows come from the generated SQL, which the original qualifies with the schema.
    assert_eq!(fx["queries"]["schema=other"]["find_many"], json!(["b@x"]));
}

#[tokio::test]
async fn generated_queries_use_the_url_schema() {
    // `plain_rows` exists with different rows in both schemas; the model delegate reads the URL schema's.
    let Some(base) = pg::fresh_db("schema_models").await else { return };
    let admin = OrmClient::new(&base).unwrap();
    admin.execute_raw("CREATE SCHEMA other", vec![]).await.unwrap();
    admin.disconnect().await.unwrap();
    for (schema, label) in [("public", "a@x"), ("other", "b@x")] {
        let db = OrmClient::new(with_query(&base, &format!("schema={schema}"))).unwrap();
        db.execute_raw("CREATE TABLE plain_rows (id bigint primary key, label text not null)", vec![]).await.unwrap();
        db.execute_raw("INSERT INTO plain_rows VALUES (1, $1)", vec![json!(label)]).await.unwrap();
        db.disconnect().await.unwrap();
    }
    let db = OrmClient::new(with_query(&base, "schema=other")).unwrap();
    let rows: Vec<Plain> = db.model::<Plain>().find_many(json!({})).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "b@x");
    let created: Plain = db.model::<Plain>().create(json!({ "data": { "id": 2, "label": "c@x" } })).await.unwrap();
    assert_eq!(created.id, 2);
    db.disconnect().await.unwrap();
    let admin = OrmClient::new(&base).unwrap();
    let public = admin.query_raw("SELECT label FROM public.plain_rows", vec![]).await.unwrap();
    assert_eq!(public.len(), 1, "public.plain_rows is untouched");
    admin.disconnect().await.unwrap();
}

#[tokio::test]
async fn raw_queries_transactions_and_every_pooled_connection() {
    let Some(base) = two_schemas("schema_conns").await else { return };
    let fx = fixture();
    let db = OrmClient::new(with_query(&base, "schema=other&connection_limit=3")).unwrap();
    assert_eq!(first_string(&db, "SELECT current_schema() AS cs", "cs").await, json!("other"));
    let in_tx = db
        ._transaction(|tx| async move { tx.query_raw("SELECT current_schema() AS cs", vec![]).await.map(|r| r[0]["cs"].clone()) })
        .await
        .unwrap_or_else(|e: OrmError| panic!("{e}"));
    assert_eq!(in_tx, fx["transaction_current_schema"]);
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move { db.query_raw("SELECT current_schema() AS cs, pg_sleep(0.2)::text AS s", vec![]).await }));
    }
    let want = fx["concurrent_current_schema"].as_array().unwrap();
    assert_eq!(want.len(), 6);
    for t in tasks {
        assert_eq!(t.await.unwrap().unwrap()[0]["cs"], want[0]);
    }
    db.disconnect().await.unwrap();
}

#[tokio::test]
async fn explicit_model_schema_wins() {
    let Some(base) = pg::fresh_db("schema_explicit").await else { return };
    let admin = OrmClient::new(&base).unwrap();
    for sql in [
        "CREATE SCHEMA audit",
        "CREATE SCHEMA other",
        "CREATE TABLE audit.events (id bigint primary key, label text not null)",
        "INSERT INTO audit.events VALUES (1, 'x')",
    ] {
        admin.execute_raw(sql, vec![]).await.unwrap();
    }
    admin.disconnect().await.unwrap();
    let db = OrmClient::new(with_query(&base, "schema=other")).unwrap();
    let events: Vec<Event> = db.model::<Event>().find_many(json!({})).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].label, "x");
    db.disconnect().await.unwrap();
}

#[tokio::test]
async fn no_parameter_keeps_the_server_search_path() {
    let Some(base) = pg::fresh_db("schema_default").await else { return };
    let db = OrmClient::new(&base).unwrap();
    assert_eq!(first_string(&db, "SHOW search_path", "search_path").await, json!("\"$user\", public"));
    db.disconnect().await.unwrap();
}

#[tokio::test]
async fn tls_parameter_is_kept_and_schema_is_removed() {
    let Some(base) = two_schemas("schema_tls").await else { return };
    let db = OrmClient::new(with_query(&base, "sslmode=disable&schema=other")).unwrap();
    assert_eq!(db.options().url, with_query(&base, "sslmode=disable&schema=other"));
    assert_eq!(first_string(&db, "SELECT current_schema() AS cs", "cs").await, json!("other"));
    db.disconnect().await.unwrap();
}

#[tokio::test]
async fn a_missing_schema_fails_to_find_tables_and_is_not_created_by_queries() {
    let Some(base) = pg::fresh_db("schema_missing").await else { return };
    let db = OrmClient::new(with_query(&base, "schema=nope")).unwrap();
    let e = db.query_raw(r#"SELECT * FROM "User""#, vec![]).await.unwrap_err();
    assert!(e.to_string().contains("User"), "{e}");
    let admin = OrmClient::new(&base).unwrap();
    let rows = admin.query_raw("SELECT 1 AS one FROM pg_namespace WHERE nspname = 'nope'", vec![]).await.unwrap();
    assert!(rows.is_empty(), "queries do no DDL");
    db.disconnect().await.unwrap();
    admin.disconnect().await.unwrap();
}

async fn catalog(base: &str) -> (Vec<String>, Vec<String>) {
    let admin = OrmClient::new(base).unwrap();
    let schemas = admin
        .query_raw("SELECT nspname FROM pg_namespace WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' ORDER BY 1", vec![])
        .await
        .unwrap();
    let tables = admin
        .query_raw(
            "SELECT table_schema || '.' || table_name AS t FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY 1",
            vec![],
        )
        .await
        .unwrap();
    admin.disconnect().await.unwrap();
    let col = |rows: Vec<Value>, k: &str| rows.iter().map(|r| r[k].as_str().unwrap().to_string()).collect::<Vec<_>>();
    (col(schemas, "nspname"), col(tables, "t"))
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn migrations_apply_inside_a_missing_schema_like_the_original() {
    let fx = fixture();
    let local = ["20260101000000_init", "20260102000000_more"].map(String::from).to_vec();
    for (label, q) in [("tenant_a", "schema=tenant_a"), ("My Schema", "schema=My%20Schema")] {
        let Some(base) = pg::fresh_db("schema_migrate").await else { return };
        let rec = &fx["migrate"][label];
        let db = OrmClient::new(with_query(&base, q)).unwrap();
        // `migrate status` creates the schema too (recorded), without applying anything.
        let st = db.migrate_status(migrations_dir()).await.unwrap();
        assert_eq!(st.pending, local, "{label}");
        let (schemas, _) = catalog(&base).await;
        assert!(rec["catalog_after_status"]["schemas"].as_array().unwrap().iter().any(|s| s == label));
        assert!(schemas.iter().any(|s| s == label), "{label}: {schemas:?}");

        assert_eq!(db.migrate_deploy(migrations_dir()).await.unwrap(), local, "{label}");
        let (_, tables) = catalog(&base).await;
        // The recorded tables, minus the canary the script created in `public`.
        let want: Vec<String> = names(&rec["catalog_after_deploy"]["tables"]).into_iter().filter(|t| t != "public.canary").collect();
        assert_eq!(tables, want, "{label}");
        let st = db.migrate_status(migrations_dir()).await.unwrap();
        assert!(st.pending.is_empty() && st.applied == local, "{label}: up to date");
        // Another schema of the same database has no history.
        let other = OrmClient::new(with_query(&base, "schema=public")).unwrap();
        assert_eq!(other.migrate_status(migrations_dir()).await.unwrap().pending, local, "{label}");
        other.disconnect().await.unwrap();
        db.disconnect().await.unwrap();
    }
}

#[tokio::test]
async fn db_push_creates_the_schema() {
    let Some(base) = pg::fresh_db("schema_push").await else { return };
    let db = OrmClient::new(with_query(&base, "schema=pushed")).unwrap();
    db.db_push::<Plain>().await.unwrap();
    let (schemas, tables) = catalog(&base).await;
    assert!(schemas.iter().any(|s| s == "pushed"));
    assert_eq!(tables, ["pushed.plain_rows"]);
    db.disconnect().await.unwrap();
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "plain_rows")]
pub struct Plain {
    #[id]
    pub id: i64,
    pub label: String,
}

#[tokio::test]
async fn hostile_schema_name_is_only_ever_an_identifier() {
    let Some(base) = pg::fresh_db("schema_hostile").await else { return };
    let name = r#"x"; DROP TABLE canary; --"#;
    let admin = OrmClient::new(&base).unwrap();
    admin.execute_raw("CREATE TABLE public.canary (id int)", vec![]).await.unwrap();
    let encoded = "x%22%3B%20DROP%20TABLE%20canary%3B%20--";
    let db = OrmClient::new(with_query(&base, &format!("schema={encoded}"))).unwrap();
    db.migrate_deploy(migrations_dir()).await.unwrap();
    assert_eq!(first_string(&db, "SELECT current_schema() AS cs", "cs").await, json!(name));
    let (schemas, tables) = catalog(&base).await;
    assert!(schemas.iter().any(|s| s == name), "{schemas:?}");
    assert!(tables.contains(&"public.canary".to_string()), "{tables:?}");
    assert!(tables.contains(&format!("{name}.Item")), "{tables:?}");
    // No other setting changes either.
    assert_eq!(first_string(&db, "SHOW statement_timeout", "statement_timeout").await, json!("0"));
    db.disconnect().await.unwrap();
    admin.disconnect().await.unwrap();
}

#[tokio::test]
async fn role_without_database_create_can_deploy_into_an_existing_schema() {
    let Some(base) = pg::fresh_db("schema_role").await else { return };
    let db_name = base.rsplit_once('/').unwrap().1.to_string();
    let admin = OrmClient::new(&base).unwrap();
    let role = "orm_test_schema_role";
    for sql in [format!("DROP OWNED BY {role}"), format!("DROP ROLE IF EXISTS {role}")] {
        // `DROP OWNED` fails when the role does not exist yet.
        let _ = admin.execute_raw(&sql, vec![]).await;
    }
    for sql in [
        format!("CREATE ROLE {role} LOGIN PASSWORD 'pw'"),
        format!("REVOKE CREATE ON DATABASE \"{db_name}\" FROM PUBLIC"),
        format!("REVOKE CREATE ON DATABASE \"{db_name}\" FROM {role}"),
        "CREATE SCHEMA app".to_string(),
        format!("GRANT USAGE, CREATE ON SCHEMA app TO {role}"),
    ] {
        admin.execute_raw(&sql, vec![]).await.unwrap();
    }
    let url = format!("postgresql://{role}:pw@{}/{db_name}", pg::host(&base));
    let rec = &fixture()["migrate_role_without_database_create"];
    assert_eq!(rec["existing_schema"]["exit"], json!(0));

    let db = OrmClient::new(with_query(&url, "schema=app")).unwrap();
    let applied = db.migrate_deploy(migrations_dir()).await.unwrap();
    assert_eq!(applied.len(), 2);
    let (_, tables) = catalog(&base).await;
    assert!(tables.contains(&"app.Item".to_string()) && tables.contains(&"app._prisma_migrations".to_string()), "{tables:?}");
    db.disconnect().await.unwrap();

    // A missing schema needs the database privilege: the original fails as well (recorded).
    assert_eq!(rec["missing_schema"]["exit"], json!(1));
    let db = OrmClient::new(with_query(&url, "schema=never_created")).unwrap();
    assert!(db.migrate_deploy(migrations_dir()).await.is_err());
    db.disconnect().await.unwrap();

    let _ = admin.execute_raw(&format!("DROP OWNED BY {role}"), vec![]).await;
    admin.execute_raw(&format!("DROP ROLE {role}"), vec![]).await.unwrap();
    admin.disconnect().await.unwrap();
}

#[tokio::test]
async fn invalid_values_are_rejected_where_the_original_misbehaves() {
    // Recorded: the original accepts an empty name (search_path becomes `""`, no table resolves) and
    // names over 63 bytes (the server truncates them), and builds an invalid search_path from a hostile name.
    // corvis-orm rejects the first two at creation (design: a silent truncation could point two
    // applications at one schema) and quotes the third; see design.md.
    let fx = fixture();
    assert_eq!(fx["invalid"]["empty"]["search_path"], json!("\"\""));
    assert_eq!(fx["invalid"]["len64"]["ok"], json!(true));
    assert!(fx["invalid"]["nul"]["error"].is_object(), "the original cannot connect with a NUL in the schema");
    for q in ["schema=", "schema=a%00b", &format!("schema={}", "a".repeat(64))] {
        let e = OrmClient::new(format!("postgresql://app:secret@127.0.0.1:1/db?{q}")).unwrap_err();
        assert_eq!(e.code(), Some("P1013"), "{q}");
        assert!(e.to_string().contains("schema") && !e.to_string().contains("secret"), "{q}: {e}");
    }
    assert!(OrmClient::new(format!("postgresql://h/db?schema={}", "a".repeat(63))).is_ok());
}

#[tokio::test]
async fn sqlite_accepts_the_parameter_like_the_original() {
    let fx = fixture();
    for (q, rec) in fx["sqlite"].as_object().unwrap() {
        assert_eq!(rec["ok"], json!(true), "`{q}`");
    }
    let db = OrmClient::new("file::memory:?schema=main").unwrap();
    db.query_raw("SELECT 1", vec![]).await.unwrap();
    db.disconnect().await.unwrap();
}
