use corvis::prelude::*;
use corvis_orm::{OrmError, OrmService, SortOrder, or};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::dto::{CreatePostDto, ListPostsQuery};
use crate::models::{Paginated, Post, PostDelegate, UserDelegate, post, user};

/// `{ _count: { _all } }` of `aggregate` and `groupBy`.
#[derive(Debug, Serialize, Deserialize)]
pub struct CountAll {
    #[serde(rename = "_all")]
    pub all: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ViewsSum {
    pub views: Option<i64>,
}

/// `_avg` keeps the engine's number as is (`0` stays `0`, not `0.0`).
#[derive(Debug, Serialize, Deserialize)]
pub struct ViewsAvg {
    pub views: Option<Number>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Totals {
    #[serde(rename = "_count")]
    pub count: CountAll,
    #[serde(rename = "_sum")]
    pub sum: ViewsSum,
    #[serde(rename = "_avg")]
    pub avg: ViewsAvg,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthorPosts {
    #[serde(rename = "_count")]
    pub count: CountAll,
    #[serde(rename = "authorId")]
    pub author_id: i64,
}

/// `GET /posts/stats`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub totals: Totals,
    pub published: i64,
    pub authors_with_posts: i64,
    pub by_author: Vec<AuthorPosts>,
}

#[injectable]
pub struct PostsService {
    db: Arc<OrmService>,
}

impl PostsService {
    /// Nested write: `author: { connect: { email } }` (P2025 -> 404 if the author does not exist).
    pub async fn create(&self, dto: CreatePostDto) -> Result<Post, OrmError> {
        let mut data = vec![
            post::title::set(dto.title),
            post::published::set(dto.published.unwrap_or(false)),
            post::author::connect(user::email::equals(dto.author_email)),
        ];
        if let Some(content) = dto.content {
            data.push(post::content::set(content));
        }
        self.db.post().create(data).include(post::author::include()).await
    }

    /// Paginated search (typed style) including the author.
    pub async fn find_all(&self, q: ListPostsQuery) -> Result<Paginated<Post>, OrmError> {
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
        Ok(Paginated::new(posts, total, page as u32, limit as u32))
    }

    pub async fn find_one(&self, id: i64) -> Result<Post, OrmError> {
        self.db.post().find_unique_or_throw(post::id::equals(id)).include(post::author::include()).await
    }

    /// `update` + `increment` – P2025 -> 404.
    pub async fn publish(&self, id: i64) -> Result<Post, OrmError> {
        self.db.post().update(post::id::equals(id)).data(vec![post::published::set(true), post::views::increment(1)]).await
    }

    pub async fn remove(&self, id: i64) -> Result<Post, OrmError> {
        self.db.post().delete(post::id::equals(id)).await
    }

    /// `aggregate` and `groupBy` return JSON; `.exec_as::<T>()` reads them into example-local structs.
    pub async fn stats(&self) -> Result<Stats, OrmError> {
        let totals = self
            .db
            .post()
            .aggregate(json!({ "_count": { "_all": true }, "_sum": { "views": true }, "_avg": { "views": true } }))
            .exec_as::<Totals>()
            .await?;
        let by_author = self
            .db
            .post()
            .group_by(json!({ "by": ["authorId"], "_count": { "_all": true }, "orderBy": { "_count": { "_all": "desc" } } }))
            .exec_as::<Vec<AuthorPosts>>()
            .await?;
        let published = self.db.post().count(vec![post::published::equals(true)]).await?;
        let authors_with_posts = self.db.user().count(vec![user::posts::some(vec![])]).await?;
        Ok(Stats { totals, published, authors_with_posts, by_author })
    }
}
