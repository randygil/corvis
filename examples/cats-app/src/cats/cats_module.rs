use corvis::prelude::*;

use super::cat_created_listener::CatCreatedListener;
use super::cats_controller::CatsController;
use super::cats_service::CatsService;
use crate::common::request_context::RequestContext;

#[module(
    controllers = [CatsController],
    providers = [CatsService, CatCreatedListener, RequestContext],
    exports = [CatsService]
)]
pub struct CatsModule;
