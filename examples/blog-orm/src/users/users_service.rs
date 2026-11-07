use corvis::prelude::*;
use corvis_orm::{OrmError, OrmService};

use super::dto::{CreateUserDto, ListUsersQuery, UpdateUserDto};
use crate::models::{User, UserDelegate, paginated, user};

#[injectable]
pub struct UsersService {
    db: Arc<OrmService>,
}

impl UsersService {
    pub async fn create(&self, dto: CreateUserDto) -> Result<User, OrmError> {
        self.db.user().create(json!({ "data": dto })).await
    }

    /// Paginated list with a post counter per user (JSON query arguments).
    pub async fn find_all(&self, q: ListUsersQuery) -> Result<Value, OrmError> {
        let page = q.page.unwrap_or(1);
        let limit = q.limit.unwrap_or(10);
        let filter = match &q.search {
            Some(s) => json!({ "OR": [
                { "email": { "contains": s, "mode": "insensitive" } },
                { "name": { "contains": s, "mode": "insensitive" } }
            ] }),
            None => json!({}),
        };
        let total = self.db.user().count(json!({ "where": filter })).await?;
        let data = self
            .db
            .user()
            .find_many(json!({
                "where": filter,
                "orderBy": { "id": "asc" },
                "skip": (page - 1) * limit,
                "take": limit,
                "include": { "_count": { "select": { "posts": true } } }
            }))
            .exec_json()
            .await?;
        Ok(paginated(data, total, page, limit))
    }

    /// `findUniqueOrThrow` including the posts, newest first (typed style). P2025 -> 404.
    pub async fn find_one(&self, id: i64) -> Result<User, OrmError> {
        self.db
            .user()
            .find_unique_or_throw(user::id::equals(id))
            .include(user::posts::include_args(json!({ "orderBy": { "createdAt": "desc" } })))
            .await
    }

    pub async fn update(&self, id: i64, dto: UpdateUserDto) -> Result<User, OrmError> {
        let mut data = Vec::new();
        if let Some(email) = dto.email {
            data.push(user::email::set(email));
        }
        if let Some(name) = dto.name {
            data.push(user::name::set(name));
        }
        self.db.user().update(user::id::equals(id)).data(data).await
    }

    pub async fn remove(&self, id: i64) -> Result<User, OrmError> {
        self.db.user().delete(json!({ "where": { "id": id } })).await
    }
}
