use corvis::auth::session::{SessionCookie, SessionModule, SessionOptions, SessionSerializer};
use corvis::prelude::*;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::users::UsersService;

/// The user a session carries: what `GET /session/me` shows. The login strategy hands over a richer user (with its
/// roles); the session keeps only the id and looks the user up again on every request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub user_id: u32,
    pub username: String,
}

/// `serializeUser` and `deserializeUser`: the session stores the user's id and reads the user again from the store on
/// every request, so a user removed after logging in is anonymous from the next request on.
#[injectable]
pub struct MemberSerializer {
    users: Arc<UsersService>,
}

#[async_trait]
impl SessionSerializer for MemberSerializer {
    type User = Member;

    async fn serialize_user(&self, user: &Member) -> Result<Value, HttpException> {
        Ok(json!(user.user_id))
    }

    async fn deserialize_user(&self, _req: &Request, stored: Value) -> Result<Option<Member>, HttpException> {
        let Some(id) = stored.as_u64().and_then(|id| u32::try_from(id).ok()) else { return Ok(None) };
        Ok(self.users.find_by_id(id).await.map(|u| Member { user_id: u.user_id, username: u.username }))
    }
}

/// The session module, configured from the environment:
///
/// - `SESSION_SECRET` signs the session cookie and must have at least 32 bytes (`openssl rand -hex 32`); a shorter
///   value stops the application. Without it a random secret is generated at every start, which is as long-lived as
///   the in-memory session store.
/// - `SESSION_COOKIE_SECURE=false` sends the cookie without `Secure`, for plain-HTTP development only. The default
///   keeps it on.
pub fn session_module() -> DynamicModule {
    SessionModule::for_root_async(|ctx| async move {
        let config = ctx.get::<ConfigService>().await?;
        let secret = match config.get::<String>("SESSION_SECRET").filter(|s| !s.is_empty()) {
            Some(secret) => secret,
            None => random_secret(),
        };
        let mut cookie = SessionCookie::default();
        if config.get::<String>("SESSION_COOKIE_SECURE").as_deref() == Some("false") {
            cookie = cookie.secure(false);
        }
        Ok(SessionOptions::with_secrets([secret])?.cookie(cookie))
    })
}

/// 32 random bytes as hex, from the operating system's generator.
fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
