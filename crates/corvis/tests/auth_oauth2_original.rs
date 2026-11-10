//! The OAuth2 strategies against what the original packages did, recorded ahead of time by
//! `tests/fixtures/auth/oauth2.mjs` (`passport-oauth2` 1.8.0, `passport-google-oauth20` 2.0.0, `passport-github2`
//! 0.1.12, `passport-oauth2-refresh` 2.2.0) in `oauth2.json`. Every recorded case is replayed against a mock provider on
//! 127.0.0.1: no Node, no network. A case the fixture marks as a `divergence` is asserted from the specification.
#![cfg(feature = "auth-oauth2")]

mod oauth2_common;

use std::sync::Arc;

use base64::Engine as _;
use corvis::auth::oauth2::{OAuth2Client, OAuth2Endpoints, OAuth2Options};
use corvis::prelude::*;
use oauth2_common::*;
use sha2::{Digest, Sha256};

const FIXTURE: &str = include_str!("fixtures/auth/oauth2.json");
const ID: &str = "client-id-FAKE";
const SECRET: &str = "client-secret-FAKE";
const REDIRECT: &str = "https://app.example/auth/callback";

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

fn challenge_of(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The scenario of the fake provider of the generator, by the `code` of the token request and the token of a profile
/// request.
fn fake_provider(public_email: bool) -> impl Fn(&Recorded) -> Reply + Send + Sync + 'static {
    move |r| {
        let json = |status, v: Value| Reply::json(status, &v.to_string());
        match (r.method.as_str(), r.path()) {
            ("POST", "/token") => {
                if r.field("grant_type").as_deref() == Some("refresh_token") {
                    return json(
                        200,
                        json!({ "access_token": "ACCESS-refreshed", "refresh_token": "REFRESH-refreshed", "expires_in": 1800, "token_type": "Bearer" }),
                    );
                }
                let code = r.field("code").unwrap_or_default();
                match code.as_str() {
                    "norefresh" => json(200, json!({ "access_token": "ACCESS-ok", "token_type": "Bearer" })),
                    "form" => Reply::form(200, "access_token=ACCESS-ok&token_type=bearer"),
                    "badrequest" => json(400, json!({ "error": "invalid_grant", "error_description": "bad code" })),
                    "down" => json(500, json!({ "x": "y" })),
                    "redirect" => Reply::redirect("http://127.0.0.1:1/elsewhere"),
                    "garbage" => Reply::json(200, "not json"),
                    "noaccess" => json(200, json!({})),
                    _ => json(
                        200,
                        json!({ "access_token": format!("ACCESS-{code}"), "token_type": "Bearer", "refresh_token": format!("REFRESH-{code}"), "expires_in": 3600, "scope": "email profile" }),
                    ),
                }
            }
            ("GET", path) if ["/userinfo", "/user", "/user/emails"].contains(&path) => {
                let token = r.header("authorization").map(|a| a.trim_start_matches("Bearer ").to_string()).unwrap_or_default();
                if token == "ACCESS-profile500" {
                    return json(500, json!({}));
                }
                match path {
                    "/userinfo" => Reply::json(200, GOOGLE_DOC),
                    "/user" => {
                        let mut user: Value = serde_json::from_str(GITHUB_USER).unwrap();
                        if public_email {
                            user["email"] = json!("public@example.com");
                        }
                        json(200, user)
                    }
                    _ => Reply::json(200, GITHUB_EMAILS),
                }
            }
            _ => json(404, json!({})),
        }
    }
}

/// The options of a configuration of the fixture, pointed at `mock`.
fn options(config: &str, mock: &Mock) -> OAuth2Options {
    let authorize = if config == "generic-tenant" { mock.url("/authorize?tenant=a") } else { mock.url("/authorize") };
    let plain = OAuth2Endpoints::new(authorize.clone(), mock.url("/token"));
    let google = || OAuth2Endpoints::new(authorize.clone(), mock.url("/token")).userinfo_url(mock.url("/userinfo"));
    let github =
        || OAuth2Endpoints::new(authorize.clone(), mock.url("/token")).userinfo_url(mock.url("/user")).emails_url(mock.url("/user/emails"));
    let options = match config {
        "generic" | "generic-tenant" => OAuth2Options::new(plain, ID, SECRET, REDIRECT),
        "generic-scope" => OAuth2Options::new(plain, ID, SECRET, REDIRECT).scope(["a", "b"]),
        "google" => OAuth2Options::google(ID, SECRET, REDIRECT).endpoints(google()).scope(["email", "profile"]),
        "google-offline" => OAuth2Options::google(ID, SECRET, REDIRECT)
            .endpoints(google())
            .scope(["email", "profile"])
            .authorization_param("access_type", "offline"),
        "github" => OAuth2Options::github(ID, SECRET, REDIRECT).endpoints(github()).scope(["user:email", "read:user"]),
        "github-default-emails" => OAuth2Options::github(ID, SECRET, REDIRECT).endpoints(github()).scope(["user:email"]),
        "github-no-email-scope" => OAuth2Options::github(ID, SECRET, REDIRECT).endpoints(github()).scope(["read:org"]),
        other => panic!("unknown configuration {other}"),
    };
    options.state_cookie(keys())
}

/// The route prefix and the strategy of a configuration.
fn route(config: &str) -> &'static str {
    match config {
        c if c.starts_with("generic") => "/auth/corp",
        c if c.starts_with("google") => "/auth/google",
        _ => "/auth/github",
    }
}

async fn app_for(config: &str, mock: &Mock) -> (corvis::CorvisApplication, Arc<Cfg>) {
    let cfg = Cfg::new(options(config, mock));
    let builder = AppBuilder::default();
    let builder = match route(config) {
        "/auth/corp" => builder.generic(&cfg),
        "/auth/google" => builder.google(&cfg),
        _ => builder.github(&cfg),
    };
    (builder.build().await, cfg)
}

fn pairs(value: &Value) -> Vec<(String, String)> {
    value.as_array().unwrap().iter().map(|p| (p[0].as_str().unwrap().to_string(), p[1].as_str().unwrap().to_string())).collect()
}

/// Compares recorded parameters, where `<state>`, `<challenge>` and `<verifier>` stand for any 43-character value.
fn assert_same_pairs(case: &str, got: &[(String, String)], want: &[(String, String)]) {
    assert_eq!(got.len(), want.len(), "{case}: {got:?} against {want:?}");
    for ((gk, gv), (wk, wv)) in got.iter().zip(want) {
        assert_eq!(gk, wk, "{case}: names in order, {got:?} against {want:?}");
        if wv.starts_with('<') && wv.ends_with('>') {
            assert_eq!(gv.len(), 43, "{case}: {gk} is a 43-character value");
        } else {
            assert_eq!(gv, wv, "{case}: {gk}");
        }
    }
}

#[tokio::test]
async fn the_authorization_redirects_have_the_parameters_of_the_original() {
    let fx = fixture();
    let mock = Mock::start(standard).await;
    for case in fx["authorization"].as_array().unwrap() {
        let config = case["config"].as_str().unwrap();
        let with_state = case["state_and_pkce"].as_bool().unwrap();
        let (app, _) = app_for(config, &mock).await;
        let l = login(&app, route(config)).await;
        assert_eq!(l.response.status, case["status"].as_u64().unwrap() as u16);
        assert_eq!(l.location.path(), case["location_path"].as_str().unwrap());
        let recorded = pairs(&case["params"]);
        let name = format!("{config} (state and pkce {with_state})");
        if with_state {
            assert_same_pairs(&name, &l.params, &recorded);
        } else {
            // divergence: Corvis always adds the challenge, its method and the state before `client_id`
            assert!(case["divergence"].is_string(), "{name}");
            let mut want = recorded.clone();
            let client = want.pop().unwrap();
            assert_eq!(client.0, "client_id");
            want.push(("code_challenge".into(), "<x>".into()));
            want.push(("code_challenge_method".into(), "S256".into()));
            want.push(("state".into(), "<x>".into()));
            want.push(client);
            assert_same_pairs(&name, &l.params, &want);
        }
    }
}

#[tokio::test]
async fn the_token_requests_have_the_fields_of_the_original() {
    let fx = fixture();
    for (config, recorded) in [
        ("generic", &fx["token_request"]["generic"]),
        ("google", &fx["token_request"]["google"]),
        ("github", &fx["token_request"]["github"]),
    ] {
        let mock = Mock::start(fake_provider(false)).await;
        let (app, _) = app_for(config, &mock).await;
        let l = login(&app, route(config)).await;
        let r = callback(&app, &format!("{}/callback", route(config)), &l, "ok").await;
        assert_eq!(r.status, 200, "{config}: {}", r.text());
        let t = &mock.hits_to("/token")[0];
        assert_eq!(t.method, recorded["method"].as_str().unwrap(), "{config}");
        assert_eq!(t.path(), recorded["path"].as_str().unwrap(), "{config}");
        assert_eq!(t.header("content-type"), recorded["content_type"].as_str(), "{config}");
        assert_eq!(t.header("authorization").is_some(), !recorded["authorization"].is_null(), "{config}");
        let form = t.form();
        assert_same_pairs(config, &form, &pairs(&recorded["form"]));
        let verifier = form.iter().find(|(k, _)| k == "code_verifier").unwrap().1.clone();
        assert_eq!(
            challenge_of(&verifier),
            l.param("code_challenge").unwrap(),
            "{config}: the verifier matches the challenge of the redirect"
        );
    }
    assert!(fx["token_request"]["divergence"].is_string());
}

#[tokio::test]
async fn the_profile_requests_go_to_the_paths_of_the_original_with_a_bearer_header() {
    let fx = fixture();
    for config in ["google", "github", "github-default-emails", "github-no-email-scope"] {
        let recorded = fx["profile_requests"][config].as_array().unwrap();
        let mock = Mock::start(fake_provider(false)).await;
        let (app, _) = app_for(config, &mock).await;
        let (_, r) = {
            let l = login(&app, route(config)).await;
            let r = callback(&app, &format!("{}/callback", route(config)), &l, "ok").await;
            (l, r)
        };
        assert_eq!(r.status, 200, "{config}");
        let sent: Vec<Recorded> = mock.hits().into_iter().filter(|h| h.path() != "/token").collect();
        assert_eq!(sent.len(), recorded.len(), "{config}");
        for (got, want) in sent.iter().zip(recorded) {
            assert_eq!(got.method, want["method"].as_str().unwrap(), "{config}");
            assert_eq!(got.path(), want["path"].as_str().unwrap(), "{config}");
            // divergence (profile_requests.divergence): the token is in a bearer header, never in the URL
            assert_eq!(got.header("authorization"), Some("Bearer ACCESS-ok"), "{config}");
            assert!(got.query().is_empty(), "{config}: {}", got.target);
        }
    }
    assert!(fx["profile_requests"]["divergence"].is_string());
}

fn drop_visibility(emails: &Value) -> Value {
    Value::Array(
        emails
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                let mut e = e.clone();
                e.as_object_mut().unwrap().remove("visibility");
                e
            })
            .collect(),
    )
}

#[tokio::test]
async fn the_profiles_equal_the_ones_the_original_hands_to_validate() {
    let fx = fixture();
    for config in ["google", "github", "github-default-emails", "github-no-email-scope"] {
        let recorded = &fx["profiles"][config];
        let mock = Mock::start(fake_provider(false)).await;
        let (app, cfg) = app_for(config, &mock).await;
        let l = login(&app, route(config)).await;
        assert_eq!(callback(&app, &format!("{}/callback", route(config)), &l, "ok").await.status, 200, "{config}");
        let seen = &cfg.seen()[0];
        let got = seen.profile.as_ref().unwrap();
        let want = &recorded["profile"];
        for key in ["id", "displayName", "name", "photos", "username", "profileUrl", "nodeId", "_json"] {
            assert_eq!(got.get(key), want.get(key), "{config}: {key}");
        }
        // A listed divergence: the original names the provider after the package; here a strategy named like a preset
        // that talks to another server (the mock) is not that provider, so its ids get the issuer in their namespace.
        let original = want["provider"].as_str().unwrap();
        assert_eq!(got["provider"], format!("{original}@{}", mock.addr), "{config}: provider");
        match config {
            "google" => assert_eq!(got["emails"], want["emails"], "google emails, flag included"),
            "github" => assert_eq!(got["emails"], drop_visibility(&want["emails"]), "github emails (allRawEmails) without `visibility`"),
            "github-default-emails" => {
                // divergence (profiles.divergence): every address with its flags, where the original keeps only the primary one
                let all = drop_visibility(&fx["profiles"]["github"]["profile"]["emails"]);
                assert_eq!(got["emails"], all);
                assert_eq!(want["emails"], json!([{ "value": "octocat@example.com" }]));
            }
            _ => assert!(
                got.get("emails").is_none() && want.get("emails").is_none(),
                "{config}: no e-mail without the scope and without a public address"
            ),
        }
        assert_eq!(seen.access, recorded["access_token"].as_str().unwrap(), "{config}");
        assert_eq!(seen.refresh.as_deref(), recorded["refresh_token"].as_str(), "{config}");
        assert_eq!(seen.expires_in, recorded["params"]["expires_in"].as_u64(), "{config}");
        assert_eq!(seen.scope.as_deref(), recorded["params"]["scope"].as_str(), "{config}");
    }
    // a public address of /user without the e-mail scope
    let recorded = &fx["profiles"]["github_public_email"]["profile"];
    let mock = Mock::start(fake_provider(true)).await;
    let (app, cfg) = app_for("github-no-email-scope", &mock).await;
    let l = login(&app, "/auth/github").await;
    assert_eq!(callback(&app, "/auth/github/callback", &l, "ok").await.status, 200);
    let got = cfg.seen()[0].profile.clone().unwrap();
    assert_eq!(got["emails"][0]["value"], recorded["emails"][0]["value"]);
    assert_eq!(got["emails"][0]["verified"], false, "the added flag: a public address is not verified");
}

#[tokio::test]
async fn the_callback_outcomes_match_the_original_except_where_the_fixture_says_otherwise() {
    let fx = fixture();
    for case in fx["outcomes"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mock = Mock::start(fake_provider(false)).await;
        let (app, cfg) = app_for(case["config"].as_str().unwrap(), &mock).await;
        let l = login(&app, "/auth/google").await;
        let query = case["query"].as_str().unwrap().replace("<state>", &l.state());
        if name == "validator_false" {
            cfg.set_verdict(Verdict::Refuse);
        }
        let r = get(&app, &format!("/auth/google/callback?{query}"), l.cookie.as_deref()).await;
        let recorded = case["status"].as_u64().unwrap() as u16;
        // What the specification says where the recorded behavior is a divergence or a different framing: the AuthGuard
        // of Corvis answers 401 for every failed attempt, and a request-caused failure is a 401 where the original 500s.
        let expected = match name {
            "state_mismatch" | "no_state" => 401, // the original answers the 403 the strategy gave
            "other_provider_error" | "token_400" => {
                assert!(case["divergence"].is_string(), "{name}");
                401
            }
            _ => recorded,
        };
        assert_eq!(r.status, expected, "{name}: {}", r.text());
        assert_eq!(mock.hits_to("/token").len() as u64, case["token_requests"].as_u64().unwrap(), "{name}: token requests");
        if expected == 500 {
            assert_eq!(r.json_value(), internal_error(), "{name}");
        }
    }
}

#[tokio::test]
async fn refresh_sends_the_form_of_passport_oauth2_refresh() {
    let fx = fixture();
    let recorded = &fx["refresh"];
    let mock = Mock::start(fake_provider(false)).await;
    let client = OAuth2Client::new(options("generic", &mock)).unwrap();
    let tokens = client.refresh("OLD-REFRESH").await.unwrap();
    let t = &mock.hits_to("/token")[0];
    assert_eq!(t.method, recorded["request"]["method"].as_str().unwrap());
    assert_eq!(t.header("content-type"), recorded["request"]["content_type"].as_str());
    assert_same_pairs("refresh", &t.form(), &pairs(&recorded["request"]["form"]));
    assert_eq!(tokens.access_token.expose(), recorded["access_token"].as_str().unwrap());
    assert_eq!(tokens.refresh_token.unwrap().expose(), recorded["refresh_token"].as_str().unwrap());
    assert_eq!(tokens.params.expires_in(), recorded["params"]["expires_in"].as_u64());
}

#[test]
fn the_fixture_names_its_sources_and_holds_no_real_credential() {
    let fx = fixture();
    for (package, version) in [
        ("passport-oauth2", "1.8.0"),
        ("passport-google-oauth20", "2.0.0"),
        ("passport-github2", "0.1.12"),
        ("passport-oauth2-refresh", "2.2.0"),
    ] {
        assert_eq!(fx["meta"][package], version);
    }
    assert!(FIXTURE.contains("client-secret-FAKE") && !FIXTURE.contains("localhost:") && !FIXTURE.contains("127.0.0.1:"));
}
