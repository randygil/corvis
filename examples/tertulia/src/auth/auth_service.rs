use corvis::JwtService;
use corvis::prelude::*;

use super::user::{ChatUser, MODERATOR};

/// Password-less demo login: any valid nickname gets a token. Names listed in
/// `MODERATORS` (comma separated, default `admin`) get the `moderator` role.
#[injectable]
pub struct AuthService {
    jwt: Arc<JwtService>,
    config: Arc<ConfigService>,
}

impl AuthService {
    pub fn login(&self, username: &str) -> HttpResult<Value> {
        let moderators: String = self.config.get_or("MODERATORS", "admin".to_string());
        let is_moderator = moderators.split(',').any(|m| m.trim().eq_ignore_ascii_case(username));
        let user = ChatUser { username: username.to_string(), role: if is_moderator { MODERATOR } else { "member" }.to_string() };
        let token = self.jwt.sign(&user).map_err(HttpException::from_anyhow)?;
        Ok(json!({ "accessToken": token, "user": user }))
    }
}
