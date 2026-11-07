use corvis::Validate;
use serde::Deserialize;

#[derive(Debug, Deserialize, Validate)]
pub struct SignInDto {
    #[is_string]
    #[is_not_empty]
    pub username: String,
    #[is_string]
    #[min_length(4)]
    pub password: String,
}
