use corvis::auth::local::{LocalValidator, PasswordError, PasswordHasher};
use corvis::prelude::*;

use crate::users::{Account, UsersService};

/// The validator of the `local` strategy (`passport-local`): username and password.
///
/// - `verify_or_dummy` spends one real verification whether the user exists or not, so the response time does not say
///   which usernames exist. Most stored hashes here are argon2id, which is what the dummy hash uses; while a database
///   still holds mostly bcrypt hashes, set `dummy_algorithm` to bcrypt (see "Encryption and hashing").
/// - A hash made by an older algorithm is replaced after a successful login (`needs_rehash`), with the password the
///   user just proved.
/// - A wrong password and an unknown user are the same `Ok(None)`: the same `401`.
#[injectable]
pub struct LocalAuth {
    users: Arc<UsersService>,
    hasher: Arc<PasswordHasher>,
}

#[async_trait]
impl LocalValidator for LocalAuth {
    type User = Account;

    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<Account>, HttpException> {
        let found = self.users.credentials(username);
        let stored = found.as_ref().map(|(_, hash)| hash.as_str());
        // A busy hasher is a 503 for every user, never a hint about this one.
        let ok = self.hasher.verify_or_dummy(password, stored).await.map_err(PasswordError::into_http_exception)?;
        let Some((account, hash)) = found.filter(|_| ok) else { return Ok(None) };
        if self.hasher.needs_rehash(&hash) {
            let upgraded = self.hasher.hash(password).await.map_err(PasswordError::into_http_exception)?;
            self.users.set_password_hash(account.id, upgraded);
        }
        Ok(Some(account))
    }
}
