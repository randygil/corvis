//! Outside `--dev` the cookies are `Secure` and carry the `__Host-` prefix, which makes a browser refuse one planted from
//! a sibling subdomain. The settings come from variables, with a provider's URLs (here the mock's, which is a loopback
//! address and so allowed over plain HTTP).

mod common;

use std::collections::HashMap;

use auth_app::Settings;
use auth_app::mock_provider::MockProvider;
use common::*;
use corvis::prelude::*;

#[corvis::test]
async fn the_cookies_are_host_prefixed_and_secure() {
    let mock = MockProvider::start().await.unwrap();
    let client = mock.client("https://app.example/auth/provider/callback".to_string());
    let vars: HashMap<&str, String> = HashMap::from([
        ("JWT_SECRET", "0123456789abcdef0123456789abcdef".to_string()),
        ("SESSION_SECRET", "1123456789abcdef0123456789abcdef".to_string()),
        ("OAUTH_STATE_SECRET", "2123456789abcdef0123456789abcdef".to_string()),
        ("SERVICE_API_KEY", "3123456789abcdef0123456789abcdef".to_string()),
        ("OAUTH_AUTHORIZE_URL", client.authorize_url),
        ("OAUTH_TOKEN_URL", client.token_url),
        ("OAUTH_USERINFO_URL", client.userinfo_url),
        ("OAUTH_CLIENT_ID", client.client_id),
        ("OAUTH_CLIENT_SECRET", client.client_secret),
        ("OAUTH_REDIRECT_URI", client.redirect_uri),
    ]);
    let settings = Settings::from_lookup(|name| vars.get(name).cloned()).unwrap();
    let h = Harness::with(settings).await;
    let c = h.client();

    let login = c.post("/session/login").json(&json!({ "username": "john", "password": "john-demo-pass" })).send().await;
    assert_eq!(login.status, 200, "{}", login.text());
    let header = set_cookie(&login);
    assert!(header.starts_with("__Host-sid=s%3A"), "{header}");
    assert!(
        header.contains("Secure") && header.contains("HttpOnly") && header.contains("Path=/") && !header.contains("Domain"),
        "{header}"
    );

    let start = c.get("/auth/provider").send().await;
    assert_eq!(start.status, 302);
    let header = set_cookie(&start);
    assert!(header.starts_with("__Host-oauth2.provider="), "{header}");
    assert!(header.contains("Secure") && header.contains("HttpOnly") && header.contains("SameSite=Lax"), "{header}");
}
