use corvis::prelude::*;
use corvis::zod_dto;
use serde::Deserialize;

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
