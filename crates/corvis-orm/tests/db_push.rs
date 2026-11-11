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

async fn postgres<S: ModelSet>(name: &str) -> Option<OrmClient> {
    let url = pg::fresh_db(&format!("push_{name}")).await?;
    let db = OrmClient::new(url).unwrap();
    db.db_push::<S>().await.unwrap();
    Some(db)
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
    let post = q(db, meta::<Post>(), "create", json!({ "data": { "title": "Hola", "categories": { "create": [{ "name": "tech" }] } }, "include": { "categories": true } }))
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
    let Some(db) = postgres::<Pair>("pk").await else { return };
    two_pairs_share_a(&db).await;
}

#[tokio::test]
async fn implicit_many_to_many_postgres() {
    let Some(db) = postgres::<(User, Post, Category)>("m2m").await else { return };
    nested_m2m_create(&db).await;
}

/// Every oracle model in one push, then an entry that references a pair by its compound key.
async fn all_oracle_models(db: &OrmClient) {
    two_pairs_share_a(db).await;
    nested_m2m_create(db).await;
    let user = q(db, meta::<User>(), "create", json!({ "data": { "email": "w@x.io" } })).await.unwrap();
    let wallet_id = "1f6a3c2e-6c55-4a43-9a2a-0d3c4a1b2c3d";
    q(db, meta::<Wallet>(), "create", json!({ "data": { "id": wallet_id, "userId": user["id"], "currency": "VES", "amount": 2 } })).await.unwrap();
    let entry = q(db, meta::<Entry>(), "create", json!({ "data": { "walletId": wallet_id, "pairA": "x", "pairB": 2, "amount": 1 }, "include": { "pair": true } }))
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
    let Some(db) = postgres::<OracleModels>("all").await else { return };
    all_oracle_models(&db).await;
}

// ---------------------------------------------------------------------------------------------
// Postgres types: enums, native types, scalar lists
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn enum_types_postgres() {
    let Some(db) = postgres::<User>("enum").await else { return };
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
    let Some(db) = postgres::<(User, Wallet)>("native").await else { return };
    let user = q(&db, meta::<User>(), "create", json!({ "data": { "email": "a@x.io", "scores": [1, 2], "tags": ["t"], "birthday": "1990-05-06T13:00:00.000Z" } }))
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
    let Some(db) = postgres::<Group>("uuid").await else { return };
    let id = "5b0f2b8e-2f0a-4a8e-8f8e-1c2d3e4f5a6b";
    q(&db, meta::<Group>(), "create", json!({ "data": { "id": id, "name": "g" } })).await.unwrap();
    let found = q(&db, meta::<Group>(), "findMany", json!({ "where": { "id": id } })).await.unwrap();
    assert_eq!(found[0]["name"], "g");
}
