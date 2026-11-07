use corvis::Validate;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct CreateCatDto {
    #[is_string]
    #[is_not_empty]
    #[max_length(50)]
    pub name: String,
    #[is_int]
    #[min(0)]
    #[max(40)]
    pub age: u32,
    #[is_string]
    #[is_in("persian", "siamese", "maine coon", "mixed")]
    pub breed: String,
}

/// `PartialType(CreateCatDto)`.
#[derive(Debug, Clone, Deserialize, Validate)]
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

#[derive(Debug, Clone, Deserialize, Validate)]
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
