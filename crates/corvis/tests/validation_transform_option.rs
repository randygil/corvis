//! The deprecated `ValidationPipe::transform` option changes nothing: values are always converted.

use corvis::prelude::*;
use corvis::testing::{Test, request};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, corvis::Validate)]
pub struct PageDto {
    #[is_int]
    pub page: u32,
}

#[controller("pages")]
pub struct PagesController;

#[routes]
impl PagesController {
    #[get]
    async fn get(&self, #[query] q: PageDto) -> Value {
        json!({ "page": q.page })
    }
}

async fn answer(pipe: ValidationPipe) -> (u16, Value) {
    let module = Test::create_testing_module().controller::<PagesController>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.use_global_pipes(pipe);
    app.init().await.unwrap();
    let res = request(&app).get("/pages?page=2").send().await;
    (res.status(), res.json())
}

#[corvis::test]
#[allow(deprecated)]
async fn transform_false_answers_like_the_default() {
    let default = answer(ValidationPipe::new()).await;
    let off = answer(ValidationPipe::new().transform(false)).await;
    assert_eq!(default, off);
    assert_eq!(default.0, 200);
    assert_eq!(default.1, json!({ "page": 2 }));
}
