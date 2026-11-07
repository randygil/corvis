use corvis::prelude::*;
use corvis_orm::{OrmError, OrmService, SortOrder, or};

use super::dto::{CreatePostDto, ListPostsQuery};
use crate::models::{Post, PostDelegate, UserDelegate, paginated, post, user};

#[injectable]
pub struct PostsService {
    db: Arc<OrmService>,
}

impl PostsService {
    /// Nested write: `author: { connect: { email } }` (P2025 -> 404 if the author does not exist).
    pub async fn create(&self, dto: CreatePostDto) -> Result<Post, OrmError> {
        self.db
            .post()
            .create(json!({
                "data": {
                    "title": dto.title,
                    "content": dto.content,
                    "published": dto.published.unwrap_or(false),
                    "author": { "connect": { "email": dto.author_email } }
                },
                "include": { "author": true }
            }))
            .await
    }

    /// Paginated search (typed style) including the author.
    pub async fn find_all(&self, q: ListPostsQuery) -> Result<Value, OrmError> {
        let page = q.page.unwrap_or(1) as i64;
        let limit = q.limit.unwrap_or(10) as i64;
        let mut filters = Vec::new();
        if let Some(s) = &q.search {
            filters.push(or(vec![post::title::contains_insensitive(s.clone()), post::content::contains_insensitive(s.clone())]));
        }
        if let Some(p) = q.published {
            filters.push(post::published::equals(p));
        }
        let total = self.db.post().count(filters.clone()).await?;
        let posts = self
            .db
            .post()
            .find_many(filters)
            .order_by(post::created_at::order(SortOrder::Desc))
            .order_by(post::id::order(SortOrder::Desc))
            .skip((page - 1) * limit)
            .take(limit)
            .include(post::author::include())
            .await?;
        Ok(paginated(serde_json::to_value(posts)?, total, page as u32, limit as u32))
    }

    pub async fn find_one(&self, id: i64) -> Result<Post, OrmError> {
        self.db.post().find_unique_or_throw(post::id::equals(id)).include(post::author::include()).await
    }

    /// `update` + `increment` – P2025 -> 404.
    pub async fn publish(&self, id: i64) -> Result<Post, OrmError> {
        self.db
            .post()
            .update(post::id::equals(id))
            .data(vec![post::published::set(true), post::views::increment(1)])
            .await
    }

    pub async fn remove(&self, id: i64) -> Result<Post, OrmError> {
        self.db.post().delete(post::id::equals(id)).await
    }

    /// `aggregate` + `groupBy` + relation filters.
    pub async fn stats(&self) -> Result<Value, OrmError> {
        let totals = self
            .db
            .post()
            .aggregate(json!({ "_count": { "_all": true }, "_sum": { "views": true }, "_avg": { "views": true } }))
            .await?;
        let by_author = self
            .db
            .post()
            .group_by(json!({ "by": ["authorId"], "_count": { "_all": true }, "orderBy": { "_count": { "_all": "desc" } } }))
            .await?;
        let published = self.db.post().count(vec![post::published::equals(true)]).await?;
        let authors = self.db.user().count(vec![user::posts::some(vec![])]).await?;
        Ok(json!({ "totals": totals, "published": published, "authorsWithPosts": authors, "byAuthor": by_author }))
    }
}
