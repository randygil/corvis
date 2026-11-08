//! The `schema.prisma` of the blog:
//!
//! ```text
//! model User {
//!   id        Int      @id @default(autoincrement())
//!   email     String   @unique
//!   name      String?
//!   createdAt DateTime @default(now()) @map("created_at")
//!   posts     Post[]
//!   @@map("users")
//! }
//!
//! model Post {
//!   id        Int      @id @default(autoincrement())
//!   title     String
//!   content   String?
//!   published Boolean  @default(false)
//!   views     Int      @default(0)
//!   createdAt DateTime @default(now()) @map("created_at")
//!   updatedAt DateTime @updatedAt @map("updated_at")
//!   authorId  Int      @map("author_id")
//!   author    User     @relation(fields: [authorId], references: [id], onDelete: Cascade)
//!   @@map("posts")
//! }
//! ```

use corvis_orm::DateTime;
use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "users")]
pub struct User {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    #[unique]
    pub email: String,
    pub name: Option<String>,
    #[default(now)]
    pub created_at: DateTime,
    #[relation(has_many)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<Post>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
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
    #[default(now)]
    pub created_at: DateTime,
    #[updated_at]
    pub updated_at: DateTime,
    pub author_id: i64,
    #[relation(belongs_to, fields = "author_id", on_delete = "Cascade")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Box<User>>,
}

/// `{ data, meta }` envelope for paginated lists.
#[derive(Debug, Serialize)]
pub struct Paginated<T> {
    pub data: Vec<T>,
    pub meta: PageMeta,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMeta {
    pub total: i64,
    pub page: u32,
    pub limit: u32,
    pub total_pages: i64,
}

impl<T> Paginated<T> {
    pub fn new(data: Vec<T>, total: i64, page: u32, limit: u32) -> Self {
        let total_pages = if limit == 0 { 0 } else { (total as f64 / limit as f64).ceil() as i64 };
        Paginated { data, meta: PageMeta { total, page, limit, total_pages } }
    }
}
