use corvis::prelude::*;
use corvis_orm::OrmResultExt;

use super::dto::{CreateUserDto, ListUsersQuery, UpdateUserDto};
use super::users_service::UsersService;
use crate::models::{Paginated, User, user};

#[controller("users")]
pub struct UsersController {
    users_service: Arc<UsersService>,
}

#[routes]
impl UsersController {
    /// 201, or 409 when the email is taken (P2002).
    #[post]
    async fn create(&self, #[body] dto: CreateUserDto) -> HttpResult<User> {
        // `.http()` maps ORM errors without needing the global filter.
        self.users_service.create(dto).await.http()
    }

    #[get]
    async fn find_all(&self, #[query] query: ListUsersQuery) -> HttpResult<Paginated<user::WithCount>> {
        // `?` turns the OrmError into a 500 carrying it as cause; the global
        // `OrmExceptionFilter` maps it to the right status.
        Ok(self.users_service.find_all(query).await?)
    }

    #[get(":id")]
    async fn find_one(&self, #[param("id", ParseIntPipe)] id: i64) -> HttpResult<User> {
        Ok(self.users_service.find_one(id).await?)
    }

    #[patch(":id")]
    async fn update(&self, #[param("id", ParseIntPipe)] id: i64, #[body] dto: UpdateUserDto) -> HttpResult<User> {
        Ok(self.users_service.update(id, dto).await?)
    }

    /// Deleting a user cascades to its posts (`onDelete: Cascade`).
    #[delete(":id")]
    async fn remove(&self, #[param("id", ParseIntPipe)] id: i64) -> HttpResult<User> {
        Ok(self.users_service.remove(id).await?)
    }
}
