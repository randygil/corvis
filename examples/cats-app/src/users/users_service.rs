use corvis::prelude::*;

use crate::common::decorators::Role;

#[derive(Debug, Clone)]
pub struct User {
    pub user_id: u32,
    pub username: String,
    pub password: String,
    pub roles: Vec<Role>,
}

/// In-memory users store (the classic authentication guide example).
#[injectable]
pub struct UsersService {
    #[init(vec![
        User { user_id: 1, username: "john".into(), password: "changeme".into(), roles: vec![Role::Admin] },
        User { user_id: 2, username: "maria".into(), password: "guess".into(), roles: vec![Role::User] },
    ])]
    users: Vec<User>,
}

impl UsersService {
    pub async fn find_one(&self, username: &str) -> Option<User> {
        self.users.iter().find(|u| u.username == username).cloned()
    }
}
