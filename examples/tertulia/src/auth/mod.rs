mod auth_controller;
mod auth_guard;
mod auth_service;
mod dto;
mod user;

use corvis::prelude::*;

pub use auth_controller::AuthController;
pub use auth_guard::{AuthGuard, ROLES_KEY, RolesGuard};
pub use auth_service::AuthService;
pub use dto::LoginDto;
pub use user::{ChatUser, MODERATOR};

#[module(controllers = [AuthController], providers = [AuthService], exports = [AuthService])]
pub struct AuthModule;
