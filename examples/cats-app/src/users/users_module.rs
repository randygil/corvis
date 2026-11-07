use corvis::prelude::*;

use super::UsersService;

#[module(providers = [UsersService], exports = [UsersService])]
pub struct UsersModule;
