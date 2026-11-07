//! DTOs validated with `corvis::z` schemas (zod-like) through the global `ZodValidationPipe`.

use corvis::prelude::*;
use corvis::zod_dto;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePostDto {
    pub title: String,
    pub content: Option<String>,
    pub published: Option<bool>,
    /// Email of an existing user (nested `connect`; 404 if it does not exist).
    pub author_email: String,
}

zod_dto!(
    CreatePostDto,
    z::object! {
        title: z::string().trim().min(1).max(200),
        content: z::string().optional(),
        published: z::boolean().optional(),
        authorEmail: z::string().email(),
    }
    .strict()
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPostsQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    /// Matches title or content (case-insensitive).
    pub search: Option<String>,
    pub published: Option<bool>,
}

// Query strings arrive as text: `z::coerce` converts them before validating.
zod_dto!(
    ListPostsQuery,
    z::object! {
        page: z::coerce::number().int().min(1).optional(),
        limit: z::coerce::number().int().min(1).max(100).optional(),
        search: z::string().optional(),
        published: z::coerce::boolean().optional(),
    }
    .strict()
);
