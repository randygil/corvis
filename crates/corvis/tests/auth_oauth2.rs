//! The scenarios of `auth/oauth2` against a mock provider on 127.0.0.1: the flow, `state` and PKCE, the callback
//! checks, the failure mapping, `return_to`, the limits of the provider client, TLS, profiles and refresh. The cases
//! recorded from the original packages are replayed by `auth_oauth2_original.rs`. Time moves with tokio's paused
//! clock where a test needs it, so no test sleeps to make a state expire.
#![cfg(feature = "auth-oauth2")]

mod oauth2_common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use base64::Engine as _;
use corvis::auth::oauth2::{OAuth2Client, OAuth2Endpoints, OAuth2Error, OAuth2HttpOptions, OAuth2Options};
use corvis::auth::session::{SameSite, SessionCookie, SessionOptions};
use corvis::prelude::*;
use corvis::testing::{TestResponse, request};
use oauth2_common::*;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

const SESSION_SECRET: &str = "oauth2-test-session-secret-0123456789abcdef";

fn session_options() -> SessionOptions {
    SessionOptions::with_secrets([SESSION_SECRET]).unwrap().cookie(SessionCookie::default().secure(false))
}

fn challenge_of(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Logs in and completes the callback with the code `code`.
async fn flow(app: &corvis::CorvisApplication, base: &str, code: &str) -> (Login, TestResponse) {
    let l = login(app, base).await;
    let r = callback(app, &format!("{base}/callback"), &l, code).await;
    (l, r)
}

// ---- registration and startup ----

#[tokio::test]
async fn the_strategy_is_named_by_the_validator_and_builds_its_options_once() {
    let mock = Mock::start(standard).await;
    let (app, cfg) = google_app(&mock).await;
    assert_eq!(cfg.options_calls.load(Ordering::SeqCst), 1);
    assert_eq!(get(&app, "/auth/google", None).await.status, 302);
    assert_eq!(cfg.options_calls.load(Ordering::SeqCst), 1);
}

async fn startup_error(options: OAuth2Options) -> String {
    let cfg = Cfg::new(options);
    match AppBuilder::default().google(&cfg).try_build().await {
        Ok(_) => panic!("the application started"),
        Err(e) => e,
    }
}

#[tokio::test]
async fn invalid_options_stop_the_application_naming_the_strategy_and_the_option() {
    let mock = Mock::start(standard).await;
    let base = || google_options(&mock);
    let cases: Vec<(&str, OAuth2Options, &str)> = vec![
        ("no storage", OAuth2Options::google("id", CLIENT_SECRET, "https://app.example/cb").scope(["email"]), "state_cookie(keys)"),
        (
            "relative redirect",
            OAuth2Options::google("id", CLIENT_SECRET, "/auth/google/callback").scope(["email"]).state_in_session(),
            "redirect_uri",
        ),
        (
            "redirect with fragment",
            OAuth2Options::google("id", CLIENT_SECRET, "https://app.example/cb#x").scope(["email"]).state_in_session(),
            "redirect_uri",
        ),
        (
            "redirect with user info",
            OAuth2Options::google("id", CLIENT_SECRET, "https://u:p@app.example/cb").scope(["email"]).state_in_session(),
            "redirect_uri",
        ),
        (
            "redirect over plain http",
            OAuth2Options::google("id", CLIENT_SECRET, "http://app.example/cb").scope(["email"]).state_in_session(),
            "redirect_uri",
        ),
        (
            "plain http token url",
            base().endpoints(OAuth2Endpoints::new("https://p.example/a", "http://provider.example/token")),
            "token_url",
        ),
        (
            "empty client id",
            OAuth2Options::google("", CLIENT_SECRET, "https://app.example/cb").scope(["email"]).state_in_session(),
            "client_id",
        ),
        (
            "empty client secret",
            OAuth2Options::google("id", "", "https://app.example/cb").scope(["email"]).state_in_session(),
            "client_secret",
        ),
        ("scope with a space", base().scope(["a b"]), "scope"),
        ("google without scope", base().scope(Vec::<String>::new()), "scope"),
        ("reserved parameter", base().authorization_param("state", "fixed"), "\"state\""),
        ("total timeout of ten minutes", base().http(OAuth2HttpOptions::new().timeout(Duration::from_secs(600))), "timeout"),
        ("state ttl", base().state_ttl(Duration::from_secs(5)), "state_ttl"),
        ("unsafe failure page", base().failure_redirect("//evil.example"), "failure_redirect"),
        ("prefixed name without Secure", base().state_cookie_secure(false).state_cookie_name("__Host-x"), "state_cookie_name"),
    ];
    for (what, options, wanted) in cases {
        let error = startup_error(options).await;
        assert!(error.contains("google"), "{what}: the error names the strategy: {error}");
        assert!(error.contains(wanted), "{what}: expected {wanted:?} in {error}");
        assert!(!error.contains(CLIENT_SECRET), "{what}: {error}");
        assert!(!error.contains(STATE_SECRET), "{what}: {error}");
    }
}

#[tokio::test]
async fn the_loopback_redirect_uri_starts() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(google_options(&mock));
    let options = OAuth2Options::google("id", CLIENT_SECRET, "http://127.0.0.1:3000/auth/google/callback")
        .endpoints(endpoints(&mock))
        .scope(["email"])
        .state_cookie(keys());
    let cfg2 = Cfg::new(options);
    drop(cfg);
    AppBuilder::default().google(&cfg2).build().await;
}

#[tokio::test]
async fn session_storage_needs_sessions_that_the_callback_can_carry() {
    let mock = Mock::start(standard).await;
    let options = || google_options(&mock);
    let session = || {
        OAuth2Options::google("id", CLIENT_SECRET, "https://app.example/cb").endpoints(endpoints(&mock)).scope(["email"]).state_in_session()
    };
    let _ = options;
    let cfg = Cfg::new(session());
    let error = AppBuilder::default().google(&cfg).try_build().await.err().expect("no SessionModule");
    assert!(error.contains("google") && error.contains("SessionModule"), "{error}");
    let cfg = Cfg::new(session());
    let strict = SessionOptions::with_secrets([SESSION_SECRET]).unwrap().cookie(SessionCookie::default().same_site(SameSite::Strict));
    let error = AppBuilder::default().google(&cfg).session(strict).try_build().await.err().expect("strict cookie");
    assert!(error.contains("google") && error.contains("SameSite=Strict") && error.contains("callback"), "{error}");
    let cfg = Cfg::new(session());
    AppBuilder::default().google(&cfg).session(session_options()).build().await;
}

// ---- the login start ----

#[tokio::test]
async fn the_login_redirects_with_the_parameters_of_the_original() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    assert_eq!(l.names(), ["response_type", "redirect_uri", "scope", "code_challenge", "code_challenge_method", "state", "client_id"]);
    assert_eq!(l.param("response_type"), Some("code"));
    assert_eq!(l.param("redirect_uri"), Some("https://app.example/auth/google/callback"));
    assert_eq!(l.param("scope"), Some("email profile"));
    assert_eq!(l.param("code_challenge_method"), Some("S256"));
    assert_eq!(l.param("client_id"), Some("google-client-id"));
    assert_eq!(l.location.path(), "/authorize");
    assert_eq!(l.response.header("cache-control"), Some("no-store"));
    assert_eq!(l.response.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(l.response.header("content-length"), Some("0"));
}

#[tokio::test]
async fn two_logins_have_different_random_values() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let (a, b) = (login(&app, "/auth/google").await, login(&app, "/auth/google").await);
    assert_ne!(a.state(), b.state());
    assert_ne!(a.param("code_challenge"), b.param("code_challenge"));
    for l in [&a, &b] {
        for v in [l.state(), l.param("code_challenge").unwrap().to_string()] {
            assert_eq!(v.len(), 43, "{v}");
            assert!(v.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        }
    }
}

#[tokio::test]
async fn the_state_cookie_has_the_documented_attributes() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    let cookies = set_cookies(&l.response);
    assert_eq!(cookies.len(), 1, "{cookies:?}");
    let c = &cookies[0];
    assert!(c.starts_with("__Host-oauth2.google=s%3A"), "{c}");
    for attr in ["Path=/", "HttpOnly", "Secure", "SameSite=Lax", "Max-Age=600"] {
        assert!(c.contains(attr), "{attr} in {c}");
    }
    assert!(!c.contains("Domain"), "{c}");
    let value =
        percent_encoding::percent_decode_str(c.split(';').next().unwrap().split_once('=').unwrap().1).decode_utf8().unwrap().into_owned();
    let record = keys().unsign(&value).expect("the value verifies with the configured keys");
    let json: Value = serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(record).unwrap()).unwrap();
    assert_eq!(json["n"], "google");
    assert_eq!(json["f"][0]["s"], l.state());
    assert!(json["f"][0]["t"].is_i64());
    // The verifier is not in the cookie: it is derived on the server from the state and the key.
    assert!(json["f"][0].get("v").is_none(), "{json}");
}

#[tokio::test]
async fn a_plain_http_development_cookie_is_unprefixed_and_not_secure() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(google_options(&mock).state_cookie_secure(false));
    let app = AppBuilder::default().google(&cfg).build().await;
    let l = login(&app, "/auth/google").await;
    let c = &set_cookies(&l.response)[0];
    assert!(c.starts_with("oauth2.google="), "{c}");
    assert!(!c.contains("Secure"), "{c}");
}

#[tokio::test]
async fn the_host_header_is_ignored() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let r = request(&app).get("/auth/google").header("host", "evil.example").header("x-forwarded-host", "evil.example").send().await;
    let l = parse_login(r);
    assert_eq!(l.param("redirect_uri"), Some("https://app.example/auth/google/callback"));
}

#[tokio::test]
async fn scope_extra_parameters_and_existing_url_parameters() {
    let mock = Mock::start(standard).await;
    // no scope: no parameter
    let cfg = Cfg::new(generic_options(&mock));
    let app = AppBuilder::default().generic(&cfg).build().await;
    let l = login(&app, "/auth/corp").await;
    assert!(l.param("scope").is_none(), "{:?}", l.names());
    // parameters in the configured URL stay, and the extras come before `response_type`
    let options = generic_options(&mock)
        .endpoints(OAuth2Endpoints::new(mock.url("/authorize?tenant=a"), mock.url("/token")))
        .authorization_param("access_type", "offline")
        .scope(["a", "b"]);
    let cfg = Cfg::new(options);
    let app = AppBuilder::default().generic(&cfg).build().await;
    let l = login(&app, "/auth/corp").await;
    assert_eq!(
        l.names(),
        [
            "tenant",
            "access_type",
            "response_type",
            "redirect_uri",
            "scope",
            "code_challenge",
            "code_challenge_method",
            "state",
            "client_id"
        ]
    );
    assert_eq!(l.param("tenant"), Some("a"));
    assert_eq!(l.param("access_type"), Some("offline"));
    assert_eq!(l.param("scope"), Some("a b"));
}

#[tokio::test]
async fn github_joins_its_scope_with_commas_and_a_guard_can_replace_it() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(github_options(&mock).scope(["user:email", "read:user"]));
    let app = AppBuilder::default().github(&cfg).build().await;
    assert_eq!(login(&app, "/auth/github").await.param("scope"), Some("user:email,read:user"));
    assert_eq!(login(&app, "/auth/github/scoped").await.param("scope"), Some("read:org"));
    // the guard's scope is validated like the configured one
    let (google, _) = google_app(&mock).await;
    let r = get(&google, "/auth/google/bad-scope", None).await;
    assert_eq!(r.status, 500);
    assert_eq!(r.json_value(), internal_error());
    assert!(set_cookies(&r).is_empty(), "nothing was stored");
}

#[tokio::test]
async fn a_login_started_outside_the_guard_phase_is_a_500_and_stores_nothing() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let r = get(&app, "/auth/google/interceptor", None).await;
    assert_eq!(r.status, 500);
    assert_eq!(r.json_value(), internal_error());
    assert!(set_cookies(&r).is_empty());
}

#[tokio::test]
async fn without_pkce_the_redirect_has_no_challenge_and_the_state_is_still_checked() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(google_options(&mock).pkce(false));
    let app = AppBuilder::default().google(&cfg).build().await;
    let l = login(&app, "/auth/google").await;
    assert!(l.param("code_challenge").is_none() && l.param("code_challenge_method").is_none(), "{:?}", l.names());
    assert!(l.param("state").is_some());
    assert_eq!(get(&app, "/auth/google/callback?code=c&state=forged", l.cookie.as_deref()).await.status, 401);
    assert_eq!(mock.count(), 0);
    assert_eq!(callback(&app, "/auth/google/callback", &l, "c").await.status, 200);
    assert!(mock.hits_to("/token")[0].field("code_verifier").is_none());
}

// ---- the callback: success ----

#[tokio::test]
async fn the_callback_exchanges_the_code_and_the_validator_receives_the_grant() {
    let mock = Mock::start(standard).await;
    let (app, cfg) = google_app(&mock).await;
    let (l, r) = flow(&app, "/auth/google", "the-code").await;
    assert_eq!(r.status, 200, "{}", r.text());
    // The strategy is named `google` but talks to the mock, not to Google's own endpoints: its ids are not Google's, so
    // the provider carries the issuer.
    assert_eq!(r.json_value()["user"], json!({ "id": format!("google@{}:110169484474386276334", mock.addr) }));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert_eq!(r.header("referrer-policy"), Some("no-referrer"));

    let token = mock.hits_to("/token");
    assert_eq!(token.len(), 1);
    let t = &token[0];
    assert_eq!(t.method, "POST");
    assert_eq!(t.header("content-type"), Some("application/x-www-form-urlencoded"));
    assert!(t.header("authorization").is_none());
    let names: Vec<String> = t.form().into_iter().map(|(k, _)| k).collect();
    assert_eq!(names, ["grant_type", "redirect_uri", "code_verifier", "client_id", "client_secret", "code"]);
    assert_eq!(t.field("grant_type").as_deref(), Some("authorization_code"));
    assert_eq!(t.field("redirect_uri").as_deref(), Some("https://app.example/auth/google/callback"));
    assert_eq!(t.field("client_id").as_deref(), Some("google-client-id"));
    assert_eq!(t.field("client_secret").as_deref(), Some(CLIENT_SECRET));
    assert_eq!(t.field("code").as_deref(), Some("the-code"));
    // the verifier matches the challenge of the redirect of the same login
    assert_eq!(challenge_of(&t.field("code_verifier").unwrap()), l.param("code_challenge").unwrap());

    let profile = mock.hits_to("/userinfo");
    assert_eq!(profile.len(), 1);
    assert_eq!(profile[0].header("authorization"), Some("Bearer ACCESS-TOKEN-FAKE"));
    assert!(!profile[0].target.contains("ACCESS-TOKEN"), "the token is not in the URL");

    let seen = cfg.seen();
    assert_eq!(seen.len(), 1);
    let s = &seen[0];
    assert_eq!((s.access.as_str(), s.refresh.as_deref()), ("ACCESS-TOKEN-FAKE", Some("REFRESH-TOKEN-FAKE")));
    assert_eq!((s.expires_in, s.scope.as_deref(), s.id_token.as_deref()), (Some(3600), Some("email profile"), Some("ID-TOKEN-FAKE")));
    let p = s.profile.as_ref().unwrap();
    assert_eq!(p["provider"], format!("google@{}", mock.addr));
    assert_eq!(p["id"], "110169484474386276334");
    assert_eq!(p["displayName"], "Ana Perez");
    assert_eq!(p["name"], json!({ "familyName": "Perez", "givenName": "Ana" }));
    assert_eq!(p["emails"], json!([{ "value": "ana@example.com", "verified": true }]));
    assert_eq!(p["photos"], json!([{ "value": "https://lh3.example/a.png" }]));
    assert_eq!(s.verified_email.as_deref(), Some("ana@example.com"));
    for secret in ["ACCESS-TOKEN", "REFRESH-TOKEN", "ID-TOKEN", "ana@example.com", "Ana Perez"] {
        assert!(!s.debug.contains(secret), "Debug of the grant holds {secret}: {}", s.debug);
    }
}

#[tokio::test]
async fn a_provider_without_a_refresh_token_gives_none() {
    let mock = Mock::start(|r| match r.path() {
        "/token" => Reply::json(200, r#"{"access_token":"t","token_type":"bearer"}"#),
        _ => standard(r),
    })
    .await;
    let (app, cfg) = google_app(&mock).await;
    assert_eq!(flow(&app, "/auth/google", "c").await.1.status, 200);
    let s = &cfg.seen()[0];
    assert!(s.refresh.is_none() && s.expires_in.is_none() && s.id_token.is_none());
}

#[tokio::test]
async fn a_generic_strategy_without_a_userinfo_url_loads_no_profile() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(generic_options(&mock).endpoints(OAuth2Endpoints::new(mock.url("/authorize"), mock.url("/token"))));
    let app = AppBuilder::default().generic(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/corp", "c").await;
    assert_eq!(r.status, 200);
    assert!(cfg.seen()[0].profile.is_none());
    assert!(mock.hits_to("/userinfo").is_empty() && mock.hits_to("/user").is_empty());
}

#[tokio::test]
async fn a_generic_userinfo_uses_the_google_mapping_with_the_strategy_name() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(generic_options(&mock));
    let app = AppBuilder::default().generic(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/corp", "c").await;
    assert_eq!(r.status, 200);
    let p = cfg.seen()[0].profile.clone().unwrap();
    assert_eq!(p["provider"], "corp");
    assert_eq!(p["emails"], json!([{ "value": "ana@example.com", "verified": true }]));
}

#[tokio::test]
async fn what_the_validator_decides() {
    let mock = Mock::start(standard).await;
    let (app, cfg) = google_app(&mock).await;
    cfg.set_verdict(Verdict::Refuse);
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    cfg.set_verdict(Verdict::Forbidden);
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!(r.status, 403);
    cfg.set_verdict(Verdict::Empty);
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()), "a user that serializes to null never authenticates");
}

// ---- the callback: checks before the provider ----

#[tokio::test]
async fn a_callback_that_cannot_be_trusted_never_reaches_the_provider() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    let cookie = l.cookie.clone().unwrap();
    let state = l.state();
    let value = cookie.split_once('=').unwrap().1.to_string();
    let mut tampered = value.clone().into_bytes();
    let n = tampered.len() - 1;
    tampered[n] = if tampered[n] == b'A' { b'B' } else { b'A' };
    let tampered = format!("__Host-oauth2.google={}", String::from_utf8(tampered).unwrap());
    let long_code = "a".repeat(2049);
    let cases: Vec<(&str, String, Option<&str>)> = vec![
        ("forged state", "?code=abc&state=forged".into(), Some(&cookie)),
        ("state of the wrong shape", format!("?code=abc&state={state}x"), Some(&cookie)),
        ("no state at all", "?code=abc".into(), Some(&cookie)),
        ("no cookie", format!("?code=abc&state={state}"), None),
        ("tampered cookie", format!("?code=abc&state={state}"), Some(&tampered)),
        ("repeated code", format!("?code=a&code=b&state={state}"), Some(&cookie)),
        ("repeated state", format!("?code=a&state={state}&state={state}"), Some(&cookie)),
        ("code and error", format!("?code=a&error=access_denied&state={state}"), Some(&cookie)),
        ("oversized code", format!("?code={long_code}&state={state}"), Some(&cookie)),
        ("empty code", format!("?code=&state={state}"), Some(&cookie)),
    ];
    for (what, query, cookie) in cases {
        let r = get(&app, &format!("/auth/google/callback{query}"), cookie).await;
        assert_eq!((r.status, r.json_value()), (401, unauthorized()), "{what}");
        assert_eq!(mock.count(), 0, "{what}: the provider received a request");
    }
    // none of that spent the real login: it still completes
    let r = get(&app, &format!("/auth/google/callback?code=abc&state={state}"), Some(&cookie)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(mock.hits_to("/token").len(), 1);
}

#[tokio::test]
async fn another_method_is_refused() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    let r = request(&app)
        .post(&format!("/auth/google/callback?code=a&state={}", l.state()))
        .header("cookie", l.cookie.as_deref().unwrap())
        .form(&[("code", "a"), ("state", &l.state())])
        .send()
        .await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    assert_eq!(mock.count(), 0);
}

#[tokio::test]
async fn a_state_record_of_another_strategy_is_refused() {
    let mock = Mock::start(standard).await;
    let google = Cfg::new(google_options(&mock));
    let github = Cfg::new(github_options(&mock));
    let app = AppBuilder::default().google(&google).github(&github).build().await;
    let l = login(&app, "/auth/google").await;
    let value = l.cookie.as_deref().unwrap().split_once('=').unwrap().1;
    let r = get(&app, &format!("/auth/github/callback?code=a&state={}", l.state()), Some(&format!("__Host-oauth2.github={value}"))).await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    assert_eq!(mock.count(), 0);
}

#[tokio::test(start_paused = true)]
async fn an_expired_state_is_refused_and_the_boundary_is_the_lifetime() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let late = login(&app, "/auth/google").await;
    let edge = login(&app, "/auth/google").await;
    tokio::time::advance(Duration::from_secs(600)).await;
    let r = callback(&app, "/auth/google/callback", &edge, "c").await;
    assert_eq!(r.status, 200, "{}", r.text());
    tokio::time::advance(Duration::from_secs(1)).await;
    let r = callback(&app, "/auth/google/callback", &late, "c").await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    assert_eq!(mock.hits_to("/token").len(), 1, "only the login inside its lifetime reached the provider");
}

#[tokio::test]
async fn the_state_cookie_is_deleted_on_success_failure_and_error() {
    let mock = Mock::start(|r| match r.path() {
        "/token" if r.field("code").as_deref() == Some("down") => Reply::json(500, "{}"),
        _ => standard(r),
    })
    .await;
    let (app, _) = google_app(&mock).await;
    for (query, status) in [("code=ok", 200), ("error=access_denied", 401), ("code=down", 500)] {
        let l = login(&app, "/auth/google").await;
        let r = get(&app, &format!("/auth/google/callback?{query}&state={}", l.state()), l.cookie.as_deref()).await;
        assert_eq!(r.status, status);
        let cleared = set_cookies(&r).into_iter().find(|c| c.starts_with("__Host-oauth2.google=;"));
        let cleared = cleared.unwrap_or_else(|| panic!("no deleting Set-Cookie for status {status}: {:?}", set_cookies(&r)));
        assert!(cleared.contains("Expires=Thu, 01 Jan 1970"), "{cleared}");
    }
}

#[tokio::test]
async fn a_replay_with_the_cookie_is_refused_by_this_instance_without_the_provider() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    assert_eq!(callback(&app, "/auth/google/callback", &l, "c").await.status, 200);
    // the browser ignored the clearing cookie and presents the same record again: the state was already redeemed
    let again = callback(&app, "/auth/google/callback", &l, "c").await;
    assert_eq!((again.status, again.json_value()), (401, unauthorized()));
    assert_eq!(mock.hits_to("/token").len(), 1, "the replay cost no token request");
}

// ---- concurrent logins ----

#[tokio::test]
async fn two_logins_of_one_browser_do_not_break_each_other() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let first = login(&app, "/auth/google").await;
    // the second tab sends the cookie the first one received
    let second = login_with_cookie(&app, "/auth/google", first.cookie.as_deref()).await;
    assert_ne!(first.state(), second.state());
    // the second callback arrives first, with the cookie holding both flows
    let r = callback(&app, "/auth/google/callback", &second, "c2").await;
    assert_eq!(r.status, 200, "{}", r.text());
    let remaining = cookie_after(&r, "__Host-oauth2.google").unwrap();
    assert!(!remaining.ends_with('='), "the cookie keeps the other flow");
    let r = get(&app, &format!("/auth/google/callback?code=c1&state={}", first.state()), Some(&remaining)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    // a flow is used once: the second callback of the same state fails
    let r = get(
        &app,
        &format!("/auth/google/callback?code=c1&state={}", first.state()),
        Some(&cookie_after(&r, "__Host-oauth2.google").unwrap()),
    )
    .await;
    assert_eq!(r.status, 401);
    assert_eq!(mock.hits_to("/token").len(), 2);
}

#[tokio::test]
async fn a_forged_callback_does_not_spend_the_flows_of_the_browser() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    let r = get(&app, "/auth/google/callback?code=x&state=forged", l.cookie.as_deref()).await;
    assert_eq!(r.status, 401);
    let kept = cookie_after(&r, "__Host-oauth2.google").unwrap();
    assert!(!kept.ends_with('='), "the real flow is kept");
    let r = get(&app, &format!("/auth/google/callback?code=c&state={}", l.state()), Some(&kept)).await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn at_most_four_flows_are_kept_and_the_oldest_goes_first() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let mut logins = vec![login(&app, "/auth/google").await];
    for _ in 0..4 {
        let cookie = logins.last().unwrap().cookie.clone();
        logins.push(login_with_cookie(&app, "/auth/google", cookie.as_deref()).await);
    }
    let last_cookie = logins.last().unwrap().cookie.clone();
    let r = get(&app, &format!("/auth/google/callback?code=c&state={}", logins[0].state()), last_cookie.as_deref()).await;
    assert_eq!(r.status, 401, "the fifth login evicted the first");
    let r = get(&app, &format!("/auth/google/callback?code=c&state={}", logins[1].state()), last_cookie.as_deref()).await;
    assert_eq!(r.status, 200);
}

// ---- the session as the store ----

fn session_app_parts() -> SessionOptions {
    session_options()
}

async fn session_app(mock: &Mock) -> (corvis::CorvisApplication, Arc<Cfg>) {
    let options = OAuth2Options::google("google-client-id", CLIENT_SECRET, "https://app.example/auth/google/callback")
        .endpoints(endpoints(mock))
        .scope(["email"])
        .state_in_session();
    let cfg = Cfg::new(options);
    (AppBuilder::default().google(&cfg).session(session_app_parts()).build().await, cfg)
}

#[tokio::test]
async fn the_session_keeps_the_state_and_a_replay_is_refused_without_the_provider() {
    let mock = Mock::start(standard).await;
    let (app, cfg) = session_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    let session_cookie = l.cookie.clone().expect("the session cookie");
    assert!(session_cookie.starts_with("connect.sid="), "{session_cookie}");
    assert_eq!(set_cookies(&l.response).len(), 1, "only the session cookie, no state cookie");
    assert_eq!(l.response.header("cache-control"), Some("no-store"));
    let r = get(&app, &format!("/auth/google/callback?code=c&state={}", l.state()), Some(&session_cookie)).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(challenge_of(&mock.hits_to("/token")[0].field("code_verifier").unwrap()), l.param("code_challenge").unwrap());
    // the second callback with the same session cookie is refused and sends no second token request
    let again = get(&app, &format!("/auth/google/callback?code=c&state={}", l.state()), Some(&session_cookie)).await;
    assert_eq!((again.status, again.json_value()), (401, unauthorized()));
    assert_eq!(mock.hits_to("/token").len(), 1);
    assert_eq!(cfg.seen().len(), 1);
}

#[tokio::test]
async fn session_storage_refuses_a_forged_or_missing_session() {
    let mock = Mock::start(standard).await;
    let (app, _) = session_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    let cookie = l.cookie.clone();
    assert_eq!(get(&app, "/auth/google/callback?code=c&state=forged", cookie.as_deref()).await.status, 401);
    assert_eq!(get(&app, &format!("/auth/google/callback?code=c&state={}", l.state()), None).await.status, 401);
    assert_eq!(mock.count(), 0);
    // the forged callback did not spend the real flow
    assert_eq!(get(&app, &format!("/auth/google/callback?code=c&state={}", l.state()), cookie.as_deref()).await.status, 200);
}

#[tokio::test]
async fn two_logins_in_one_session_both_complete() {
    let mock = Mock::start(standard).await;
    let (app, _) = session_app(&mock).await;
    let first = login(&app, "/auth/google").await;
    let second = login_with_cookie(&app, "/auth/google", first.cookie.as_deref()).await;
    let cookie = first.cookie.clone();
    assert_eq!(get(&app, &format!("/auth/google/callback?code=c&state={}", second.state()), cookie.as_deref()).await.status, 200);
    assert_eq!(get(&app, &format!("/auth/google/callback?code=c&state={}", first.state()), cookie.as_deref()).await.status, 200);
}

// ---- failure mapping ----

async fn challenge_of_callback(app: &corvis::CorvisApplication, l: &Login, query: &str) -> TestResponse {
    get(app, &format!("/auth/google/challenge?{query}&state={}", l.state()), l.cookie.as_deref()).await
}

#[tokio::test]
async fn request_caused_failures_are_401_with_a_fixed_challenge() {
    let mock = Mock::start(|r| match r.path() {
        "/token" if r.field("code").as_deref() == Some("reused") => {
            Reply::json(400, r#"{"error":"invalid_grant","error_description":"<script>alert(1)</script>"}"#)
        }
        _ => standard(r),
    })
    .await;
    let (app, _) = google_app(&mock).await;
    // consent denied: the plain 401 of the original
    let l = login(&app, "/auth/google").await;
    let r =
        get(&app, &format!("/auth/google/callback?error=access_denied&error_description=denied&state={}", l.state()), l.cookie.as_deref())
            .await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    // other provider errors are 401 too, with the error code in the challenge and nothing else
    let desc = "%3Cscript%3Ealert(1)%3C%2Fscript%3E";
    let l = login(&app, "/auth/google").await;
    let r = challenge_of_callback(&app, &l, &format!("error=server_error&error_description={desc}")).await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json_value(), json!({ "message": "provider error", "error": "server_error" }));
    assert!(!r.text().contains("script") && !format!("{:?}", r.headers).contains("script"));
    // denied consent through the challenge
    let l = login(&app, "/auth/google").await;
    let r = challenge_of_callback(&app, &l, "error=access_denied").await;
    assert_eq!(r.json_value(), json!({ "message": "access denied" }));
    // a malformed error code is dropped
    for bad in ["a".repeat(200), "%3Cb%3E".to_string()] {
        let l = login(&app, "/auth/google").await;
        let r = challenge_of_callback(&app, &l, &format!("error={bad}")).await;
        assert_eq!(r.json_value(), json!({ "message": "provider error" }), "{bad}");
    }
    assert_eq!(mock.count(), 0, "no provider error reached the provider");
    // a code the provider rejects
    let l = login(&app, "/auth/google").await;
    let r = challenge_of_callback(&app, &l, "code=reused").await;
    assert_eq!(r.json_value(), json!({ "message": "code rejected", "error": "invalid_grant" }));
    assert!(!r.text().contains("script"));
    let l = login(&app, "/auth/google").await;
    let r = get(&app, &format!("/auth/google/callback?code=reused&state={}", l.state()), l.cookie.as_deref()).await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
}

#[tokio::test]
async fn a_failed_attempt_is_visible_to_an_optional_guard() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    let r = get(&app, &format!("/auth/google/optional?error=server_error&state={}", l.state()), l.cookie.as_deref()).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json_value()["auth_info"], json!({ "message": "provider error", "error": "server_error" }));
    assert_eq!(r.json_value()["user"], Value::Null);
}

#[tokio::test]
async fn provider_failures_are_the_fixed_500() {
    let body = "PROVIDER-BODY-SECRET";
    type Case = (&'static str, Box<dyn Fn(&Recorded) -> Reply + Send + Sync>);
    let cases: Vec<Case> = vec![
        (
            "token endpoint down",
            Box::new(move |r| if r.path() == "/token" { Reply::json(500, &format!("{{\"x\":\"{body}\"}}")) } else { standard(r) }),
        ),
        (
            "token endpoint 403",
            Box::new(move |r| if r.path() == "/token" { Reply::json(403, &format!("{{\"x\":\"{body}\"}}")) } else { standard(r) }),
        ),
        ("not json", Box::new(move |r| if r.path() == "/token" { Reply::json(200, body) } else { standard(r) })),
        ("no access token", Box::new(|r| if r.path() == "/token" { Reply::json(200, "{}") } else { standard(r) })),
        ("html", Box::new(move |r| if r.path() == "/token" { Reply::typed(200, "text/html", body) } else { standard(r) })),
        (
            "unknown token type",
            Box::new(|r| if r.path() == "/token" { Reply::json(200, r#"{"access_token":"t","token_type":"mac"}"#) } else { standard(r) }),
        ),
        ("profile down", Box::new(move |r| if r.path() == "/userinfo" { Reply::json(500, body) } else { standard(r) })),
        ("profile not json", Box::new(move |r| if r.path() == "/userinfo" { Reply::typed(200, "text/plain", body) } else { standard(r) })),
        ("profile without subject", Box::new(|r| if r.path() == "/userinfo" { Reply::json(200, r#"{"name":"x"}"#) } else { standard(r) })),
        ("profile subject is a number", Box::new(|r| if r.path() == "/userinfo" { Reply::json(200, r#"{"sub":5}"#) } else { standard(r) })),
        ("profile redirects", Box::new(|r| if r.path() == "/userinfo" { Reply::redirect("http://127.0.0.1:1/x") } else { standard(r) })),
    ];
    for (what, script) in cases {
        let mock = Mock::start(script).await;
        let (app, cfg) = google_app(&mock).await;
        let (_, r) = flow(&app, "/auth/google", "c").await;
        assert_eq!((r.status, r.json_value()), (500, internal_error()), "{what}");
        assert!(!r.text().contains(body) && !format!("{:?}", r.headers).contains(body), "{what}: the provider's text reached the response");
        assert!(cfg.seen().is_empty(), "{what}: the validator ran");
        if what == "unknown token type" {
            assert!(mock.hits_to("/userinfo").is_empty(), "no profile request after an unusable token");
        }
    }
}

#[tokio::test]
async fn a_token_endpoint_that_redirects_is_not_followed() {
    let other = Mock::start(standard).await;
    let target = other.url("/token");
    let mock = Mock::start(move |r| if r.path() == "/token" { Reply::redirect(&target) } else { standard(r) }).await;
    let (app, _) = google_app(&mock).await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!((r.status, r.json_value()), (500, internal_error()));
    assert_eq!(other.count(), 0, "the redirect target received nothing");
}

#[tokio::test]
async fn a_form_encoded_token_response_is_accepted() {
    let mock =
        Mock::start(
            |r| if r.path() == "/token" { Reply::form(200, "access_token=t-form&token_type=bearer&scope=email") } else { standard(r) },
        )
        .await;
    let (app, cfg) = google_app(&mock).await;
    assert_eq!(flow(&app, "/auth/google", "c").await.1.status, 200);
    assert_eq!(cfg.seen()[0].access, "t-form");
    // GitHub answers a refused code with a 200 and an error member: that is the 401 too
    let mock = Mock::start(|r| {
        if r.path() == "/token" { Reply::form(200, "error=bad_verification_code&error_description=nope") } else { standard(r) }
    })
    .await;
    let (app, _) = google_app(&mock).await;
    assert_eq!(flow(&app, "/auth/google", "c").await.1.status, 401);
}

#[tokio::test]
async fn basic_client_authentication_form_encodes_the_credentials() {
    let mock = Mock::start(standard).await;
    let secret = "se:cr%et";
    let cfg = Cfg::new(google_options(&mock).client_auth(corvis::auth::oauth2::ClientAuth::Basic));
    let _ = secret;
    let app = AppBuilder::default().google(&cfg).build().await;
    assert_eq!(flow(&app, "/auth/google", "c").await.1.status, 200);
    let t = &mock.hits_to("/token")[0];
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            form_urlencoded::byte_serialize(b"google-client-id").collect::<String>(),
            form_urlencoded::byte_serialize(CLIENT_SECRET.as_bytes()).collect::<String>()
        ))
    );
    assert_eq!(t.header("authorization"), Some(expected.as_str()));
    assert!(t.field("client_secret").is_none() && t.field("client_id").is_none(), "{}", t.body);
}

#[tokio::test]
async fn failure_redirect_replaces_the_401_of_a_callback_only() {
    let mock =
        Mock::start(
            |r| if r.path() == "/token" && r.field("code").as_deref() == Some("down") { Reply::json(500, "{}") } else { standard(r) },
        )
        .await;
    let cfg = Cfg::new(google_options(&mock).failure_redirect("/login?error=oauth"));
    let app = AppBuilder::default().google(&cfg).build().await;
    let l = login(&app, "/auth/google").await;
    assert_eq!(l.location.path(), "/authorize", "the login start is not affected");
    let r = get(&app, &format!("/auth/google/callback?error=access_denied&state={}", l.state()), l.cookie.as_deref()).await;
    assert_eq!(r.status, 302);
    assert_eq!(r.header("location"), Some("/login?error=oauth"));
    let r = get(&app, "/auth/google/callback?code=x&state=forged", None).await;
    assert_eq!((r.status, r.header("location")), (302, Some("/login?error=oauth")));
    let l = login(&app, "/auth/google").await;
    let r = callback(&app, "/auth/google/callback", &l, "down").await;
    assert_eq!((r.status, r.json_value()), (500, internal_error()), "an error is not redirected");
}

// ---- return_to ----

async fn login_with_return(app: &corvis::CorvisApplication, query: &str) -> Login {
    login(app, &format!("/auth/google?{query}")).await
}

async fn return_app(mock: &Mock) -> corvis::CorvisApplication {
    let cfg = Cfg::new(google_options(mock).return_to_param("return_to"));
    AppBuilder::default().google(&cfg).build().await
}

#[tokio::test]
async fn return_to_is_kept_in_the_record_and_never_sent_to_the_provider() {
    let mock = Mock::start(standard).await;
    let app = return_app(&mock).await;
    let l = login_with_return(&app, "return_to=%2Fcats%2F7%3Ftab%3Dtoys").await;
    assert!(!l.location.as_str().contains("cats") && !l.location.as_str().contains("return_to"), "{}", l.location);
    let r = callback(&app, "/auth/google/callback", &l, "c").await;
    assert_eq!(r.json_value()["return_to"], "/cats/7?tab=toys");
    // a callback query value is never consulted
    let l = login_with_return(&app, "other=1").await;
    let r = get(&app, &format!("/auth/google/callback?code=c&state={}&return_to=/admin", l.state()), l.cookie.as_deref()).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json_value()["return_to"], Value::Null);
}

#[tokio::test]
async fn unsafe_return_to_values_are_dropped_and_the_login_continues() {
    let mock = Mock::start(standard).await;
    let app = return_app(&mock).await;
    let long = format!("%2F{}", "a".repeat(2000));
    for value in [
        "https%3A%2F%2Fevil.example",
        "%2F%2Fevil.example",
        "%2F%5Cevil.example",
        "%2Fa%5Cb",
        "%2Fa%0Ab",
        "%2Fa%00b",
        "%2Fcaf%C3%A9",
        "cats",
        &long,
    ] {
        let l = login_with_return(&app, &format!("return_to={value}")).await;
        let r = callback(&app, "/auth/google/callback", &l, "c").await;
        assert_eq!(r.status, 200, "{value}");
        assert_eq!(r.json_value()["return_to"], Value::Null, "{value}");
    }
    let l = login_with_return(&app, "return_to=%2Fa&return_to=%2Fb").await;
    assert_eq!(callback(&app, "/auth/google/callback", &l, "c").await.json_value()["return_to"], Value::Null);
    // the option is off by default
    let (plain, _) = google_app(&mock).await;
    let l = login_with_return(&plain, "return_to=%2Fcats").await;
    assert_eq!(callback(&plain, "/auth/google/callback", &l, "c").await.json_value()["return_to"], Value::Null);
}

#[tokio::test]
async fn a_failed_callback_has_no_return_to() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(google_options(&mock).return_to_param("return_to"));
    let app = AppBuilder::default().google(&cfg).build().await;
    cfg.set_verdict(Verdict::Refuse);
    let l = login_with_return(&app, "return_to=%2Fcats").await;
    let r = callback(&app, "/auth/google/optional", &l, "c").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json_value()["return_to"], Value::Null);
}

// ---- the provider client ----

#[tokio::test]
async fn an_oversized_response_stops_the_read_with_and_without_a_length() {
    for content_length in [true, false] {
        let mock = Mock::start(
            move |r| if r.path() == "/token" { Reply::Stream { content_length, total: 256 * 1024 * 1024 } } else { standard(r) },
        )
        .await;
        let (app, _) = google_app(&mock).await;
        let (_, r) = flow(&app, "/auth/google", "c").await;
        assert_eq!((r.status, r.json_value()), (500, internal_error()), "content_length {content_length}");
        let written = mock.written.load(Ordering::SeqCst);
        assert!(written < 128 * 1024 * 1024, "the mock managed to write {written} bytes (content_length {content_length})");
    }
}

#[tokio::test]
async fn oversized_headers_are_refused() {
    let mock = Mock::start(|r| {
        if r.path() == "/token" {
            let mut raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nX-Big: ".to_vec();
            raw.extend(std::iter::repeat_n(b'a', 20 * 1024));
            raw.extend_from_slice(b"\r\n\r\n{}");
            Reply::Raw(raw)
        } else {
            standard(r)
        }
    })
    .await;
    let (app, _) = google_app(&mock).await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!((r.status, r.json_value()), (500, internal_error()));
}

#[tokio::test]
async fn a_provider_that_never_answers_is_a_500_after_the_timeout_and_the_connection_is_closed() {
    let mock = Mock::start(|r| if r.path() == "/token" { Reply::Hang } else { standard(r) }).await;
    let cfg = Cfg::new(google_options(&mock).http(OAuth2HttpOptions::new().timeout(Duration::from_secs(1))));
    let app = AppBuilder::default().google(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!((r.status, r.json_value()), (500, internal_error()));
    tokio::time::timeout(Duration::from_secs(10), mock.closed.notified()).await.expect("the client closed the connection");
}

#[tokio::test]
async fn a_connection_that_cannot_be_made_is_a_500() {
    let mock = Mock::start(standard).await;
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let options = google_options(&mock).endpoints(
        OAuth2Endpoints::new(mock.url("/authorize"), format!("http://127.0.0.1:{dead}/token")).userinfo_url(mock.url("/userinfo")),
    );
    let cfg = Cfg::new(options);
    let app = AppBuilder::default().google(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!((r.status, r.json_value()), (500, internal_error()));
}

#[tokio::test]
async fn proxy_variables_are_ignored() {
    let proxy = Mock::start(standard).await;
    let url = format!("http://{}", proxy.addr);
    // SAFETY: set before the flow runs; nothing else in this binary reads these variables.
    unsafe {
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"] {
            std::env::set_var(name, &url);
        }
    }
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!(r.status, 200);
    assert_eq!(proxy.count(), 0, "the proxy received nothing");
    assert_eq!(mock.hits_to("/token").len(), 1);
}

#[tokio::test]
async fn too_many_requests_in_flight_answer_503_without_contacting_the_provider() {
    let gate = Arc::new(Notify::new());
    let g = gate.clone();
    let mock = Mock::start(move |r| if r.path() == "/token" { Reply::Wait(g.clone(), Box::new(standard(r))) } else { standard(r) }).await;
    let cfg = Cfg::new(google_options(&mock).http(OAuth2HttpOptions::new().max_in_flight(1)));
    let app = AppBuilder::default().google(&cfg).build().await;
    let (first, second) = (login(&app, "/auth/google").await, login(&app, "/auth/google").await);
    let held = {
        let client = request(&app);
        let state = first.state();
        let cookie = first.cookie.clone().unwrap();
        tokio::spawn(
            async move { client.get(&format!("/auth/google/callback?code=c&state={state}")).header("cookie", &cookie).send().await },
        )
    };
    mock.arrived.notified().await;
    let r = get(&app, &format!("/auth/google/callback?code=c&state={}", second.state()), second.cookie.as_deref()).await;
    assert_eq!(r.status, 503, "{}", r.text());
    // Spec "OAuth2 at its limit": the body of every 503 of the authentication module, byte for byte.
    assert_eq!(r.text(), r#"{"statusCode":503,"message":"Service Unavailable"}"#);
    assert_eq!(mock.hits_to("/token").len(), 1, "the extra callback never reached the provider");
    gate.notify_one();
    assert_eq!(held.await.unwrap().status, 200);
}

// ---- TLS ----

#[tokio::test]
async fn the_provider_certificate_must_be_trusted_and_match_the_host() {
    let pki = pki(&["localhost"], false);
    let tls_mock = Mock::start_tls(standard, pki.server.clone(), "localhost").await;
    let https_options = |http: OAuth2HttpOptions| {
        google_options(&tls_mock)
            .endpoints(OAuth2Endpoints::new(tls_mock.url("/authorize"), tls_mock.url("/token")).userinfo_url(tls_mock.url("/userinfo")))
            .http(http)
    };
    // not under a configured root: refused
    let cfg = Cfg::new(https_options(OAuth2HttpOptions::new()));
    let app = AppBuilder::default().google(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!((r.status, r.json_value()), (500, internal_error()));
    assert!(cfg.seen().is_empty());
    // added as an extra root: accepted
    let cfg = Cfg::new(https_options(OAuth2HttpOptions::new().extra_root_certificate(pki.root_der.clone())));
    let app = AppBuilder::default().google(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(cfg.seen().len(), 1);
}

#[tokio::test]
async fn a_certificate_for_another_host_is_refused_even_under_a_trusted_root() {
    let pki = pki(&["other.example"], false);
    let tls_mock = Mock::start_tls(standard, pki.server.clone(), "localhost").await;
    let options = google_options(&tls_mock)
        .endpoints(OAuth2Endpoints::new(tls_mock.url("/authorize"), tls_mock.url("/token")).userinfo_url(tls_mock.url("/userinfo")))
        .http(OAuth2HttpOptions::new().extra_root_certificate(pki.root_der.clone()));
    let cfg = Cfg::new(options);
    let app = AppBuilder::default().google(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!((r.status, r.json_value()), (500, internal_error()));
    assert!(tls_mock.hits().is_empty(), "no request was sent over the failed handshake");
}

#[tokio::test]
async fn a_plain_http_mock_behind_an_https_url_fails_the_handshake() {
    let mock = Mock::start(standard).await;
    let options = google_options(&mock).endpoints(
        OAuth2Endpoints::new(mock.url("/authorize"), format!("https://127.0.0.1:{}/token", mock.addr.port()))
            .userinfo_url(mock.url("/userinfo")),
    );
    let cfg = Cfg::new(options);
    let app = AppBuilder::default().google(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!((r.status, r.json_value()), (500, internal_error()));
}

// ---- profiles ----

#[tokio::test]
async fn the_github_profile_with_emails_and_the_primary_verified_address() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(github_options(&mock).scope(["user:email"]));
    let app = AppBuilder::default().github(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/github", "c").await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(
        r.json_value()["user"],
        json!({ "id": format!("github@{}:583231", mock.addr) }),
        "a numeric id becomes a string, under the provider of a strategy named like the preset that is not the preset"
    );
    let s = &cfg.seen()[0];
    let p = s.profile.as_ref().unwrap();
    assert_eq!(p["provider"], format!("github@{}", mock.addr));
    assert_eq!(p["username"], "octocat");
    assert_eq!(p["displayName"], "The Octocat");
    assert_eq!(p["profileUrl"], "https://github.com/octocat");
    assert_eq!(p["nodeId"], "MDQ6VXNlcjU4MzIzMQ==");
    assert_eq!(p["photos"], json!([{ "value": "https://avatars.example/u/583231" }]));
    assert_eq!(p["emails"][1], json!({ "value": "octocat@example.com", "primary": true, "verified": true }));
    assert_eq!(s.verified_email.as_deref(), Some("octocat@example.com"));
    let user = &mock.hits_to("/user")[0];
    assert_eq!(user.header("authorization"), Some("Bearer ACCESS-TOKEN-FAKE"));
    assert_eq!(user.header("accept"), Some("application/vnd.github+json"));
    assert!(user.header("user-agent").is_some_and(|u| !u.is_empty()));
    let emails = &mock.hits_to("/user/emails")[0];
    assert_eq!(emails.header("authorization"), Some("Bearer ACCESS-TOKEN-FAKE"));
    assert!(emails.query().is_empty() && user.query().is_empty());
}

#[tokio::test]
async fn without_the_email_scope_github_emails_are_not_requested() {
    let mock = Mock::start(|r| {
        if r.path() == "/user" { Reply::json(200, r#"{"id":"42","login":"o","email":"public@example.com"}"#) } else { standard(r) }
    })
    .await;
    let cfg = Cfg::new(github_options(&mock).scope(["read:org"]));
    let app = AppBuilder::default().github(&cfg).build().await;
    assert_eq!(flow(&app, "/auth/github", "c").await.1.status, 200);
    assert!(mock.hits_to("/user/emails").is_empty());
    let s = &cfg.seen()[0];
    assert_eq!(s.profile.as_ref().unwrap()["emails"], json!([{ "value": "public@example.com", "primary": false, "verified": false }]));
    assert_eq!(s.verified_email, None, "the public address of /user is not verified");
    // the guard's scope decides, not the configured one: read:org on a login that configured user:email
    let cfg = Cfg::new(github_options(&mock).scope(["user:email"]));
    let app = AppBuilder::default().github(&cfg).build().await;
    let before = mock.hits_to("/user/emails").len();
    let l = login(&app, "/auth/github/scoped").await;
    assert_eq!(l.param("scope"), Some("read:org"));
    let r = get(&app, &format!("/auth/github/scoped/callback?code=c&state={}", l.state()), l.cookie.as_deref()).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(mock.hits_to("/user/emails").len(), before, "the guard's scope has no e-mail access");
}

#[tokio::test]
async fn a_failed_github_email_request_fails_the_login() {
    let mock = Mock::start(|r| if r.path() == "/user/emails" { Reply::json(500, "{}") } else { standard(r) }).await;
    let cfg = Cfg::new(github_options(&mock).scope(["user:email"]));
    let app = AppBuilder::default().github(&cfg).build().await;
    let (_, r) = flow(&app, "/auth/github", "c").await;
    assert_eq!((r.status, r.json_value()), (500, internal_error()));
    assert!(cfg.seen().is_empty());
}

#[tokio::test]
async fn an_unverified_primary_github_address_is_not_a_verified_email() {
    let emails = r#"[{"email":"p@example.com","primary":true,"verified":false},{"email":"s@example.com","primary":false,"verified":true}]"#;
    let mock = Mock::start(move |r| if r.path() == "/user/emails" { Reply::json(200, emails) } else { standard(r) }).await;
    let cfg = Cfg::new(github_options(&mock).scope(["user"]));
    let app = AppBuilder::default().github(&cfg).build().await;
    assert_eq!(flow(&app, "/auth/github", "c").await.1.status, 200);
    assert_eq!(cfg.seen()[0].verified_email, None);
}

#[tokio::test]
async fn an_unverified_google_address_is_not_a_verified_email() {
    let mock = Mock::start(|r| {
        if r.path() == "/userinfo" {
            Reply::json(200, r#"{"sub":"1","email":"a@example.com","email_verified":false}"#)
        } else {
            standard(r)
        }
    })
    .await;
    let (app, cfg) = google_app(&mock).await;
    assert_eq!(flow(&app, "/auth/google", "c").await.1.status, 200);
    let s = &cfg.seen()[0];
    assert_eq!(s.verified_email, None);
    assert_eq!(s.profile.as_ref().unwrap()["emails"], json!([{ "value": "a@example.com", "verified": false }]));
}

// ---- refresh ----

#[tokio::test]
async fn refresh_sends_the_request_of_the_original_and_returns_the_new_tokens() {
    let mock = Mock::start(|r| {
        if r.path() == "/token" {
            Reply::json(200, r#"{"access_token":"NEW-ACCESS","refresh_token":"NEW-REFRESH","expires_in":1800,"token_type":"Bearer"}"#)
        } else {
            standard(r)
        }
    })
    .await;
    let client = OAuth2Client::new(google_options(&mock)).unwrap();
    let tokens = client.refresh("OLD-REFRESH").await.unwrap();
    assert_eq!(tokens.access_token.expose(), "NEW-ACCESS");
    assert_eq!(tokens.refresh_token.as_ref().unwrap().expose(), "NEW-REFRESH");
    assert_eq!(tokens.params.expires_in(), Some(1800));
    let t = &mock.hits_to("/token")[0];
    let names: Vec<String> = t.form().into_iter().map(|(k, _)| k).collect();
    assert_eq!(names, ["grant_type", "client_id", "client_secret", "refresh_token"]);
    assert_eq!(t.field("grant_type").as_deref(), Some("refresh_token"));
    assert_eq!(t.field("refresh_token").as_deref(), Some("OLD-REFRESH"));
    let printed = format!("{client:?} {tokens:?}");
    assert!(!printed.contains("NEW-") && !printed.contains(CLIENT_SECRET), "{printed}");
}

#[tokio::test]
async fn a_rejected_refresh_names_the_code_and_nothing_else() {
    let mock = Mock::start(|_| Reply::json(400, r#"{"error":"invalid_grant","error_description":"PROVIDER-TEXT"}"#)).await;
    let client = OAuth2Client::new(google_options(&mock)).unwrap();
    let error = client.refresh("RT").await.unwrap_err();
    assert_eq!(error, OAuth2Error::Rejected { code: Some("invalid_grant".into()) });
    let text = error.to_string();
    assert!(text.contains("invalid_grant") && !text.contains("PROVIDER-TEXT"), "{text}");
}

#[tokio::test]
async fn the_client_validates_its_options() {
    let error = OAuth2Client::new(OAuth2Options::google("id", CLIENT_SECRET, "/relative").scope(["email"])).unwrap_err().to_string();
    assert!(error.contains("redirect_uri") && !error.contains(CLIENT_SECRET), "{error}");
}

// ---- Debug ----

#[tokio::test]
async fn debug_output_holds_no_secret() {
    let mock = Mock::start(standard).await;
    let options = google_options(&mock);
    let client = OAuth2Client::new(options.clone()).unwrap();
    for printed in [format!("{options:?}"), format!("{client:?}"), format!("{:?}", keys())] {
        for secret in [CLIENT_SECRET, STATE_SECRET] {
            assert!(!printed.contains(secret), "{printed}");
        }
    }
    let (app, cfg) = google_app(&mock).await;
    let (_, r) = flow(&app, "/auth/google", "c").await;
    assert_eq!(r.status, 200);
    let seen = cfg.seen();
    assert!(!seen[0].debug.contains("TOKEN"));
}
