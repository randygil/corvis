use corvis::Validate;
use corvis_swagger::ApiSchema;
use serde::{Deserialize, Serialize};

use super::entities::Cat;

#[derive(Debug, Clone, Deserialize, Validate, ApiSchema)]
pub struct CreateCatDto {
    #[api_property(description = "Name of the cat", example = "Tom")]
    #[is_string]
    #[is_not_empty]
    #[max_length(50)]
    pub name: String,
    #[is_int]
    #[min(0)]
    #[max(40)]
    #[api_property(description = "Age in years", example = 3)]
    pub age: u32,
    #[api_property(description = "Breed", example = "siamese")]
    #[is_string]
    #[is_in("persian", "siamese", "maine coon", "mixed")]
    pub breed: String,
}

/// `PartialType(CreateCatDto)`.
#[derive(Debug, Clone, Deserialize, Validate, ApiSchema)]
pub struct UpdateCatDto {
    #[is_optional]
    #[is_string]
    #[max_length(50)]
    pub name: Option<String>,
    #[is_optional]
    #[is_int]
    #[min(0)]
    pub age: Option<u32>,
    #[is_optional]
    #[is_in("persian", "siamese", "maine coon", "mixed")]
    pub breed: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Validate, ApiSchema)]
pub struct ListCatsQuery {
    #[is_optional]
    #[is_int]
    #[min(1)]
    pub page: Option<u32>,
    #[is_optional]
    #[is_int]
    #[min(1)]
    #[max(100)]
    pub limit: Option<u32>,
    #[is_optional]
    pub breed: Option<String>,
}

/// `GET /cats/count`.
#[derive(Debug, Serialize)]
pub struct CountDto {
    pub count: usize,
}

/// `GET /v2/cats/:id`: the cat and the API version that answered.
#[derive(Debug, Serialize)]
pub struct CatV2Dto {
    pub cat: Cat,
    #[serde(rename = "apiVersion")]
    pub api_version: u32,
}
