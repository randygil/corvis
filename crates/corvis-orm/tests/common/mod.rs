#![allow(dead_code)]

use chrono::{DateTime, Utc};
use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "users")]
pub struct User {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    #[unique]
    pub email: String,
    pub name: Option<String>,
    #[default(0)]
    pub age: i32,
    #[default(now)]
    pub created_at: DateTime<Utc>,
    #[relation(has_many)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<Post>>,
    #[relation(has_one)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Box<Profile>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "posts")]
pub struct Post {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    pub title: String,
    pub content: Option<String>,
    #[default(false)]
    pub published: bool,
    #[default(0)]
    pub views: i64,
    pub author_id: i64,
    #[relation(belongs_to, fields = "author_id", references = "id", on_delete = "Cascade")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Box<User>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
pub struct Profile {
    #[id]
    #[default(uuid)]
    pub id: String,
    pub bio: String,
    #[unique]
    pub user_id: i64,
    #[relation(belongs_to, fields = "user_id")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<Box<User>>,
}

pub async fn client() -> OrmClient {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<(User, Post, Profile)>().await.unwrap();
    db
}

/// Ana (2 posts, one published), Bob (1 post), Carla (no name, no posts).
pub async fn seeded() -> OrmClient {
    let db = client().await;
    db
        .user()
        .create(json!({ "data": {
            "email": "ana@corvis.dev", "name": "Ana", "age": 30,
            "posts": { "create": [
                { "title": "Hola mundo", "published": true, "views": 10 },
                { "title": "Borrador", "views": 3 }
            ] }
        } }))
        .await
        .unwrap();
    db
        .user()
        .create(json!({ "data": {
            "email": "bob@example.com", "name": "Bob", "age": 25,
            "posts": { "create": { "title": "Rust y SQL", "published": true, "views": 7 } }
        } }))
        .await
        .unwrap();
    db
        .user()
        .create(json!({ "data": { "email": "carla@corvis.dev", "age": 41 } }))
        .await
        .unwrap();
    db
}
