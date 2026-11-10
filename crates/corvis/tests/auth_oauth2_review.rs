//! Regression tests of the first security review of `auth/oauth2`: one flow replayed to exhaust the in-flight slots
//! (M1), the namespace of a profile `id` across instances of one provider (M2), a forged record with an impossible
//! timestamp (I1) and the validator's refusal with a failure page (I2). The log warnings of the cookie names (L1) are
//! in `auth_oauth2_log.rs` and the example's error body (L2) in the example's own tests.
#![cfg(feature = "auth-oauth2")]

mod oauth2_common;

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use corvis::auth::oauth2::OAuth2HttpOptions;
use corvis::testing::request;
use oauth2_common::*;
use serde_json::json;
use tokio::sync::Notify;

fn tokens(mock: &Mock) -> usize {
    mock.hits_to("/token").len()
}

/// A mock whose token endpoint holds the answer to the code `hold` until `gate` is signalled.
fn gated(gate: &Arc<Notify>) -> impl Fn(&Recorded) -> Reply + Send + Sync + 'static {
    let gate = gate.clone();
    move |r| {
        if r.path() == "/token" && r.field("code").as_deref() == Some("hold") {
            Reply::Wait(gate.clone(), Box::new(standard(r)))
        } else {
            standard(r)
        }
    }
}

// ---- M1: one client replaying its own flow ----

#[tokio::test]
async fn concurrent_replays_of_one_flow_cost_one_token_request() {
    let gate = Arc::new(Notify::new());
    let mock = Mock::start(gated(&gate)).await;
    let (app, cfg) = google_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    let (state, cookie) = (l.state(), l.cookie.clone().unwrap());
    let held = {
        let client = request(&app);
        let (state, cookie) = (state.clone(), cookie.clone());
        tokio::spawn(
            async move { client.get(&format!("/auth/google/callback?code=hold&state={state}")).header("cookie", &cookie).send().await },
        )
    };
    mock.arrived.notified().await;
    // the flow is being redeemed right now: every copy of the callback is refused with the uniform failure
    for i in 0..60 {
        let r = get(&app, &format!("/auth/google/callback?code=random-{i}&state={state}"), Some(&cookie)).await;
        assert_eq!((r.status, r.json_value()), (401, unauthorized()), "replay {i}");
    }
    assert_eq!(tokens(&mock), 1, "sixty replays reached the provider {} times", tokens(&mock) - 1);
    gate.notify_one();
    assert_eq!(held.await.unwrap().status, 200);
    // and once it has completed
    for i in 0..5 {
        let r = get(&app, &format!("/auth/google/callback?code=late-{i}&state={state}"), Some(&cookie)).await;
        assert_eq!(r.status, 401);
    }
    assert_eq!(tokens(&mock), 1);
    assert_eq!(cfg.seen().len(), 1);
}

#[tokio::test]
async fn forged_states_never_enter_the_set_and_the_real_flow_still_completes() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let l = login(&app, "/auth/google").await;
    for i in 0..200 {
        let state = format!("{:A>43}", i);
        let r = get(&app, &format!("/auth/google/callback?code=x&state={state}"), l.cookie.as_deref()).await;
        assert_eq!(r.status, 401);
    }
    assert_eq!(tokens(&mock), 0);
    assert_eq!(callback(&app, "/auth/google/callback", &l, "c").await.status, 200);
}

#[tokio::test]
async fn the_set_is_bounded_and_the_oldest_states_are_forgotten() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(google_options(&mock).redeemed_states_max(16));
    let app = AppBuilder::default().google(&cfg).build().await;
    let mut logins = Vec::new();
    for i in 0..20 {
        let l = login(&app, "/auth/google").await;
        assert_eq!(callback(&app, "/auth/google/callback", &l, &format!("c{i}")).await.status, 200);
        logins.push(l);
    }
    assert_eq!(tokens(&mock), 20);
    // the newest are still refused locally
    assert_eq!(callback(&app, "/auth/google/callback", &logins[19], "again").await.status, 401);
    assert_eq!(tokens(&mock), 20);
    // the oldest were dropped to keep the memory bounded: a cost control of one instance, not a guarantee
    assert_eq!(callback(&app, "/auth/google/callback", &logins[0], "again").await.status, 200);
    assert_eq!(tokens(&mock), 21);
}

#[tokio::test]
async fn one_client_address_cannot_hold_every_slot() {
    let gate = Arc::new(Notify::new());
    let mock = Mock::start(gated(&gate)).await;
    // 8 slots, so a quarter (2) per client address
    let cfg = Cfg::new(google_options(&mock).http(OAuth2HttpOptions::new().max_in_flight(8)));
    let app = AppBuilder::default().google(&cfg).trust_proxy().build().await;
    let (a, b) = ("203.0.113.1", "203.0.113.2");
    let mut held = Vec::new();
    for _ in 0..2 {
        let l = parse_login(request(&app).get("/auth/google").header("x-forwarded-for", a).send().await);
        let client = request(&app);
        let (state, cookie) = (l.state(), l.cookie.clone().unwrap());
        held.push(tokio::spawn(async move {
            client
                .get(&format!("/auth/google/callback?code=hold&state={state}"))
                .header("cookie", &cookie)
                .header("x-forwarded-for", a)
                .send()
                .await
        }));
        mock.arrived.notified().await;
    }
    // a third flow of the same client is answered 503 without a token request, and burns nothing of the flow
    let third = parse_login(request(&app).get("/auth/google").header("x-forwarded-for", a).send().await);
    let target = format!("/auth/google/callback?code=ok&state={}", third.state());
    let r = request(&app).get(&target).header("cookie", third.cookie.as_deref().unwrap()).header("x-forwarded-for", a).send().await;
    assert_eq!(r.status, 503, "{}", r.text());
    assert_eq!(tokens(&mock), 2);
    // another address proceeds while the first one is at its share
    let other = parse_login(request(&app).get("/auth/google").header("x-forwarded-for", b).send().await);
    let target_b = format!("/auth/google/callback?code=ok&state={}", other.state());
    let r = request(&app).get(&target_b).header("cookie", other.cookie.as_deref().unwrap()).header("x-forwarded-for", b).send().await;
    assert_eq!(r.status, 200, "{}", r.text());
    gate.notify_one();
    gate.notify_one();
    for h in held {
        assert_eq!(h.await.unwrap().status, 200);
    }
    // the slots are given back: the refused flow completes now
    let r = request(&app).get(&target).header("cookie", third.cookie.as_deref().unwrap()).header("x-forwarded-for", a).send().await;
    assert_eq!(r.status, 200, "{}", r.text());
}

#[tokio::test]
async fn the_per_client_share_is_configurable_and_validated() {
    let mock = Mock::start(standard).await;
    for bad in [0usize, 1025] {
        let cfg = Cfg::new(google_options(&mock).http(OAuth2HttpOptions::new().max_in_flight_per_client(bad)));
        let error = AppBuilder::default().google(&cfg).try_build().await.err().expect("refused");
        assert!(error.contains("max_in_flight_per_client") && error.contains("google"), "{error}");
    }
    for bad in [0usize, 15, 10_000_001] {
        let cfg = Cfg::new(google_options(&mock).redeemed_states_max(bad));
        let error = AppBuilder::default().google(&cfg).try_build().await.err().expect("refused");
        assert!(error.contains("redeemed_states_max"), "{error}");
    }
    let ok = Cfg::new(
        google_options(&mock).http(OAuth2HttpOptions::new().max_in_flight(1024).max_in_flight_per_client(1024)).redeemed_states_max(16),
    );
    AppBuilder::default().google(&ok).build().await;
}

// ---- M2: the namespace of an id ----

fn ghe_script(r: &Recorded) -> Reply {
    match r.path() {
        "/user" => Reply::json(200, r#"{"id":583231,"sub":"583231","login":"octocat"}"#),
        _ => standard(r),
    }
}

#[tokio::test]
async fn the_provider_of_a_profile_is_the_strategy_name_unless_the_preset_is_used_unchanged() {
    let mock = Mock::start(ghe_script).await;
    // A GitHub preset pointed at another instance (here the mock), named `corp`.
    let ghe = Cfg::new(github_options(&mock));
    let app = AppBuilder::default().generic(&ghe).build().await;
    let (_, r) = flow_of(&app, "/auth/corp").await;
    assert_eq!(r.json_value()["user"], json!({ "id": "corp:583231" }), "{}", r.text());
    // The same subject through a generic strategy named `sso`.
    let sso = Cfg::new(
        corvis::auth::oauth2::OAuth2Options::new(
            corvis::auth::oauth2::OAuth2Endpoints::new(mock.url("/authorize"), mock.url("/token")).userinfo_url(mock.url("/user")),
            "sso-client-id",
            CLIENT_SECRET,
            "https://app.example/auth/sso/callback",
        )
        .state_cookie(keys()),
    );
    let app = AppBuilder::default().sso(&sso).build().await;
    let (_, r) = flow_of(&app, "/auth/sso").await;
    assert_eq!(r.json_value()["user"], json!({ "id": "sso:583231" }), "{}", r.text());
    // A strategy NAMED `github` that does not use the preset's own endpoints (here the mock is behind it) is not
    // github.com either: its name alone would put another server's ids into github.com's namespace, so it gets the
    // issuer appended. The namespaces of the same subject therefore all differ: github (the unchanged preset, checked in
    // the options' unit test), github@host (named like the preset, other endpoints), corp (preset with other
    // endpoints) and sso (generic).
    let github = Cfg::new(github_options(&mock));
    let app = AppBuilder::default().github(&github).build().await;
    let (_, r) = flow_of(&app, "/auth/github").await;
    assert_eq!(r.json_value()["user"], json!({ "id": format!("github@{}:583231", mock.addr) }));
    // The issuer is the host of the authorization endpoint.
    let expected = mock.addr.to_string();
    for cfg in [&ghe, &sso, &github] {
        assert_eq!(cfg.seen()[0].issuer.as_deref(), Some(expected.as_str()));
    }
}

async fn flow_of(app: &corvis::CorvisApplication, base: &str) -> (Login, corvis::testing::TestResponse) {
    let l = login(app, base).await;
    let r = callback(app, &format!("{base}/callback"), &l, "c").await;
    assert_eq!(r.status, 200, "{}", r.text());
    (l, r)
}

// ---- I1: an impossible timestamp ----

/// A state cookie signed with the server's own key, as the server would have issued it at `issued`.
fn forge(name: &str, strategy: &str, state: &str, issued: i64) -> String {
    let record = json!({ "n": strategy, "f": [{ "s": state, "t": issued }] }).to_string();
    let signed = keys().sign(&URL_SAFE_NO_PAD.encode(record.as_bytes()));
    format!("{name}={}", percent_encoding::utf8_percent_encode(&signed, percent_encoding::NON_ALPHANUMERIC))
}

#[tokio::test]
async fn a_record_with_an_impossible_timestamp_is_an_invalid_flow() {
    let mock = Mock::start(standard).await;
    let (app, _) = google_app(&mock).await;
    let state = "B".repeat(43);
    for issued in [i64::MIN, i64::MIN + 1, i64::MAX, i64::MAX - 1, 0, -1] {
        let cookie = forge("__Host-oauth2.google", "google", &state, issued);
        let r = get(&app, &format!("/auth/google/callback?code=x&state={state}"), Some(&cookie)).await;
        assert_eq!((r.status, r.json_value()), (401, unauthorized()), "issued {issued}");
    }
    assert_eq!(tokens(&mock), 0);
}

// ---- I2: the validator's refusal with a failure page ----

#[tokio::test]
async fn with_a_failure_page_every_refusal_goes_there_including_the_validators() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(google_options(&mock).failure_redirect("/login?error=oauth"));
    let app = AppBuilder::default().google(&cfg).build().await;
    for verdict in [Verdict::Refuse, Verdict::Empty] {
        cfg.set_verdict(verdict);
        let l = login(&app, "/auth/google").await;
        let r = callback(&app, "/auth/google/callback", &l, "c").await;
        assert_eq!((r.status, r.header("location")), (302, Some("/login?error=oauth")));
        assert!(r.text().is_empty() || !r.text().contains("Unauthorized"), "{}", r.text());
    }
    // an exception of the validator is passed on as it is
    cfg.set_verdict(Verdict::Forbidden);
    let l = login(&app, "/auth/google").await;
    assert_eq!(callback(&app, "/auth/google/callback", &l, "c").await.status, 403);
    // without a failure page the refusal stays a 401
    let plain = Cfg::new(google_options(&mock));
    let app = AppBuilder::default().google(&plain).build().await;
    plain.set_verdict(Verdict::Refuse);
    let l = login(&app, "/auth/google").await;
    let r = callback(&app, "/auth/google/callback", &l, "c").await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
}

/// The router's own 404 names the URL it did not find: with the redacted form, a callback URL that misses its route (a
/// typo in the path) does not answer the `code`, the `state` or the provider's error text back.
#[tokio::test]
async fn a_callback_url_that_misses_its_route_does_not_answer_its_query_back() {
    let mock = Mock::start(standard).await;
    let cfg = Cfg::new(github_options(&mock));
    let app = AppBuilder::default().github(&cfg).build().await;
    let url = "/auth/github/callbackZ?code=CODEX6&state=STATEX6&error_description=DESCX6&error_uri=URIX6&page=2";
    let r = corvis::testing::request(&app).get(url).send().await;
    assert_eq!(r.status, 404, "{}", r.text());
    let body = r.text();
    assert!(body.contains("/auth/github/callbackZ") && body.contains("page=2"), "{body}");
    for secret in ["CODEX6", "STATEX6", "DESCX6", "URIX6"] {
        assert!(!body.contains(secret), "{body}");
    }
}
