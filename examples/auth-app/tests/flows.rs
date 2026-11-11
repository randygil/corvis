//! Every flow of the guide: password login for a token, the token route, the service key, the session, the OAuth2
//! login through the bundled mock provider (no network), and what must never authenticate.

mod common;

use auth_app::mock_provider::MockProfile;
use common::*;
use corvis::JwtService;
use corvis::JwtSignOptions;
use corvis::prelude::*;

// ---- local login and the token route --------------------------------------------------------------------------------

#[corvis::test]
async fn login_for_a_token_and_the_token_route() {
    let h = Harness::start().await;
    let c = h.client();
    let token = token_of(&c, "john", "john-demo-pass").await;

    let me = c.get("/me").bearer(&token).send().await;
    assert_eq!(me.status, 200, "{}", me.text());
    assert_eq!(me.json_value(), json!({ "id": 1, "username": "john", "roles": ["admin"] }));
    assert!(!me.text().contains("argon2"), "a hash never reaches a response");
}

#[corvis::test]
async fn a_wrong_password_and_an_unknown_user_get_the_same_answer() {
    let h = Harness::start().await;
    let c = h.client();
    let wrong = c.post("/auth/login").json(&json!({ "username": "john", "password": "not-the-password" })).send().await;
    let unknown = c.post("/auth/login").json(&json!({ "username": "nobody", "password": "not-the-password" })).send().await;
    let missing = c.post("/auth/login").json(&json!({ "username": "john" })).send().await;
    for r in [&wrong, &unknown, &missing] {
        assert_eq!(r.status, 401);
        assert_eq!(r.text(), UNAUTHORIZED);
        assert!(r.headers.get("set-cookie").is_none());
    }
}

#[corvis::test]
async fn a_legacy_bcrypt_hash_is_replaced_by_argon2id_on_the_first_login() {
    let h = Harness::start().await;
    let c = h.client();
    let before = h.users().password_hash_of("walt").unwrap();
    assert!(before.starts_with("$2b$"), "{before}");

    let first = c.post("/auth/login").json(&json!({ "username": "walt", "password": "walt-demo-pass" })).send().await;
    assert_eq!(first.status, 200, "{}", first.text());
    let after = h.users().password_hash_of("walt").unwrap();
    assert!(after.starts_with("$argon2id$"), "{after}");

    // The next login verifies the new hash, and a wrong password still fails.
    assert_eq!(c.post("/auth/login").json(&json!({ "username": "walt", "password": "walt-demo-pass" })).send().await.status, 200);
    assert_eq!(c.post("/auth/login").json(&json!({ "username": "walt", "password": "wrong" })).send().await.status, 401);
    // A failed login does not touch the stored hash.
    assert_eq!(h.users().password_hash_of("walt").unwrap(), after);
}

#[corvis::test]
async fn a_missing_or_expired_token_is_the_plain_401() {
    let h = Harness::start().await;
    let c = h.client();
    let none = c.get("/me").send().await;
    assert_eq!((none.status, none.text().as_str()), (401, UNAUTHORIZED));

    // Signed with the application's own key, but already expired.
    let jwt = h.app.get::<JwtService>().unwrap();
    let claims = json!({ "sub": 1, "username": "john", "roles": ["admin"] });
    let expired = jwt.sign_with(&claims, JwtSignOptions { expires_in: Some("-10s".into()), ..Default::default() }).unwrap();
    let r = c.get("/me").bearer(&expired).send().await;
    assert_eq!((r.status, r.text().as_str()), (401, UNAUTHORIZED));

    // A token signed with another secret.
    let other = JwtService::new(corvis::JwtModuleOptions::new("another-secret-another-secret-0123456789").expires_in("1h"));
    let forged = other.sign(&claims).unwrap();
    assert_eq!(c.get("/me").bearer(&forged).send().await.status, 401);

    // A valid token in the wrong place: the query string is not a place the strategy reads.
    let token = token_of(&c, "john", "john-demo-pass").await;
    assert_eq!(c.get(&format!("/me?access_token={token}")).send().await.status, 401);
}

// ---- the service key --------------------------------------------------------------------------------------------------

#[corvis::test]
async fn the_service_key_and_a_user_token_both_reach_the_reports() {
    let h = Harness::start().await;
    let c = h.client();

    let service = c.get("/reports").header("x-api-key", &h.settings.service_api_key).send().await;
    assert_eq!(service.status, 200, "{}", service.text());
    assert_eq!(service.json_value(), json!({ "caller": { "service": "reports" } }));
    assert!(!service.text().contains(&h.settings.service_api_key));

    let token = token_of(&c, "maria", "maria-demo-pass").await;
    let user = c.get("/reports").bearer(&token).send().await;
    assert_eq!(user.status, 200, "{}", user.text());
    assert_eq!(user.json_value()["caller"]["username"], "maria");
}

#[corvis::test]
async fn a_wrong_or_missing_service_key_is_the_plain_401() {
    let h = Harness::start().await;
    let c = h.client();
    let wrong = c.get("/reports").header("x-api-key", "not-the-service-key-not-the-service-key").send().await;
    assert_eq!((wrong.status, wrong.text().as_str()), (401, UNAUTHORIZED));
    let empty = c.get("/reports").header("x-api-key", "").send().await;
    assert_eq!(empty.status, 401);
    let none = c.get("/reports").send().await;
    assert_eq!(none.status, 401);
    // The key in the query string is not a place the strategy reads.
    let query = c.get(&format!("/reports?api_key={}", h.settings.service_api_key)).send().await;
    assert_eq!(query.status, 401);
}

// ---- sessions ---------------------------------------------------------------------------------------------------------

#[corvis::test]
async fn a_session_login_opens_the_protected_route_and_a_logout_closes_it() {
    let h = Harness::start().await;
    let c = h.client();
    let anonymous = c.get("/session/me").send().await;
    assert_eq!(anonymous.status, 401);
    assert!(anonymous.headers.get("set-cookie").is_none());

    let wrong = c.post("/session/login").json(&json!({ "username": "john", "password": "wrong" })).send().await;
    assert_eq!(wrong.status, 401);
    assert!(wrong.headers.get("set-cookie").is_none(), "a failed login sets no cookie");

    let login = c.post("/session/login").json(&json!({ "username": "john", "password": "john-demo-pass" })).send().await;
    assert_eq!(login.status, 200, "{}", login.text());
    let header = set_cookie(&login);
    assert!(header.starts_with("sid=s%3A") && header.contains("HttpOnly") && header.contains("SameSite=Lax"), "{header}");
    let cookie = cookie_of(&login);

    let me = c.get("/session/me").header("cookie", &cookie).send().await;
    assert_eq!(me.status, 200, "{}", me.text());
    assert_eq!(me.json_value()["username"], "john");
    // The cookie does not open the token routes.
    assert_eq!(c.get("/me").header("cookie", &cookie).send().await.status, 401);

    let out = c.post("/session/logout").header("cookie", &cookie).send().await;
    assert_eq!(out.status, 204, "{}", out.text());
    assert_eq!(c.get("/session/me").header("cookie", &cookie).send().await.status, 401);
    assert_eq!(c.get("/session/me").header("cookie", &cookie_of(&out)).send().await.status, 401);
}

#[corvis::test]
async fn the_session_identifier_changes_at_login() {
    let h = Harness::start().await;
    let c = h.client();
    let first = session_of(&c, "john", "john-demo-pass").await;
    // Logging in again with the cookie in hand (what a fixation attack plants) gives another identifier, and the old
    // one is anonymous.
    let again =
        c.post("/session/login").header("cookie", &first).json(&json!({ "username": "maria", "password": "maria-demo-pass" })).send().await;
    assert_eq!(again.status, 200);
    let second = cookie_of(&again);
    assert_ne!(second, first);
    assert_eq!(c.get("/session/me").header("cookie", &first).send().await.status, 401);
    assert_eq!(c.get("/session/me").header("cookie", &second).send().await.json_value()["username"], "maria");
}

#[corvis::test]
async fn a_cookie_sent_twice_is_no_credential() {
    let h = Harness::start().await;
    let c = h.client();
    let cookie = session_of(&c, "john", "john-demo-pass").await;
    // A cookie planted from a sibling subdomain next to the user's own: neither is trusted.
    let twice = format!("{cookie}; sid=s%3Aplanted.signature");
    assert_eq!(c.get("/session/me").header("cookie", &twice).send().await.status, 401);
}

// ---- tokens and cookies of a previous run ------------------------------------------------------------------------------

#[corvis::test]
async fn a_restart_refuses_the_credentials_of_the_previous_run() {
    let before = Harness::start().await;
    let token = token_of(&before.client(), "john", "john-demo-pass").await;
    let cookie = session_of(&before.client(), "john", "john-demo-pass").await;
    // The same application, started again: `--dev` generates new secrets, and the session store is in memory.
    let after = Harness::start().await;
    let c = after.client();
    assert_eq!(c.get("/me").bearer(&token).send().await.status, 401);
    assert_eq!(c.get("/session/me").header("cookie", &cookie).send().await.status, 401);
    assert_ne!(before.settings.jwt_secret, after.settings.jwt_secret);
}

// ---- OAuth2 through the mock provider ---------------------------------------------------------------------------------

/// Starts a login and returns the state cookie and the provider's authorization URL.
async fn start_provider_login(h: &Harness, c: &corvis::testing::TestClient) -> (String, String) {
    let start = c.get("/auth/provider").send().await;
    assert_eq!(start.status, 302, "{}", start.text());
    let location = start.header("location").unwrap().to_string();
    assert!(location.starts_with(&format!("http://{}/authorize?", h.settings.mock().unwrap().addr())), "{location}");
    assert!(location.contains("code_challenge_method=S256") && location.contains("state="), "{location}");
    assert!(location.contains("code_challenge="), "{location}");
    (cookie_of(&start), location)
}

#[corvis::test]
async fn the_provider_login_ends_with_a_session() {
    let h = Harness::start().await;
    let c = h.client();
    let (state_cookie, location) = start_provider_login(&h, &c).await;
    assert!(state_cookie.starts_with("oauth2.provider="), "{state_cookie}");
    assert_eq!(h.settings.mock().unwrap().token_requests(), 0, "starting a login contacts nobody");

    let callback = approve_at_provider(&h.settings, &location).await;
    let done = c.get(&callback).header("cookie", &state_cookie).send().await;
    assert_eq!(done.status, 200, "{}", done.text());
    assert_eq!(done.header("cache-control"), Some("no-store"));
    assert_eq!(done.json_value()["username"], "provider:Mock Person");
    assert_eq!(h.settings.mock().unwrap().token_requests(), 1);
    assert_eq!(h.settings.mock().unwrap().userinfo_requests(), 1);
    assert!(!done.text().contains("mock-"), "the provider's token never reaches the response");

    // The callback logged in to the session: its cookie is the one the response set last.
    let session = done
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .find(|c| c.starts_with("sid="))
        .expect("a session cookie");
    let me = c.get("/session/me").header("cookie", &session).send().await;
    assert_eq!(me.status, 200, "{}", me.text());
    assert_eq!(me.json_value()["username"], "provider:Mock Person");
    assert_eq!(me.json_value()["roles"], json!(["user"]));
}

#[corvis::test]
async fn a_callback_with_a_state_nobody_issued_is_refused_before_the_provider_is_asked() {
    let h = Harness::start().await;
    let c = h.client();
    let (state_cookie, _) = start_provider_login(&h, &c).await;

    let forged = c.get("/auth/provider/callback?code=SECRET-CODE&state=forged").header("cookie", &state_cookie).send().await;
    assert_eq!(forged.status, 401);
    assert!(!forged.text().contains("SECRET-CODE") && !forged.text().contains("forged"), "{}", forged.text());
    // No state cookie at all: a callback that was not started by this browser.
    let no_cookie = c.get("/auth/provider/callback?code=SECRET-CODE&state=forged").send().await;
    assert_eq!(no_cookie.status, 401);
    // The provider's own refusal.
    let denied = c.get("/auth/provider/callback?error=access_denied&state=x").header("cookie", &state_cookie).send().await;
    assert_eq!(denied.status, 401);

    assert_eq!(h.settings.mock().unwrap().token_requests(), 0, "no token request was sent");
}

#[corvis::test]
async fn a_state_cannot_be_redeemed_twice() {
    let h = Harness::start().await;
    let c = h.client();
    let (state_cookie, location) = start_provider_login(&h, &c).await;
    let callback = approve_at_provider(&h.settings, &location).await;
    assert_eq!(c.get(&callback).header("cookie", &state_cookie).send().await.status, 200);
    let replay = c.get(&callback).header("cookie", &state_cookie).send().await;
    assert_eq!(replay.status, 401);
    assert_eq!(h.settings.mock().unwrap().token_requests(), 1, "the replay never reached the provider");
}

async fn provider_login(h: &Harness, c: &corvis::testing::TestClient) -> corvis::testing::TestResponse {
    let (state_cookie, location) = start_provider_login(h, c).await;
    let callback = approve_at_provider(&h.settings, &location).await;
    c.get(&callback).header("cookie", &state_cookie).send().await
}

#[corvis::test]
async fn a_verified_email_links_the_provider_user_to_the_account_that_has_it() {
    let h = Harness::start().await;
    let c = h.client();
    h.settings.mock().unwrap().set_profile(MockProfile {
        sub: "mock-7".into(),
        name: "Somebody".into(),
        email: "maria@example.test".into(),
        email_verified: true,
    });
    let done = provider_login(&h, &c).await;
    assert_eq!(done.status, 200, "{}", done.text());
    // The provider vouched for the address, so this is maria's account, with maria's roles.
    assert_eq!(done.json_value()["username"], "maria");

    // The link is stored under (provider, id): the next login does not depend on the address any more.
    h.settings.mock().unwrap().set_profile(MockProfile {
        sub: "mock-7".into(),
        name: "Somebody".into(),
        email: "changed@example.test".into(),
        email_verified: true,
    });
    assert_eq!(provider_login(&h, &c).await.json_value()["username"], "maria");
}

#[corvis::test]
async fn an_unverified_email_never_links_to_an_account() {
    let h = Harness::start().await;
    let c = h.client();
    // The same address as the admin, but the provider did not verify it: a new account, not john's.
    h.settings.mock().unwrap().set_profile(MockProfile {
        sub: "mock-8".into(),
        name: "Impostor".into(),
        email: "john@example.test".into(),
        email_verified: false,
    });
    let done = provider_login(&h, &c).await;
    assert_eq!(done.status, 200, "{}", done.text());
    assert_eq!(done.json_value()["username"], "provider:Impostor");
    let session = done
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
        .find(|c| c.starts_with("sid="))
        .unwrap();
    let me = c.get("/session/me").header("cookie", &session).send().await.json_value();
    assert_eq!(me["roles"], json!(["user"]));
    assert_ne!(me["id"], json!(1));
    // And it cannot reach the admin route.
    assert_eq!(c.get("/admin/users").header("cookie", &session).send().await.status, 403);
}

#[corvis::test]
async fn a_provider_answer_without_an_id_is_never_a_login() {
    // The profile has no `sub`: the strategy fails the request (the provider misbehaved), and no session is created.
    let h = Harness::start().await;
    let c = h.client();
    h.settings.mock().unwrap().set_profile(MockProfile { sub: String::new(), ..MockProfile::default() });
    let done = provider_login(&h, &c).await;
    assert_eq!(done.status, 500, "{}", done.text());
    assert!(done.headers.get_all("set-cookie").iter().all(|v| !v.to_str().unwrap().starts_with("sid=")));
}
