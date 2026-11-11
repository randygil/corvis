use corvis::auth::session::{SessionCookie, SessionModule, SessionOptions, SessionSerializer};
use corvis::prelude::*;

use crate::settings::Settings;
use crate::users::{Account, UsersService};

/// `serializeUser` and `deserializeUser`: the session stores the id of the user and reads the user again on every
/// request, so a user removed after logging in is anonymous from the next request on. The stored value is an object,
/// never a bare `0` or `""`, which would not authenticate.
#[injectable]
pub struct UserSerializer {
    users: Arc<UsersService>,
}

#[async_trait]
impl SessionSerializer for UserSerializer {
    type User = Account;

    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!({ "id": user.id }))
    }

    async fn deserialize_user(&self, _req: &Request, stored: Value) -> Result<Option<Account>, HttpException> {
        let Some(id) = stored.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else { return Ok(None) };
        Ok(self.users.find_by_id(id))
    }
}

/// The session module. In a deployment the cookie is `__Host-sid`, `Secure`, `HttpOnly` and `SameSite=Lax`: the
/// `__Host-` prefix makes a browser refuse a cookie planted from a sibling subdomain. Only `--dev` (plain HTTP on
/// loopback) drops `Secure` and, with it, the prefix.
pub fn session_module() -> DynamicModule {
    SessionModule::for_root_async(|ctx| async move {
        let settings = ctx.get::<Settings>().await?;
        let options = SessionOptions::with_secrets([settings.session_secret.clone()])?;
        Ok(if settings.dev { options.name("sid").cookie(SessionCookie::default().secure(false)) } else { options.name("__Host-sid") })
    })
}
