//! DTOs validated with `corvis::z` schemas (zod-like) through the global `ZodValidationPipe`.

use corvis::prelude::*;
use corvis::zod_dto;
use serde::{Deserialize, Serialize};

fn create_user_schema() -> z::ZObject {
    z::object! {
        email: z::string().trim().to_lower_case().email(),
        name: z::string().trim().max(80).optional(),
    }
    .strict()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserDto {
    pub email: String,
    pub name: Option<String>,
}

zod_dto!(CreateUserDto, create_user_schema());

/// `PartialType(CreateUserDto)` → `createUserSchema.partial()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserDto {
    pub email: Option<String>,
    pub name: Option<String>,
}

zod_dto!(UpdateUserDto, create_user_schema().partial());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListUsersQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    /// Matches email or name (case-insensitive).
    pub search: Option<String>,
}

zod_dto!(
    ListUsersQuery,
    z::object! {
        page: z::coerce::number().int().min(1).optional(),
        limit: z::coerce::number().int().min(1).max(100).optional(),
        search: z::string().optional(),
    }
    .strict()
);
