use corvis::prelude::*;
use corvis_orm::{Data, OrmError, OrmService, SortOrder, or};

use super::dto::{CreateUserDto, ListUsersQuery, UpdateUserDto};
use crate::models::{Paginated, User, UserDelegate, user};

#[injectable]
pub struct UsersService {
    db: Arc<OrmService>,
}

impl UsersService {
    /// `Data` writes a serializable DTO as the data of the query.
    pub async fn create(&self, dto: CreateUserDto) -> Result<User, OrmError> {
        self.db.user().create(Data(&dto)).await
    }

    /// Paginated list with a post counter per user: `user::posts::count()` adds `_count`, and
    /// `user::WithCount` reads it back next to the model fields.
    pub async fn find_all(&self, q: ListUsersQuery) -> Result<Paginated<user::WithCount>, OrmError> {
        let page = q.page.unwrap_or(1);
        let limit = q.limit.unwrap_or(10);
        let filter = match &q.search {
            Some(s) => vec![or(vec![user::email::contains_insensitive(s.clone()), user::name::contains_insensitive(s.clone())])],
            None => vec![],
        };
        let total = self.db.user().count(filter.clone()).await?;
        let data = self
            .db
            .user()
            .find_many(filter)
            .order_by(user::id::order(SortOrder::Asc))
            .skip(i64::from((page - 1) * limit))
            .take(i64::from(limit))
            .include(user::posts::count())
            .exec_as::<Vec<user::WithCount>>()
            .await?;
        Ok(Paginated::new(data, total, page, limit))
    }

    /// `findUniqueOrThrow` including the posts, newest first (typed style). P2025 -> 404.
    pub async fn find_one(&self, id: i64) -> Result<User, OrmError> {
        self.db
            .user()
            .find_unique_or_throw(user::id::equals(id))
            .include(user::posts::include_args(json!({ "orderBy": { "createdAt": "desc" } })))
            .await
    }

    /// Only the fields the DTO carries change (`None` fields are skipped when serializing).
    pub async fn update(&self, id: i64, dto: UpdateUserDto) -> Result<User, OrmError> {
        self.db.user().update(user::id::equals(id)).data(Data(&dto)).await
    }

    pub async fn remove(&self, id: i64) -> Result<User, OrmError> {
        self.db.user().delete(user::id::equals(id)).await
    }
}
