use corvis::Validate;
use corvis_swagger::ApiSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, Validate, ApiSchema)]
pub struct SignInDto {
    #[is_string]
    #[is_not_empty]
    pub username: String,
    #[is_string]
    #[min_length(4)]
    pub password: String,
}
