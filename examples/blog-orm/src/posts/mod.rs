mod dto;
mod posts_controller;
mod posts_service;

use corvis::prelude::*;

pub use posts_controller::PostsController;
pub use posts_service::PostsService;

#[module(controllers = [PostsController], providers = [PostsService])]
pub struct PostsModule;
