use corvis::WsClient;
use corvis::prelude::*;
use serde::{Deserialize, Serialize};

pub const MODERATOR: &str = "moderator";

/// The authenticated user: the JWT claims, attached to the HTTP request by `AuthGuard` and
/// to the WebSocket client by the gateway on connect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatUser {
    pub username: String,
    pub role: String,
}

impl ChatUser {
    pub fn is_moderator(&self) -> bool {
        self.role == MODERATOR
    }

    /// The user of an HTTP request or of a WebSocket message, whichever `ctx` is.
    pub fn of(ctx: &ExecutionContext) -> Option<ChatUser> {
        match ctx.get_type() {
            corvis::ContextType::Ws => ctx.switch_to_ws().get_client::<WsClient>().and_then(|c| c.get::<ChatUser>()),
            _ => ctx.request().get::<ChatUser>(),
        }
    }
}

/// `@CurrentUser()` for HTTP handlers protected by `AuthGuard`.
#[async_trait]
impl FromContext for ChatUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ChatUser::of(ctx).ok_or_else(|| UnauthorizedException::new("Unauthorized"))
    }
}
