use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cat {
    pub id: u32,
    pub name: String,
    pub age: u32,
    pub breed: String,
    pub owner_id: u32,
}

/// Payload of the `cat.created` event.
#[derive(Debug, Clone)]
pub struct CatCreatedEvent {
    pub cat: Cat,
}
