use corvis::Validate;
use corvis_swagger::ApiSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Validate, ApiSchema)]
pub struct SignInDto {
    #[is_string]
    #[is_not_empty]
    pub username: String,
    #[is_string]
    #[min_length(4)]
    pub password: String,
}

/// The body of a successful sign-in.
#[derive(Debug, Serialize)]
pub struct TokenDto {
    pub access_token: String,
}
