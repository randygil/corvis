use corvis_swagger::ApiSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, ApiSchema)]
#[serde(rename_all = "camelCase")]
#[api_schema(description = "A cat of the shelter")]
pub struct Cat {
    #[api_property(description = "Identifier", example = 1)]
    pub id: u32,
    pub name: String,
    pub age: u32,
    pub breed: String,
    #[api_property(description = "Id of the user that created the cat", example = 1)]
    pub owner_id: u32,
}

/// Payload of the `cat.created` event.
#[derive(Debug, Clone)]
pub struct CatCreatedEvent {
    pub cat: Cat,
}
