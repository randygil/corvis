//! ```text
//! model Room {
//!   name       String   @id
//!   topic      String
//!   maxMembers Int      @default(50) @map("max_members")
//!   createdBy  String   @map("created_by")
//!   createdAt  DateTime @default(now()) @map("created_at")
//!   @@map("rooms")
//! }
//!
//! model ChatMessage {
//!   id        Int      @id @default(autoincrement())
//!   room      String
//!   username  String
//!   text      String
//!   createdAt DateTime @default(now()) @map("created_at")
//!   @@map("messages")
//! }
//! ```

use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "rooms")]
pub struct Room {
    #[id]
    pub name: String,
    pub topic: String,
    #[default(50)]
    pub max_members: i64,
    pub created_by: String,
    #[default(now)]
    pub created_at: corvis_orm::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "messages")]
pub struct ChatMessage {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    pub room: String,
    pub username: String,
    pub text: String,
    #[default(now)]
    pub created_at: corvis_orm::DateTime,
}
