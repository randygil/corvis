use corvis::prelude::*;

use crate::common::decorators::Role;

#[derive(Debug, Clone)]
pub struct User {
    pub user_id: u32,
    pub username: String,
    /// An argon2id hash (PHC string), never the password.
    pub password_hash: String,
    pub roles: Vec<Role>,
}

/// In-memory users store (the classic authentication guide example). The passwords are stored as argon2id hashes made
/// with `PasswordHasher::default()`: the sign-in passwords are `changeme` (john) and `guess` (maria).
#[injectable]
pub struct UsersService {
    #[init(vec![
        User { user_id: 1, username: "john".into(), password_hash: "$argon2id$v=19$m=19456,t=2,p=1$P4ya+YMobDOxFT8wzO4xZQ$QsrLi/0rL5lnrI8kIspWhPMq87FstrqvlsOX3Hr9DCk".into(), roles: vec![Role::Admin] },
        User { user_id: 2, username: "maria".into(), password_hash: "$argon2id$v=19$m=19456,t=2,p=1$+uopZgnZJiN+kjylO3rjLw$nRjOY5mRUJ3f6LHmz0B0y/0BWApPxbAj20jhWSlNKf0".into(), roles: vec![Role::User] },
    ])]
    users: Vec<User>,
}

impl UsersService {
    pub async fn find_one(&self, username: &str) -> Option<User> {
        self.users.iter().find(|u| u.username == username).cloned()
    }

    pub async fn find_by_id(&self, user_id: u32) -> Option<User> {
        self.users.iter().find(|u| u.user_id == user_id).cloned()
    }
}
