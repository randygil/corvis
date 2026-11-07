use corvis::prelude::*;

use super::dto::{CreatePostDto, ListPostsQuery};
use super::posts_service::PostsService;
use crate::models::Post;

#[controller("posts")]
pub struct PostsController {
    posts_service: Arc<PostsService>,
}

#[routes]
impl PostsController {
    #[post]
    async fn create(&self, #[body] dto: CreatePostDto) -> HttpResult<Post> {
        Ok(self.posts_service.create(dto).await?)
    }

    #[get]
    async fn find_all(&self, #[query] query: ListPostsQuery) -> HttpResult<Value> {
        Ok(self.posts_service.find_all(query).await?)
    }

    #[get("stats")]
    async fn stats(&self) -> HttpResult<Value> {
        Ok(self.posts_service.stats().await?)
    }

    #[get(":id")]
    async fn find_one(&self, #[param("id", ParseIntPipe)] id: i64) -> HttpResult<Post> {
        Ok(self.posts_service.find_one(id).await?)
    }

    #[patch(":id/publish")]
    async fn publish(&self, #[param("id", ParseIntPipe)] id: i64) -> HttpResult<Post> {
        Ok(self.posts_service.publish(id).await?)
    }

    #[delete(":id")]
    #[http_code(HttpStatus::NO_CONTENT)]
    async fn remove(&self, #[param("id", ParseIntPipe)] id: i64) -> HttpResult<()> {
        self.posts_service.remove(id).await?;
        Ok(())
    }
}
