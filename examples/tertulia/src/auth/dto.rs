use corvis::prelude::*;
use corvis::zod_dto;
use serde::{Deserialize, Serialize};

use super::user::ChatUser;

/// Answer of `POST /api/auth/login`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub access_token: String,
    pub user: ChatUser,
}

#[derive(Debug, Deserialize)]
pub struct LoginDto {
    pub username: String,
}

zod_dto!(
    LoginDto,
    z::strict_object! {
        username: z::string().trim().min(2).max(20).regex("^[A-Za-z0-9_.-]+$"),
    }
);
