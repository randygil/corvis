mod common;

use common::*;
use corvis_orm::prelude::*;
use corvis_orm::{and, or};

#[tokio::test]
async fn crud_roundtrip() {
    let db = client().await;
    let u: User = db.user().create(json!({ "data": { "email": "a@b.c", "name": "Ana" } })).await.unwrap();
    assert_eq!(u.id, 1);
    assert_eq!(u.email, "a@b.c");
    assert_eq!(u.age, 0, "literal default applied");
    assert!(u.created_at <= chrono::Utc::now());
    assert!(u.posts.is_none());

    let found = db.user().find_unique(json!({ "where": { "email": "a@b.c" } })).await.unwrap();
    assert_eq!(found.unwrap().id, 1);
    assert!(db.user().find_unique(json!({ "where": { "id": 99 } })).await.unwrap().is_none());

    let updated = db.user().update(json!({ "where": { "id": 1 }, "data": { "name": "Ana B", "age": { "increment": 2 } } })).await.unwrap();
    assert_eq!(updated.name.as_deref(), Some("Ana B"));
    assert_eq!(updated.age, 2);

    let deleted = db.user().delete(json!({ "where": { "id": 1 } })).await.unwrap();
    assert_eq!(deleted.email, "a@b.c");
    assert_eq!(db.user().count(()).await.unwrap(), 0);
}

#[tokio::test]
async fn filters_combined() {
    let db = seeded().await;
    let users = db
        .user()
        .find_many(json!({
            "where": {
                "email": { "contains": "@corvis.dev" },
                "OR": [{ "name": null }, { "name": { "startsWith": "A" } }]
            },
            "orderBy": { "id": "desc" }
        }))
        .await
        .unwrap();
    let emails: Vec<_> = users.iter().map(|u| u.email.as_str()).collect();
    assert_eq!(emails, ["carla@corvis.dev", "ana@corvis.dev"]);

    // in / notIn / gt / lte / NOT / not
    let n = db.user().count(json!({ "where": { "age": { "gt": 25, "lte": 41 } } })).await.unwrap();
    assert_eq!(n, 2);
    let n = db.user().count(json!({ "where": { "id": { "in": [1, 3] } } })).await.unwrap();
    assert_eq!(n, 2);
    let n = db.user().count(json!({ "where": { "id": { "notIn": [1] } } })).await.unwrap();
    assert_eq!(n, 2);
    let n = db.user().count(json!({ "where": { "NOT": { "email": { "endsWith": ".dev" } } } })).await.unwrap();
    assert_eq!(n, 1);
    let n = db.user().count(json!({ "where": { "name": { "not": null } } })).await.unwrap();
    assert_eq!(n, 2);
    // case sensitivity
    let n = db.user().count(json!({ "where": { "name": { "contains": "ana" } } })).await.unwrap();
    assert_eq!(n, 0);
    let n = db.user().count(json!({ "where": { "name": { "contains": "ana", "mode": "insensitive" } } })).await.unwrap();
    assert_eq!(n, 1);
    // relation filters
    let n = db.user().count(json!({ "where": { "posts": { "some": { "published": true } } } })).await.unwrap();
    assert_eq!(n, 2);
    let n = db.user().count(json!({ "where": { "posts": { "none": {} } } })).await.unwrap();
    assert_eq!(n, 1);
    let n = db.user().count(json!({ "where": { "posts": { "every": { "published": true } } } })).await.unwrap();
    assert_eq!(n, 2, "Bob (all published) and Carla (vacuously)");
    let posts = db.post().find_many(json!({ "where": { "author": { "is": { "name": "Bob" } } } })).await.unwrap();
    assert_eq!(posts.len(), 1);
    let posts = db.post().find_many(json!({ "where": { "author": { "isNot": { "name": "Bob" } } } })).await.unwrap();
    assert_eq!(posts.len(), 2);
}

#[tokio::test]
async fn order_take_skip_distinct_cursor() {
    let db = seeded().await;
    let posts = db.post().find_many(json!({ "orderBy": [{ "views": "desc" }], "take": 2, "skip": 1 })).await.unwrap();
    assert_eq!(posts.iter().map(|p| p.views).collect::<Vec<_>>(), [7, 3]);

    let last = db.post().find_many(json!({ "orderBy": { "id": "asc" }, "take": -1 })).await.unwrap();
    assert_eq!(last[0].id, 3);

    let first = db.post().find_first(json!({ "where": { "published": true }, "orderBy": { "views": "asc" } })).await.unwrap();
    assert_eq!(first.unwrap().title, "Rust y SQL");

    let page = db.post().find_many(json!({ "cursor": { "id": 2 }, "take": 2, "orderBy": { "id": "asc" } })).await.unwrap();
    assert_eq!(page.iter().map(|p| p.id).collect::<Vec<_>>(), [2, 3]);

    let distinct = db.post().find_many(json!({ "distinct": ["author_id"], "orderBy": { "id": "asc" } })).await.unwrap();
    assert_eq!(distinct.iter().map(|p| p.author_id).collect::<Vec<_>>(), [1, 2]);

    // order by relation
    let by_author = db.post().find_many(json!({ "orderBy": [{ "author": { "name": "desc" } }, { "id": "asc" }] })).await.unwrap();
    assert_eq!(by_author[0].author_id, 2);
    let by_count = db.user().find_many(json!({ "orderBy": { "posts": { "_count": "desc" } } })).await.unwrap();
    assert_eq!(by_count[0].email, "ana@corvis.dev");
}

#[tokio::test]
async fn include_both_directions_and_select() {
    let db = seeded().await;
    let users = db
        .user()
        .find_many(json!({
            "include": { "posts": { "where": { "published": true }, "orderBy": { "id": "desc" } }, "profile": true },
            "orderBy": { "id": "asc" }
        }))
        .await
        .unwrap();
    assert_eq!(users[0].posts.as_ref().unwrap().len(), 1);
    assert_eq!(users[1].posts.as_ref().unwrap()[0].title, "Rust y SQL");
    assert!(users[2].posts.as_ref().unwrap().is_empty());
    assert!(users[0].profile.is_none());

    // nested take per parent
    let users = db.user().find_many(json!({ "include": { "posts": { "take": 1, "orderBy": { "views": "asc" } } } })).await.unwrap();
    assert_eq!(users[0].posts.as_ref().unwrap()[0].title, "Borrador");

    let posts = db.post().find_many(json!({ "include": { "author": true }, "orderBy": { "id": "asc" } })).await.unwrap();
    assert_eq!(posts[2].author.as_ref().unwrap().email, "bob@example.com");

    // nested include
    let post = db
        .post()
        .find_unique(json!({ "where": { "id": 1 }, "include": { "author": { "include": { "posts": true } } } }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(post.author.unwrap().posts.unwrap().len(), 2);

    // select -> partial JSON
    let rows: Value = db
        .user()
        .find_many(json!({ "select": { "email": true, "posts": { "select": { "title": true } }, "_count": { "select": { "posts": true } } }, "where": { "id": 1 } }))
        .exec_json()
        .await
        .unwrap();
    assert_eq!(
        rows,
        json!([{ "email": "ana@corvis.dev", "posts": [{ "title": "Hola mundo" }, { "title": "Borrador" }], "_count": { "posts": 2 } }])
    );
}

#[tokio::test]
async fn nested_create_and_relation_writes() {
    let db = client().await;
    let u = db
        .user()
        .create(json!({
            "data": {
                "email": "nested@corvis.dev",
                "posts": { "create": [{ "title": "Uno" }, { "title": "Dos" }] },
                "profile": { "create": { "bio": "Rustacean" } }
            },
            "include": { "posts": true, "profile": true }
        }))
        .await
        .unwrap();
    assert_eq!(u.posts.as_ref().unwrap().len(), 2);
    assert_eq!(u.profile.as_ref().unwrap().bio, "Rustacean");
    assert_eq!(u.profile.as_ref().unwrap().id.len(), 36, "uuid default");

    // belongs_to connect / create
    let p = db.post().create(json!({ "data": { "title": "Tres", "author": { "connect": { "email": "nested@corvis.dev" } } }, "include": { "author": true } })).await.unwrap();
    assert_eq!(p.author.unwrap().id, u.id);
    let p =
        db.post().create(json!({ "data": { "title": "Cuatro", "author": { "create": { "email": "new@corvis.dev" } } } })).await.unwrap();
    assert_eq!(p.author_id, 2);

    // nested update: update/delete posts of a user
    let u = db
        .user()
        .update(json!({
            "where": { "id": 1 },
            "data": {
                "posts": {
                    "updateMany": { "where": { "title": { "startsWith": "U" } }, "data": { "published": true } },
                    "delete": { "id": 2 },
                    "create": { "title": "Cinco" }
                }
            },
            "include": { "posts": { "orderBy": { "id": "asc" } } }
        }))
        .await
        .unwrap();
    let titles: Vec<_> = u.posts.unwrap().into_iter().map(|p| (p.title, p.published)).collect();
    assert_eq!(titles, [("Uno".to_string(), true), ("Tres".to_string(), false), ("Cinco".to_string(), false)]);

    // connect a nonexistent record -> P2025, and the nested write is rolled back
    let err = db.user().create(json!({ "data": { "email": "rollback@x.y", "posts": { "connect": [{ "id": 999 }] } } })).await.unwrap_err();
    assert_eq!(err.code(), Some("P2025"));
    assert!(db.user().find_unique(json!({ "where": { "email": "rollback@x.y" } })).await.unwrap().is_none());
}

#[tokio::test]
async fn unique_violation_p2002() {
    let db = seeded().await;
    let err = db.user().create(json!({ "data": { "email": "ana@corvis.dev" } })).await.unwrap_err();
    assert_eq!(err.code(), Some("P2002"));
    assert_eq!(err.message(), "Unique constraint failed on the fields: (`email`)");
    assert_eq!(err.meta().unwrap()["target"], json!(["email"]));
    assert_eq!(err.name(), "KnownRequestError");
}

#[tokio::test]
async fn not_found_p2025() {
    let db = seeded().await;
    let err = db.user().update(json!({ "where": { "id": 42 }, "data": { "name": "x" } })).await.unwrap_err();
    assert!(err.is_not_found());
    assert!(err.message().contains("No record was found for an update."), "{err}");
    let err = db.user().delete(json!({ "where": { "id": 42 } })).await.unwrap_err();
    assert_eq!(err.code(), Some("P2025"));
    let err = db.user().find_unique_or_throw(json!({ "where": { "id": 42 } })).await.unwrap_err();
    assert_eq!(err.code(), Some("P2025"));
    let err = db.user().find_first_or_throw(json!({ "where": { "name": "Nadie" } })).await.unwrap_err();
    assert_eq!(err.code(), Some("P2025"));
}

#[tokio::test]
async fn foreign_key_p2003() {
    let db = seeded().await;
    let err = db.post().create(json!({ "data": { "title": "Huérfano", "author_id": 999 } })).await.unwrap_err();
    assert_eq!(err.code(), Some("P2003"), "{err}");
}

#[tokio::test]
async fn validation_errors() {
    let db = seeded().await;
    let err = db.user().find_many(json!({ "where": { "emial": "x" } })).await.unwrap_err();
    assert!(err.is_validation());
    assert_eq!(err.name(), "QueryValidationError");
    assert!(err.message().contains("Invalid `client.user.findMany()` invocation"), "{err}");
    assert!(err.message().contains("Unknown argument `emial`. Did you mean `email`?"), "{err}");

    let err = db.user().find_many(json!({ "wher": {} })).await.unwrap_err();
    assert!(err.message().contains("Did you mean `where`?"), "{err}");

    let err = db.user().find_many(json!({ "where": { "id": "uno" } })).await.unwrap_err();
    assert!(err.message().contains("Argument `id`: Invalid value provided. Expected BigInt, provided String."), "{err}");

    let err = db.user().create(json!({ "data": { "name": "Sin email" } })).await.unwrap_err();
    assert!(err.message().contains("Argument `email` is missing."), "{err}");

    let err = db.user().find_unique(json!({ "where": { "name": "Ana" } })).await.unwrap_err();
    assert!(err.message().contains("needs at least one of `id` or `email` arguments"), "{err}");

    let err = db.user().find_many(json!({ "include": { "postss": true } })).await.unwrap_err();
    assert!(err.message().contains("Did you mean `posts`?"), "{err}");

    let err = db.user().find_many(json!({ "select": { "id": true }, "include": { "posts": true } })).await.unwrap_err();
    assert!(err.message().contains("either use `include` or `select`"), "{err}");
}

#[tokio::test]
async fn many_operations_and_upsert() {
    let db = seeded().await;
    let r = db
        .user()
        .create_many(
            json!({ "data": [{ "email": "x1@x.y" }, { "email": "x2@x.y" }, { "email": "ana@corvis.dev" }], "skipDuplicates": true }),
        )
        .await
        .unwrap();
    assert_eq!(r.count, 2);
    let r = db.user().update_many(json!({ "where": { "email": { "endsWith": "@x.y" } }, "data": { "age": 18 } })).await.unwrap();
    assert_eq!(r.count, 2);
    let r = db.user().delete_many(json!({ "where": { "age": 18 } })).await.unwrap();
    assert_eq!(r.count, 2);

    let u = db
        .user()
        .upsert(json!({ "where": { "email": "dora@corvis.dev" }, "create": { "email": "dora@corvis.dev", "name": "Dora" }, "update": { "name": "X" } }))
        .await
        .unwrap();
    assert_eq!(u.name.as_deref(), Some("Dora"));
    let u = db
        .user()
        .upsert(
            json!({ "where": { "email": "dora@corvis.dev" }, "create": { "email": "dora@corvis.dev" }, "update": { "name": "Dora 2" } }),
        )
        .await
        .unwrap();
    assert_eq!(u.name.as_deref(), Some("Dora 2"));
    assert_eq!(db.user().count(()).await.unwrap(), 4);
}

#[tokio::test]
async fn aggregate_and_group_by() {
    let db = seeded().await;
    let agg = db
        .post()
        .aggregate(json!({ "_count": { "_all": true }, "_sum": { "views": true }, "_avg": { "views": true }, "_max": { "views": true }, "_min": { "title": true } }))
        .await
        .unwrap();
    assert_eq!(agg["_count"]["_all"], 3);
    assert_eq!(agg["_sum"]["views"], 20);
    assert!((agg["_avg"]["views"].as_f64().unwrap() - 20.0 / 3.0).abs() < 1e-9);
    assert_eq!(agg["_max"]["views"], 10);
    assert_eq!(agg["_min"]["title"], "Borrador");

    let groups = db
        .post()
        .group_by(json!({ "by": ["author_id"], "_count": { "_all": true }, "_sum": { "views": true }, "orderBy": { "author_id": "asc" } }))
        .await
        .unwrap();
    assert_eq!(
        groups,
        json!([
            { "author_id": 1, "_count": { "_all": 2 }, "_sum": { "views": 13 } },
            { "author_id": 2, "_count": { "_all": 1 }, "_sum": { "views": 7 } }
        ])
    );
    let c = db.post().count(json!({ "select": { "_all": true, "content": true } })).exec_json().await.unwrap();
    assert_eq!(c, json!({ "_all": 3, "content": 0 }));
}

#[tokio::test]
async fn transactions_commit_and_rollback() {
    let db = client().await;
    let (u, p) = db
        ._transaction(|tx| async move {
            let u = tx.user().create(json!({ "data": { "email": "tx@corvis.dev" } })).await?;
            let p = tx.post().create(json!({ "data": { "title": "En tx", "author_id": u.id } })).await?;
            Ok::<_, OrmError>((u, p))
        })
        .await
        .unwrap();
    assert_eq!(p.author_id, u.id);

    let res: Result<(), OrmError> = db
        ._transaction(|tx| async move {
            tx.user().create(json!({ "data": { "email": "gone@corvis.dev" } })).await?;
            tx.user().create(json!({ "data": { "email": "tx@corvis.dev" } })).await?; // P2002
            Ok(())
        })
        .await;
    assert_eq!(res.unwrap_err().code(), Some("P2002"));
    assert_eq!(db.user().count(()).await.unwrap(), 1, "rolled back");
}

#[tokio::test]
async fn typed_api() {
    let db = seeded().await;
    let users = db
        .user()
        .find_many(user::email::contains("@corvis.dev"))
        .order_by(user::id::order(SortOrder::Desc))
        .take(10)
        .include(user::posts::include())
        .exec()
        .await
        .unwrap();
    assert_eq!(users.len(), 2);
    assert_eq!(users[0].email, "carla@corvis.dev");
    assert!(users[1].posts.as_ref().unwrap().len() == 2);

    let users =
        db.user().find_many(vec![or(vec![user::name::is_null(), user::name::starts_with("B")]), user::age::lt(40)]).exec().await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].name.as_deref(), Some("Bob"));

    let posts = db
        .post()
        .find_many(vec![and(vec![post::views::gte(3), post::views::lte(7)]), post::author::is(vec![user::name::equals("Ana")])])
        .exec()
        .await
        .unwrap();
    assert_eq!(posts.len(), 1);

    let n = db.user().count(vec![user::posts::some(vec![post::published::equals(true)])]).await.unwrap();
    assert_eq!(n, 2);

    let created = db
        .user()
        .create(vec![
            user::email::set("typed@corvis.dev"),
            user::name::set("Typed"),
            user::posts::create(vec![vec![post::title::set("T1")]]),
        ])
        .include(user::posts::include())
        .exec()
        .await
        .unwrap();
    assert_eq!(created.posts.unwrap().len(), 1);

    let updated = db
        .user()
        .update(user::email::equals("typed@corvis.dev"))
        .data(vec![user::age::increment(5), user::name::set_null()])
        .await
        .unwrap();
    assert_eq!(updated.age, 5);
    assert!(updated.name.is_none());

    let found = db.user().find_unique(user::id::equals(updated.id)).await.unwrap();
    assert!(found.is_some());

    // The model-generic delegate is equivalent.
    let same = db.model::<User>().count(()).await.unwrap();
    assert_eq!(same, 4);

    // Typed helpers build the same JSON as the JSON style.
    let q = db.user().find_many(user::email::contains("@x")).take(5);
    assert_eq!(q.args(), &json!({ "where": { "email": { "contains": "@x" } }, "take": 5 }));
}

#[tokio::test]
async fn raw_queries_and_schema() {
    let db = seeded().await;
    let rows = db.query_raw("SELECT email, age FROM users WHERE age > ? ORDER BY age", vec![json!(26)]).await.unwrap();
    assert_eq!(rows, vec![json!({ "email": "ana@corvis.dev", "age": 30 }), json!({ "email": "carla@corvis.dev", "age": 41 })]);
    let n = db.execute_raw("UPDATE users SET age = age + 1 WHERE id = ?", vec![json!(1)]).await.unwrap();
    assert_eq!(n, 1);
    let sql = db.schema_sql::<(User, Post)>().unwrap().join(";\n");
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS \"users_email_key\""), "{sql}");
    assert!(sql.contains("FOREIGN KEY (\"author_id\") REFERENCES \"users\" (\"id\") ON DELETE CASCADE"), "{sql}");
    // db push is idempotent
    db.db_push::<(User, Post, Profile)>().await.unwrap();
    db.db_push_force_reset::<(User, Post, Profile)>().await.unwrap();
    assert_eq!(db.user().count(()).await.unwrap(), 0);
}

#[tokio::test]
async fn datetime_filters() {
    let db = seeded().await;
    let future = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    let n = db.user().count(json!({ "where": { "created_at": { "lt": future } } })).await.unwrap();
    assert_eq!(n, 3);
    let n = db.user().count(vec![user::created_at::gt(chrono::Utc::now() + chrono::Duration::days(1))]).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn postgres_sql_generation() {
    // No server needed: only check the generated SQL and placeholders.
    let db = OrmClient::new("postgresql://user:pass@127.0.0.1:1/db").unwrap();
    assert_eq!(db.dialect(), corvis_orm::Dialect::Postgres);
    let ddl = db.schema_sql::<(User, Post)>().unwrap();
    assert!(ddl[0].contains("\"id\" BIGSERIAL NOT NULL"), "{}", ddl[0]);
    assert!(ddl[0].contains("\"created_at\" TIMESTAMPTZ(3) NOT NULL"), "{}", ddl[0]);
    assert!(ddl[0].contains("CONSTRAINT \"users_pkey\" PRIMARY KEY (\"id\")"), "{}", ddl[0]);
    let sql = corvis_orm::debug_sql(
        &db,
        User::meta(),
        &json!({ "email": { "contains": "x", "mode": "insensitive" }, "OR": [{ "age": { "gt": 3 } }, { "id": { "in": [1, 2] } }] }),
    )
    .unwrap();
    // Like the reference client: the column is cast to text and Postgres' default `\` escape is used.
    assert_eq!(sql, "((t0.\"email\"::text ILIKE $1) AND (((t0.\"age\" > $2)) OR ((t0.\"id\" IN ($3, $4)))))");
}

fn pool_tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("corvis-orm-client-pool-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn pool_slash(dir: &std::path::Path) -> String {
    dir.to_string_lossy().replace('\\', "/")
}

#[tokio::test]
async fn pool_sqlite_scheme_url_with_connection_limit_works() {
    // The driver rejects `connection_limit` as an unknown query parameter; the client removes it.
    let dir = pool_tmp("scheme");
    let db = OrmClient::new(format!("sqlite://{}/pool.db?connection_limit=1", pool_slash(&dir))).unwrap();
    db.db_push::<(User, Post, Profile)>().await.unwrap();
    db.user().create(json!({ "data": { "email": "p@corvis.dev" } })).await.unwrap();
    assert_eq!(db.user().count(()).await.unwrap(), 1);
    db.disconnect().await.unwrap();
    assert!(dir.join("pool.db").exists(), "the file is pool.db, the query string is not part of the name");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Holds the only connection with an interactive transaction while a raw query runs outside it.
async fn pool_held_by_transaction(url: String, hold: std::time::Duration) -> (Result<u64, OrmError>, std::time::Duration) {
    let db = OrmClient::new(url).unwrap();
    db.db_push::<(User, Post, Profile)>().await.unwrap();
    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();
    let holder = {
        let db = db.clone();
        tokio::spawn(async move {
            db._transaction(|tx| async move {
                tx.user().count(()).await?;
                let _ = held_tx.send(());
                tokio::time::sleep(hold).await;
                Ok::<_, OrmError>(())
            })
            .await
        })
    };
    held_rx.await.unwrap();
    let t = std::time::Instant::now();
    let res = db.execute_raw("SELECT 1", vec![]).await;
    let waited = t.elapsed();
    holder.await.unwrap().unwrap();
    db.disconnect().await.unwrap();
    (res, waited)
}

#[tokio::test]
async fn pool_timeout_in_the_url_gives_p2024() {
    let dir = pool_tmp("timeout");
    let url = format!("file:{}/pool.db?pool_timeout=1&connection_limit=1", pool_slash(&dir));
    let (res, waited) = pool_held_by_transaction(url, std::time::Duration::from_millis(2500)).await;
    assert_eq!(res.unwrap_err().code(), Some("P2024"));
    assert!(waited >= std::time::Duration::from_millis(900) && waited < std::time::Duration::from_millis(2000), "{waited:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn pool_timeout_zero_waits_without_limit() {
    let dir = pool_tmp("nolimit");
    let url = format!("file:{}/pool.db?pool_timeout=0&connection_limit=1", pool_slash(&dir));
    let (res, waited) = pool_held_by_transaction(url, std::time::Duration::from_millis(1500)).await;
    res.unwrap();
    assert!(waited >= std::time::Duration::from_millis(1000), "{waited:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
