//! The users, in memory. A real application reads them from a database; the rules shown here do not change.
//!
//! No password is stored in plain text: `john` and `maria` have argon2id hashes, and `walt` still has the bcrypt hash a
//! Node application made, which the login replaces by an argon2id hash the first time `walt` signs in. The demo
//! passwords are in the README. The hashes were made with:
//!
//! ```text
//! PasswordHasher::new(PasswordHasherOptions::default())?.hash(password).await     // john, maria
//! PasswordHasher::new(PasswordHasherOptions::bcrypt(10))?.hash(password).await    // walt (what bcryptjs makes)
//! ```

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const ROLES_KEY: &str = "roles";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}

/// The user every strategy of the application hands to the routes: the token, the session and the provider login all
/// produce this one. It holds no password hash and no e-mail address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: u32,
    pub username: String,
    pub roles: Vec<Role>,
}

#[derive(Debug, Clone)]
struct UserRecord {
    account: Account,
    email: String,
    password_hash: String,
}

const JOHN_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$a1a4SMl8dhNY7ZBOvKyV+g$Lzb1DU/58iMWo+RIJdYzwonfgPIxVin5dsgCTnBXPvM";
const MARIA_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$ducsdHJOdWLASGbEumHgNg$w1DkkDXSigp+SQS102gVbjRfmCF4FWO5BW7wSi0FJ5k";
const WALT_BCRYPT_HASH: &str = "$2b$10$OPb2rMS5elBPhh.hyky/J.dsFRvnxuT89P3AR9zAAwjx4ZgkyQ1WW";

pub struct UsersService {
    users: Mutex<Vec<UserRecord>>,
    /// `(provider, id)` -> the user id of the linked account.
    links: Mutex<HashMap<(String, String), u32>>,
}

impl UsersService {
    pub fn seeded() -> UsersService {
        let user = |id: u32, name: &str, roles: Vec<Role>, hash: &str| UserRecord {
            account: Account { id, username: name.to_string(), roles },
            email: format!("{name}@example.test"),
            password_hash: hash.to_string(),
        };
        UsersService {
            users: Mutex::new(vec![
                user(1, "john", vec![Role::Admin], JOHN_HASH),
                user(2, "maria", vec![Role::User], MARIA_HASH),
                user(3, "walt", vec![Role::User], WALT_BCRYPT_HASH),
            ]),
            links: Mutex::new(HashMap::new()),
        }
    }

    /// The account and the stored hash of `username`. An account made by a provider login has no password.
    pub fn credentials(&self, username: &str) -> Option<(Account, String)> {
        let users = self.users.lock().unwrap_or_else(|e| e.into_inner());
        users.iter().find(|u| u.account.username == username && !u.password_hash.is_empty()).map(|u| (u.account.clone(), u.password_hash.clone()))
    }

    /// Every account, without its hash or its e-mail address.
    pub fn accounts(&self) -> Vec<Account> {
        self.users.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|u| u.account.clone()).collect()
    }

    pub fn find_by_id(&self, id: u32) -> Option<Account> {
        let users = self.users.lock().unwrap_or_else(|e| e.into_inner());
        users.iter().find(|u| u.account.id == id).map(|u| u.account.clone())
    }

    /// Replaces the hash of a user (after a login with an old one).
    pub fn set_password_hash(&self, id: u32, hash: String) {
        let mut users = self.users.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(user) = users.iter_mut().find(|u| u.account.id == id) {
            user.password_hash = hash;
        }
    }

    /// The stored hash of `username`, for the tests that check which algorithm it uses.
    pub fn password_hash_of(&self, username: &str) -> Option<String> {
        self.credentials(username).map(|(_, hash)| hash)
    }

    /// The account of a user of an OAuth2 provider.
    ///
    /// The account is found by `(provider, id)`: the provider's own identifier of the user, never the e-mail address
    /// alone. An address is a claim the provider may not have checked, and one that changes hands. Only an address the
    /// provider reports as verified (`verified_email`) may link the provider's user to an account that already exists;
    /// otherwise a new account is created. The link is stored, so the next login does not depend on the address.
    pub fn for_provider_user(&self, provider: &str, id: &str, verified_email: Option<&str>, display_name: &str) -> Account {
        let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
        let mut users = self.users.lock().unwrap_or_else(|e| e.into_inner());
        let key = (provider.to_string(), id.to_string());
        if let Some(user_id) = links.get(&key)
            && let Some(user) = users.iter().find(|u| u.account.id == *user_id)
        {
            return user.account.clone();
        }
        let linked = verified_email.and_then(|email| users.iter().find(|u| u.email.eq_ignore_ascii_case(email)));
        if let Some(user) = linked {
            links.insert(key, user.account.id);
            return user.account.clone();
        }
        // Ids from 1000 on, so they never clash with the seeded users.
        let next = 1000 + links.len() as u32;
        let account = Account { id: next, username: format!("{provider}:{display_name}"), roles: vec![Role::User] };
        links.insert(key, next);
        // The e-mail is left empty: an address nobody verified must never match a later login.
        users.push(UserRecord { account: account.clone(), email: String::new(), password_hash: String::new() });
        account
    }
}
