mod dto;
mod users_controller;
mod users_service;

use corvis::prelude::*;

pub use users_controller::UsersController;
pub use users_service::UsersService;

#[module(controllers = [UsersController], providers = [UsersService], exports = [UsersService])]
pub struct UsersModule;
