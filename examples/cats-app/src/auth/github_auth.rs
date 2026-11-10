use std::collections::HashMap;
use std::sync::Mutex;

use corvis::auth::oauth2::{OAuth2Endpoints, OAuth2Grant, OAuth2Options, OAuth2Strategy, OAuth2Validator};
use corvis::prelude::*;

use super::auth_module::AuthModule;
use super::github_controller::GithubController;
use super::local_auth::LoginUser;
use crate::common::decorators::Role;

/// The accounts created by a GitHub login, found again by `(provider, id)`: the GitHub user id, never the login name or
/// the e-mail address, which can change hands. In memory, like the rest of this example.
#[injectable]
pub struct GithubAccounts {
    #[init(Mutex::new(HashMap::new()))]
    by_github_id: Mutex<HashMap<String, LoginUser>>,
}

impl GithubAccounts {
    /// The account of the GitHub user `id`, created on the first login. User ids from 1000 on, so they cannot clash with
    /// the seeded users.
    fn find_or_create(&self, provider: &str, id: &str, login: &str) -> LoginUser {
        let mut accounts = self.by_github_id.lock().unwrap_or_else(|e| e.into_inner());
        let next = 1000 + accounts.len() as u32;
        accounts
            .entry(format!("{provider}:{id}"))
            .or_insert_with(|| LoginUser { user_id: next, username: format!("github:{login}"), roles: vec![Role::User] })
            .clone()
    }
}

/// The validator of the `github` strategy. Everything comes from the environment:
///
/// - `GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET` of the OAuth app; without the id the GitHub login is not mounted.
/// - `GITHUB_REDIRECT_URI`: the absolute URL of the callback route as registered with GitHub (default
///   `http://127.0.0.1:3000/api/v1/auth/github/callback`).
/// - `OAUTH_STATE_SECRET`: signs the cookie that ties a login to the browser that started it, at least 32 bytes
///   (`openssl rand -hex 32`).
/// - `GITHUB_ENDPOINTS_BASE` (for tests): a base URL that replaces GitHub's, with `/authorize`, `/token`, `/user` and
///   `/user/emails` under it.
#[injectable]
pub struct GithubAuth {
    config: Arc<ConfigService>,
    accounts: Arc<GithubAccounts>,
}

#[async_trait]
impl OAuth2Validator for GithubAuth {
    const NAME: &'static str = "github";
    type User = LoginUser;

    fn options(&self) -> OAuth2Options {
        let get = |name: &str| self.config.get::<String>(name).unwrap_or_default();
        let redirect = self.config.get::<String>("GITHUB_REDIRECT_URI").filter(|v| !v.is_empty());
        let redirect = redirect.unwrap_or_else(|| "http://127.0.0.1:3000/api/v1/auth/github/callback".to_string());
        let mut options = OAuth2Options::github(get("GITHUB_CLIENT_ID"), get("GITHUB_CLIENT_SECRET"), redirect)
            .scope(["user:email"])
            // A missing or short secret stops the application with an error that names the option.
            .state_cookie_secrets([get("OAUTH_STATE_SECRET")])
            .return_to_param("return_to");
        if let Some(base) = self.config.get::<String>("GITHUB_ENDPOINTS_BASE").filter(|v| !v.is_empty()) {
            options = options.endpoints(
                OAuth2Endpoints::new(format!("{base}/authorize"), format!("{base}/token"))
                    .userinfo_url(format!("{base}/user"))
                    .emails_url(format!("{base}/user/emails")),
            );
        }
        options
    }

    async fn validate(&self, _req: &Request, grant: OAuth2Grant) -> Result<Option<LoginUser>, HttpException> {
        // Identify by (provider, id), never by e-mail alone. `provider` is "github" only for github.com itself: when the
        // endpoints are replaced (a GitHub Enterprise server) it is `github@<that server's host>`, because this strategy is
        // named like the preset (a strategy with a name of its own would get that name), so a user id of that server
        // cannot become the account of the same number on github.com. A GitHub account without a verified primary
        // address is still a GitHub account; an application that links accounts by e-mail must use `verified_email()`
        // and nothing else.
        let Some(profile) = grant.profile else { return Ok(None) };
        let login = profile.username.as_deref().unwrap_or("user");
        Ok(Some(self.accounts.find_or_create(&profile.provider, &profile.id, login)))
    }
}

/// The module of the GitHub login: the validator, the strategy and the two routes. It is mounted only when
/// `GITHUB_CLIENT_ID` is set, so the example runs without a GitHub app.
#[module]
pub struct GithubLogin;

pub fn github_module() -> DynamicModule {
    let module = DynamicModule::new::<GithubLogin>();
    if std::env::var("GITHUB_CLIENT_ID").map_or(true, |v| v.is_empty()) {
        return module;
    }
    module
        .import(ModuleImport::of::<AuthModule>())
        .providers(vec![
            Provider::class::<GithubAccounts>(),
            Provider::class::<GithubAuth>(),
            Provider::strategy::<OAuth2Strategy<GithubAuth>>(),
        ])
        .controller(ControllerDef::of::<GithubController>())
}
