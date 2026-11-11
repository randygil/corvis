use corvis::auth::oauth2::{OAuth2Endpoints, OAuth2Grant, OAuth2Options, OAuth2Validator};
use corvis::prelude::*;

use crate::settings::Settings;
use crate::users::{Account, UsersService};

/// The validator of the generic `provider` strategy (`passport-oauth2`): any OAuth2 provider whose endpoints and client
/// come from the settings. The framework does the authorization code flow with `state` in a signed cookie and PKCE;
/// `validate` runs only for a callback that passed every check.
#[injectable]
pub struct ProviderAuth {
    settings: Arc<Settings>,
    users: Arc<UsersService>,
}

#[async_trait]
impl OAuth2Validator for ProviderAuth {
    const NAME: &'static str = "provider";
    type User = Account;

    fn options(&self) -> OAuth2Options {
        let client = &self.settings.oauth;
        let endpoints = OAuth2Endpoints::new(&client.authorize_url, &client.token_url).userinfo_url(&client.userinfo_url);
        OAuth2Options::new(endpoints, &client.client_id, &client.client_secret, &client.redirect_uri)
            .scope(["openid", "email", "profile"])
            // The state of a login lives in a cookie signed with a secret of its own, not the session's. It is named
            // `__Host-oauth2.provider`; `--dev` has no TLS, so there the cookie is not `Secure` and loses the prefix.
            .state_cookie_secrets([self.settings.oauth_state_secret.clone()])
            .state_cookie_secure(!self.settings.dev)
    }

    async fn validate(&self, _req: &Request, grant: OAuth2Grant) -> Result<Option<Account>, HttpException> {
        let Some(profile) = grant.profile else { return Ok(None) };
        // Identify the user by (provider, id), never by e-mail alone: `provider` is the name of this strategy because
        // its endpoints are not a preset's. Only `verified_email()` may link the user to an account that exists.
        let name = profile.display_name.as_deref().unwrap_or("user");
        Ok(Some(self.users.for_provider_user(&profile.provider, &profile.id, profile.verified_email(), name)))
    }
}
