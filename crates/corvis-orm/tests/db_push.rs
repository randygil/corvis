//! What `db push` creates: the schema of the original client's migration for the same models (keys, join
//! tables, enum types, native types, scalar lists, indexes and names), what it refuses, and that it is atomic.
//!
//! The models are those of the Postgres oracle (`tests/generated/oracle.rs`, generated from
//! `tests/fixtures/oracle/schema.prisma`, whose reference migration is `tests/fixtures/oracle/migration.sql`).

#[path = "common/pg.rs"]
mod pg;

#[allow(dead_code)]
mod oracle {
    include!("generated/oracle.rs");
}

use corvis_orm::ModelSet;
use corvis_orm::prelude::*;
use oracle::{Category, Comment, Entry, Group, Pair, Post, Profile, Rate, User, Wallet};

type OracleModels = (User, Profile, Post, Comment, Category, Group, Wallet, Pair, Entry, Rate);

async fn sqlite<S: ModelSet>() -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<S>().await.unwrap();
    db
}

/// A database pushed with `S`, and the guard that drops it when the test ends.
async fn postgres<S: ModelSet>(name: &str) -> Option<(pg::DropOnExit, OrmClient)> {
    let (url, guard) = pg::fresh_db_bounded(&format!("push_{name}"), 30).await?;
    let db = OrmClient::new(url).unwrap();
    db.db_push::<S>().await.unwrap();
    Some((guard, db))
}

async fn q(db: &OrmClient, model: &'static corvis_orm::ModelMeta, action: &str, args: Value) -> corvis_orm::Result<Value> {
    db.query_json(model, action, args).await
}

fn meta<M: corvis_orm::Model>() -> &'static corvis_orm::ModelMeta {
    M::meta()
}

// ---------------------------------------------------------------------------------------------
// Keys and join tables (both dialects)
// ---------------------------------------------------------------------------------------------

#[test]
fn compound_primary_key_covers_every_field() {
    for url in ["sqlite::memory:", "postgres://u:p@127.0.0.1:1/db"] {
        let db = OrmClient::new(url).unwrap();
        let ddl = db.schema_sql::<Pair>().unwrap().join("\n");
        assert!(ddl.contains(r#"CONSTRAINT "Pair_pkey" PRIMARY KEY ("a","b")"#), "{url}:\n{ddl}");
    }
}

async fn two_pairs_share_a(db: &OrmClient) {
    let pair = meta::<Pair>();
    q(db, pair, "create", json!({ "data": { "a": "x", "b": 1, "label": "one" } })).await.unwrap();
    q(db, pair, "create", json!({ "data": { "a": "x", "b": 2, "label": "two" } })).await.unwrap();
    assert_eq!(q(db, pair, "count", json!({})).await.unwrap(), json!(2));
}

async fn nested_m2m_create(db: &OrmClient) {
    let post = q(
        db,
        meta::<Post>(),
        "create",
        json!({ "data": { "title": "Hola", "categories": { "create": [{ "name": "tech" }] } }, "include": { "categories": true } }),
    )
    .await
    .unwrap();
    assert_eq!(post["categories"][0]["name"], "tech", "{post}");
}

#[tokio::test]
async fn compound_primary_key_sqlite() {
    two_pairs_share_a(&sqlite::<Pair>().await).await;
}

#[tokio::test]
async fn implicit_many_to_many_sqlite() {
    nested_m2m_create(&sqlite::<(User, Post, Category)>().await).await;
}

#[tokio::test]
async fn compound_primary_key_postgres() {
    let Some((_drop, db)) = postgres::<Pair>("pk").await else { return };
    two_pairs_share_a(&db).await;
}

#[tokio::test]
async fn implicit_many_to_many_postgres() {
    let Some((_drop, db)) = postgres::<(User, Post, Category)>("m2m").await else { return };
    nested_m2m_create(&db).await;
}

/// Every oracle model in one push, then an entry that references a pair by its compound key.
async fn all_oracle_models(db: &OrmClient) {
    two_pairs_share_a(db).await;
    nested_m2m_create(db).await;
    let user = q(db, meta::<User>(), "create", json!({ "data": { "email": "w@x.io" } })).await.unwrap();
    let wallet_id = "1f6a3c2e-6c55-4a43-9a2a-0d3c4a1b2c3d";
    q(db, meta::<Wallet>(), "create", json!({ "data": { "id": wallet_id, "userId": user["id"], "currency": "VES", "amount": 2 } }))
        .await
        .unwrap();
    let entry = q(
        db,
        meta::<Entry>(),
        "create",
        json!({ "data": { "walletId": wallet_id, "pairA": "x", "pairB": 2, "amount": 1 }, "include": { "pair": true } }),
    )
    .await
    .unwrap();
    assert_eq!(entry["pair"]["label"], "two", "{entry}");
}

#[tokio::test]
async fn all_oracle_models_sqlite() {
    all_oracle_models(&sqlite::<OracleModels>().await).await;
}

#[tokio::test]
async fn all_oracle_models_postgres() {
    let Some((_drop, db)) = postgres::<OracleModels>("all").await else { return };
    all_oracle_models(&db).await;
}

// ---------------------------------------------------------------------------------------------
// Postgres types: enums, native types, scalar lists
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn enum_types_postgres() {
    let Some((_drop, db)) = postgres::<User>("enum").await else { return };
    let user = q(&db, meta::<User>(), "create", json!({ "data": { "email": "a@x.io", "role": "ADMIN", "roles": ["GUEST", "MEMBER"] } }))
        .await
        .unwrap();
    assert_eq!((user["role"].clone(), user["roles"].clone()), (json!("ADMIN"), json!(["GUEST", "MEMBER"])));
    let found = q(&db, meta::<User>(), "findMany", json!({ "where": { "role": "ADMIN", "roles": { "has": "MEMBER" } } })).await.unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1);
    // The default is the mapped value of `MEMBER`.
    let user = q(&db, meta::<User>(), "create", json!({ "data": { "email": "b@x.io" } })).await.unwrap();
    assert_eq!(user["role"], "MEMBER");
}

#[tokio::test]
async fn native_types_and_scalar_lists_postgres() {
    let Some((_drop, db)) = postgres::<(User, Wallet)>("native").await else { return };
    let user = q(
        &db,
        meta::<User>(),
        "create",
        json!({ "data": { "email": "a@x.io", "scores": [1, 2], "tags": ["t"], "birthday": "1990-05-06T13:00:00.000Z" } }),
    )
    .await
    .unwrap();
    assert_eq!((user["scores"].clone(), user["birthday"].clone()), (json!([1, 2]), json!("1990-05-06T00:00:00.000Z")));
    let wallet_id = "8f6a3c2e-6c55-4a43-9a2a-0d3c4a1b2c3d";
    q(&db, meta::<Wallet>(), "create", json!({ "data": { "id": wallet_id, "userId": user["id"], "currency": "USD", "amount": "1.5" } }))
        .await
        .unwrap();
    let found = q(&db, meta::<Wallet>(), "findMany", json!({ "where": { "id": wallet_id } })).await.unwrap();
    assert_eq!(found[0]["amount"], "1.5");
    let found = q(&db, meta::<User>(), "findMany", json!({ "where": { "scores": { "hasSome": [2, 9] } } })).await.unwrap();
    assert_eq!(found.as_array().unwrap().len(), 1);
    let types: Vec<Value> = db
        .query_raw(
            "SELECT column_name::text AS name, udt_name::text AS udt FROM information_schema.columns \
             WHERE table_name = 'users' AND column_name IN ('scores', 'roles', 'birthday', 'created_at') ORDER BY column_name",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(
        Value::Array(types),
        json!([
            { "name": "birthday", "udt": "date" },
            { "name": "created_at", "udt": "timestamp" },
            { "name": "roles", "udt": "_role_t" },
            { "name": "scores", "udt": "_int4" }
        ])
    );
}

#[tokio::test]
async fn uuid_filter_postgres() {
    let Some((_drop, db)) = postgres::<Group>("uuid").await else { return };
    let id = "5b0f2b8e-2f0a-4a8e-8f8e-1c2d3e4f5a6b";
    q(&db, meta::<Group>(), "create", json!({ "data": { "id": id, "name": "g" } })).await.unwrap();
    let found = q(&db, meta::<Group>(), "findMany", json!({ "where": { "id": id } })).await.unwrap();
    assert_eq!(found[0]["name"], "g");
}

// ---------------------------------------------------------------------------------------------
// Refusals, atomicity, quoting, existing tables
// ---------------------------------------------------------------------------------------------

mod edge {
    use corvis_orm::prelude::*;

    #[derive(Debug, Clone, Serialize, Deserialize, Model)]
    #[model(table = "first")]
    pub struct First {
        #[id]
        #[default(autoincrement)]
        pub id: i32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Model)]
    #[model(table = "second")]
    pub struct Second {
        #[id]
        #[default(autoincrement)]
        pub id: i32,
    }

    /// Its `CREATE TABLE` fails on both databases: the default calls a function that does not exist (and is not
    /// parenthesized, which SQLite requires of an expression).
    #[derive(Debug, Clone, Serialize, Deserialize, Model)]
    #[model(table = "third")]
    pub struct Third {
        #[id]
        #[default(autoincrement)]
        pub id: i32,
        #[default(dbgenerated("no_such_function()"))]
        pub code: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Model)]
    #[model(table = "odd_type")]
    pub struct OddType {
        #[id]
        #[default(autoincrement)]
        pub id: i32,
        #[db("Geometry(4326)")]
        pub area: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Model)]
    #[model(table = "odd_index", index(fields = ["name"], kind = "Rtree"))]
    pub struct OddIndex {
        #[id]
        #[default(autoincrement)]
        pub id: i32,
        pub name: String,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, OrmEnum)]
    #[orm_enum(name = "Mood", map = "mo\"od; DROP TYPE x; --")]
    pub enum Mood {
        #[map("it's ok")]
        Ok,
        Bad,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Model)]
    #[model(table = "we\"ird; DROP TABLE first; --", unique(fields = ["label", "mood"]))]
    pub struct Weird {
        #[id]
        #[default(autoincrement)]
        pub id: i32,
        #[map("la\"bel")]
        #[default("it's")]
        pub label: String,
        #[default(Ok)]
        pub mood: Mood,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Model)]
    #[model(table = "in_schema", schema = "sch\"ema; --")]
    pub struct InSchema {
        #[id]
        #[default(autoincrement)]
        pub id: i32,
        pub mood: Mood,
    }
}

async fn tables(db: &OrmClient) -> Vec<String> {
    let sql = match db.dialect() {
        corvis_orm::Dialect::Sqlite => "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        corvis_orm::Dialect::Postgres => {
            "SELECT tablename::text AS name FROM pg_tables WHERE schemaname NOT IN ('pg_catalog', 'information_schema') ORDER BY 1"
        }
    };
    db.query_raw(sql, vec![]).await.unwrap().into_iter().map(|r| r["name"].as_str().unwrap().to_string()).collect()
}

async fn push_is_atomic(db: &OrmClient) {
    let e = db.db_push::<(edge::First, edge::Second, edge::Third)>().await.unwrap_err();
    assert!(!e.message().is_empty());
    assert_eq!(tables(db).await, Vec::<String>::new(), "a failed push left tables behind");
    // Without the failing model the same push works.
    db.db_push::<(edge::First, edge::Second)>().await.unwrap();
    assert_eq!(tables(db).await, ["first", "second"]);
}

#[tokio::test]
async fn a_failed_push_leaves_nothing_sqlite() {
    push_is_atomic(&OrmClient::new("sqlite::memory:").unwrap()).await;
}

#[tokio::test]
async fn a_failed_push_leaves_nothing_postgres() {
    let Some((url, _drop_url)) = pg::fresh_db_bounded("push_atomic", 30).await else { return };
    push_is_atomic(&OrmClient::new(url).unwrap()).await;
}

#[test]
fn unknown_native_types_and_index_types_are_refused_before_any_statement() {
    let db = OrmClient::new("postgres://u:p@127.0.0.1:1/db").unwrap();
    let e = db.schema_sql::<(edge::First, edge::OddType)>().unwrap_err();
    assert!(e.is_validation(), "{e}");
    for part in ["OddType.area", "@db.Geometry(4326)"] {
        assert!(e.message().contains(part), "{}", e.message());
    }
    let e = db.schema_sql::<edge::OddIndex>().unwrap_err();
    assert!(e.is_validation() && e.message().contains("OddIndex.name") && e.message().contains("Rtree"), "{}", e.message());
    // SQLite has no native or index types: they are ignored there, as before.
    let sqlite = OrmClient::new("sqlite::memory:").unwrap();
    sqlite.schema_sql::<(edge::OddType, edge::OddIndex)>().unwrap();
}

#[tokio::test]
async fn a_refused_push_runs_no_statement_postgres() {
    // The server is never contacted: the plan fails first (port 1 refuses connections).
    let db = OrmClient::new("postgres://u:p@127.0.0.1:1/db").unwrap();
    let e = db.db_push::<(edge::First, edge::OddType)>().await.unwrap_err();
    assert!(e.is_validation(), "{e}");
}

async fn hostile_names(db: &OrmClient) {
    db.db_push::<(edge::First, edge::Weird)>().await.unwrap();
    let weird = edge::Weird::meta();
    let row = q(db, weird, "create", json!({ "data": {} })).await.unwrap();
    assert_eq!((row["label"].clone(), row["mood"].clone()), (json!("it's"), json!("Ok")));
    let row = q(db, weird, "create", json!({ "data": { "mood": "Bad" } })).await.unwrap();
    assert_eq!(row["mood"], "Bad");
    let dup = q(db, weird, "create", json!({ "data": { "mood": "Bad" } })).await.unwrap_err();
    assert_eq!(dup.code(), Some("P2002"), "the compound unique index exists");
    assert!(tables(db).await.contains(&"first".to_string()), "the table named in the hostile name is still there");
}

#[tokio::test]
async fn hostile_names_are_quoted_sqlite() {
    hostile_names(&OrmClient::new("sqlite::memory:").unwrap()).await;
}

#[tokio::test]
async fn hostile_names_are_quoted_postgres() {
    let Some((url, _drop_url)) = pg::fresh_db_bounded("push_hostile", 30).await else { return };
    let db = OrmClient::new(url).unwrap();
    hostile_names(&db).await;
    db.db_push::<edge::InSchema>().await.unwrap();
    let row = q(&db, edge::InSchema::meta(), "create", json!({ "data": { "mood": "Ok" } })).await.unwrap();
    assert_eq!(row["mood"], "Ok");
    // Pushing again finds the schema, the enum type and the tables, and creates nothing.
    db.db_push::<(edge::First, edge::Weird, edge::InSchema)>().await.unwrap();
}

async fn existing_tables_are_left_alone(db: &OrmClient) {
    db.db_execute("CREATE TABLE \"first\" (\"id\" INTEGER NOT NULL PRIMARY KEY, \"legacy\" TEXT)").await.unwrap();
    db.db_push::<(edge::First, edge::Second)>().await.unwrap();
    assert_eq!(tables(db).await, ["first", "second"]);
    let rows = db.query_raw("SELECT \"legacy\" FROM \"first\"", vec![]).await.unwrap();
    assert!(rows.is_empty(), "the old column is still there");
}

#[tokio::test]
async fn existing_tables_are_left_alone_sqlite() {
    existing_tables_are_left_alone(&OrmClient::new("sqlite::memory:").unwrap()).await;
}

#[tokio::test]
async fn existing_tables_are_left_alone_postgres() {
    let Some((url, _drop_url)) = pg::fresh_db_bounded("push_existing", 30).await else { return };
    existing_tables_are_left_alone(&OrmClient::new(url).unwrap()).await;
}

#[tokio::test]
async fn force_reset_recreates_the_tables_and_join_tables() {
    let db = sqlite::<(User, Post, Category)>().await;
    nested_m2m_create(&db).await;
    db.db_push_force_reset::<(User, Post, Category)>().await.unwrap();
    assert_eq!(q(&db, meta::<Category>(), "count", json!({})).await.unwrap(), json!(0));
    nested_m2m_create(&db).await;
}

// ---------------------------------------------------------------------------------------------
// Catalog comparison with the reference migration
// ---------------------------------------------------------------------------------------------

/// The reference migration of the oracle schema without its block of extras (a `CHECK` and a trigger that the
/// oracle adds by hand: not part of what the original's migration builds for the models).
fn reference_migration() -> &'static str {
    let sql = include_str!("fixtures/oracle/migration.sql");
    sql.split("-- Oracle extras").next().unwrap()
}

/// Tables, columns (type, nullability, default), primary, unique and foreign key constraints, indexes and enum types
/// (labels in order) of the current schema, each list sorted, as JSON.
async fn pg_catalog(db: &OrmClient) -> Value {
    let queries = [
        (
            "columns",
            "SELECT c.relname::text || '.' || a.attname::text AS column, format_type(a.atttypid, a.atttypmod) AS type, \
                    a.attnotnull AS not_null, pg_get_expr(d.adbin, d.adrelid) AS default \
             FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE n.nspname = current_schema() AND c.relkind = 'r' AND a.attnum > 0 AND NOT a.attisdropped ORDER BY 1",
        ),
        (
            "constraints",
            "SELECT c.relname::text || '.' || con.conname::text AS name, con.contype::text AS kind, pg_get_constraintdef(con.oid) AS def \
             FROM pg_constraint con JOIN pg_class c ON c.oid = con.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = current_schema() ORDER BY 1",
        ),
        ("indexes", "SELECT indexname::text AS name, indexdef::text AS def FROM pg_indexes WHERE schemaname = current_schema() ORDER BY 1"),
        (
            "enums",
            "SELECT t.typname::text AS name, string_agg(e.enumlabel::text, ',' ORDER BY e.enumsortorder) AS labels \
             FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid JOIN pg_namespace n ON n.oid = t.typnamespace \
             WHERE n.nspname = current_schema() GROUP BY 1 ORDER BY 1",
        ),
    ];
    let mut out = serde_json::Map::new();
    for (name, sql) in queries {
        out.insert(name.to_string(), Value::Array(db.query_raw(sql, vec![]).await.unwrap()));
    }
    Value::Object(out)
}

/// The entries of each list that only one of the two catalogs has, except `accepted` (entries known to differ).
fn catalog_differences(reference: &Value, pushed: &Value, accepted: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for (section, items) in reference.as_object().unwrap() {
        let other = pushed[section].as_array().unwrap();
        let items = items.as_array().unwrap();
        for (side, a, b) in [("only in the reference", items, other), ("only in the pushed database", other, items)] {
            for x in a.iter().filter(|x| !b.contains(x)) {
                let line = format!("{section} {side}: {x}");
                if !accepted.iter().any(|acc| line.contains(acc)) {
                    out.push(line);
                }
            }
        }
    }
    out
}

#[tokio::test]
async fn catalog_equals_the_reference_migration_postgres() {
    let Some((reference_url, _drop_reference_url)) = pg::fresh_db_bounded("push_catalog_ref", 30).await else { return };
    let Some((pushed_url, _drop_pushed_url)) = pg::fresh_db_bounded("push_catalog_pushed", 30).await else { return };
    let reference = OrmClient::new(reference_url).unwrap();
    reference.db_execute(reference_migration()).await.unwrap();
    let pushed = OrmClient::new(pushed_url).unwrap();
    pushed.db_push::<OracleModels>().await.unwrap();
    let diffs = catalog_differences(&pg_catalog(&reference).await, &pg_catalog(&pushed).await, &[]);
    assert!(diffs.is_empty(), "{} differences:\n{}", diffs.len(), diffs.join("\n"));
}

mod houseplan {
    #![allow(dead_code)]
    include!("generated/houseplan.rs");
}

/// The houseplan models (one of the port applications, `corvis-tests/nest/houseplan`) pushed, against its own
/// migrations. Skipped when that folder is not there.
#[tokio::test]
async fn catalog_equals_the_houseplan_migrations_postgres() {
    use houseplan::*;
    let Some(dir) = pg::app_dir("houseplan/api/prisma/migrations") else {
        eprintln!("skipping: corvis-tests/nest/houseplan is not available (set CORVIS_TESTS_DIR)");
        return;
    };
    let Some((reference_url, _drop_reference_url)) = pg::fresh_db_bounded("push_hp_ref", 30).await else { return };
    let Some((pushed_url, _drop_pushed_url)) = pg::fresh_db_bounded("push_hp_pushed", 30).await else { return };
    let reference = OrmClient::new(reference_url).unwrap();
    if let Err(e) = reference.migrate_deploy(&dir).await {
        eprintln!("skipping: the houseplan migrations do not apply here (the `vector` extension is needed): {e}");
        return;
    }
    let pushed = OrmClient::new(pushed_url).unwrap();
    pushed.db_execute("CREATE EXTENSION IF NOT EXISTS vector").await.unwrap();
    pushed
        .db_push::<(
            Account,
            RawEvent,
            Category,
            Bag,
            Transaction,
            Debt,
            PlanItem,
            PlanEntry,
            TransactionVersion,
            BagAllocation,
            MerchantRule,
            WalletSnapshot,
            FxRate,
            PendingPrompt,
            ChatTurn,
            Embedding,
        )>()
        .await
        .unwrap();
    pushed.db_push::<(Setting, WebSession)>().await.unwrap();
    let accepted = [
        // The migrations' own bookkeeping table.
        "_prisma_migrations",
        // Written by hand in `0000_init/migration.sql` (`USING hnsw` with an operator class on an `Unsupported`
        // column); `schema.prisma` does not declare it, so no client builds it from the models.
        "embeddings_hnsw",
    ];
    let diffs = catalog_differences(&pg_catalog(&reference).await, &pg_catalog(&pushed).await, &accepted);
    assert!(diffs.is_empty(), "{} differences:\n{}", diffs.len(), diffs.join("\n"));
}
