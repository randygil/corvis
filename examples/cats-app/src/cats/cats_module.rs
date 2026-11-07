use corvis::prelude::*;

use super::cat_created_listener::CatCreatedListener;
use super::cats_controller::CatsController;
use super::cats_service::CatsService;

#[module(controllers = [CatsController], providers = [CatsService, CatCreatedListener], exports = [CatsService])]
pub struct CatsModule;
