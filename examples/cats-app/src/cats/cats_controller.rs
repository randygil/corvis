use std::time::Duration;

use corvis::prelude::*;
use corvis_swagger::*;

use super::cats_service::CatsService;
use super::dto::{CatV2Dto, CountDto, CreateCatDto, ListCatsQuery, UpdateCatDto};
use super::entities::Cat;
use crate::common::decorators::{CurrentUser, ROLES_KEY, Role};
use crate::common::interceptors::{RequestIdInterceptor, TimingInterceptor, TransformInterceptor};
use crate::common::request_context::RequestContext;

#[controller("cats")]
#[api_tags("cats")]
#[use_interceptors(TimingInterceptor, RequestIdInterceptor)]
pub struct CatsController {
    cats_service: Arc<CatsService>,
}

#[routes]
impl CatsController {
    #[post]
    #[api_bearer_auth]
    #[api_operation(summary = "Create a cat owned by the signed-in user")]
    #[api_created_response(type = Cat)]
    #[api_unauthorized_response(description = "Missing or invalid token")]
    fn create(&self, #[body] dto: CreateCatDto, user: CurrentUser) -> Cat {
        self.cats_service.create(dto, user.0.sub)
    }

    #[get]
    #[public]
    #[use_interceptors(TransformInterceptor)]
    #[api_operation(summary = "List cats")]
    fn find_all(&self, #[query] query: ListCatsQuery) -> Vec<Cat> {
        self.cats_service.find_all(&query)
    }

    #[get("count")]
    #[public]
    #[use_interceptors(CacheInterceptor)]
    #[set_metadata(CACHE_TTL_METADATA, 5_000)]
    fn count(&self) -> CountDto {
        CountDto { count: self.cats_service.count() }
    }

    /// The id of this request, from the request-scoped `RequestContext` (`X-Request-Id` or generated).
    #[get("request-id")]
    #[public]
    fn request_id(&self, ctx: Scoped<RequestContext>) -> Value {
        json!({ "requestId": ctx.request_id() })
    }

    /// Server-sent events: `curl -N localhost:3000/api/v1/cats/stream`.
    #[sse("stream")]
    #[public]
    fn stream(&self) -> Sse {
        let svc = self.cats_service.clone();
        let stream = futures::stream::unfold(0u32, move |n| {
            let svc = svc.clone();
            async move {
                if n > 0 {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                (n < 5).then(|| (MessageEvent::new(json!({ "tick": n, "cats": svc.count() })).id(n.to_string()), n + 1))
            }
        });
        Sse::new(stream)
    }

    #[get(":id")]
    #[public]
    #[api_ok_response(type = Cat)]
    #[api_not_found_response(description = "No cat with that id")]
    fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
        self.cats_service.find_one(id)
    }

    #[get(":id")]
    #[version("2")]
    #[public]
    fn find_one_v2(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<CatV2Dto> {
        let cat = self.cats_service.find_one(id)?;
        Ok(CatV2Dto { cat, api_version: 2 })
    }

    #[patch(":id")]
    #[api_bearer_auth]
    fn update(&self, #[param("id", ParseIntPipe)] id: u32, #[body] dto: UpdateCatDto) -> HttpResult<Cat> {
        self.cats_service.update(id, dto)
    }

    #[delete(":id")]
    #[api_bearer_auth]
    #[api_forbidden_response(description = "Admins only")]
    #[http_code(HttpStatus::NO_CONTENT)]
    #[set_metadata(ROLES_KEY, [Role::Admin])]
    fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> {
        self.cats_service.remove(id)
    }
}

use corvis::futures;
use corvis::tokio;
