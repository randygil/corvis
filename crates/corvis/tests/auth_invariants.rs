//! The invariants of `auth/security-invariants`: properties that hold for every combination of the strategies, guards,
//! sessions and sockets of `corvis::auth`, one test per scenario of the specification, named after it. They need every
//! auth feature, so CI runs this binary with `--features "auth-local auth-api-key auth-session auth-oauth2"`. Counters
//! (validator calls, store operations, the provider's hit count), never timing. The threat model page names the test
//! of each mitigation: renaming a test means updating `docs/security/authentication-threat-model.md`.
#![cfg(all(feature = "jwt", feature = "auth-local", feature = "auth-api-key", feature = "auth-session", feature = "auth-oauth2"))]

mod auth_common;
mod oauth2_common;
mod session_common;

use auth_common::*;
use corvis::prelude::*;

// ---- the handler's user comes from the route's guard ----

#[tokio::test]
async fn smoke_every_strategy_authenticates_its_own_route() {
    let h = Harness::new().await;
    let r = h.get("/jwt", &[("authorization", &bearer(&token("1")))]).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("1".into())), "{}", r.text());
    let r = h.get("/key", &[("x-api-key", API_KEY)]).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("3".into())), "{}", r.text());
    let r = h.post_json("/login", &[], json!({ "username": "bob", "password": PASSWORD })).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("2".into())), "{}", r.text());
    let cookie = h.login("ana").await;
    let r = h.get("/session", &[("cookie", &cookie)]).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("1".into())), "{}", r.text());
    let l = oauth2_common::login(&h.app, "/oauth").await;
    let r = oauth2_common::callback(&h.app, "/oauth", &l, "code-1").await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(user_id(&r).unwrap().starts_with("gh:"));
}

#[tokio::test]
async fn session_of_one_user_and_token_of_another() {
    let h = Harness::new().await;
    let cookie = h.login("ana").await;
    let auth = bearer(&token("2"));
    let r = h.get("/jwt", &[("cookie", &cookie), ("authorization", &auth)]).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("2".into())), "{}", r.text());
    assert_eq!(h.probe.last_user("jwt").unwrap()["id"], "2");
    // The typed extractor reads the same user.
    let r = h.get("/jwt-typed", &[("cookie", &cookie), ("authorization", &auth)]).await;
    assert_eq!(r.json_value(), json!({ "typed": "2" }));
}

#[tokio::test]
async fn optional_token_guard_with_a_session_cookie() {
    let h = Harness::new().await;
    let cookie = h.login("ana").await;
    let auth = bearer(&expired_token("2"));
    let r = h.get("/jwt-opt", &[("cookie", &cookie), ("authorization", &auth)]).await;
    assert_eq!(r.status, 200);
    assert_eq!(user_id(&r), None, "the route's guard decides the identity, the session user is not the fallback");
    // No token at all: the same.
    let r = h.get("/jwt-opt", &[("cookie", &cookie)]).await;
    assert_eq!((r.status, user_id(&r)), (200, None));
    assert_eq!(h.probe.runs("jwt-opt"), 2);
}

#[tokio::test]
async fn session_strategy_in_the_chain() {
    let h = Harness::new().await;
    let cookie = h.login("ana").await;
    let auth = bearer(&expired_token("2"));
    let r = h.get("/chain-jwt-session", &[("cookie", &cookie), ("authorization", &auth)]).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("1".into())), "{}", r.text());
    // A valid token wins over the session, in the order of the chain.
    let r = h.get("/chain-jwt-session", &[("cookie", &cookie), ("authorization", &bearer(&token("2")))]).await;
    assert_eq!(user_id(&r), Some("2".into()));
}

/// Review finding: a token cookie sent twice (one planted from a sibling subdomain in front of the victim's own) yields
/// no token instead of the first value, so the attacker's token cannot replace the victim's identity.
#[tokio::test]
async fn a_jwt_cookie_sent_twice_is_no_token() {
    let h = Harness::new().await;
    let (attacker, victim) = (token("2"), token("1"));
    // One cookie: the token is read.
    let r = h.get("/jwt", &[("cookie", &format!("jwt={victim}"))]).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("1".into())), "{}", r.text());
    // Two, in either order, even with the same value: the uniform 401 and no validator call.
    let before = h.probe.jwt.load(std::sync::atomic::Ordering::SeqCst);
    for pair in [format!("jwt={attacker}; jwt={victim}"), format!("jwt={victim}; jwt={attacker}"), format!("jwt={victim}; jwt={victim}")] {
        let r = h.get("/jwt", &[("cookie", &pair)]).await;
        assert_eq!((r.status, r.json_value()), (401, unauthorized()), "{pair}");
    }
    assert_eq!(h.probe.jwt.load(std::sync::atomic::Ordering::SeqCst), before, "the validator never ran");
    // In a chain the session of the victim answers instead of the planted token.
    let session = h.login("ana").await;
    let r = h.get("/chain-jwt-session", &[("cookie", &format!("jwt={attacker}; jwt={attacker}; {session}"))]).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("1".into())), "{}", r.text());
    // A bearer header is still read next to a repeated cookie.
    let r = h.get("/jwt", &[("cookie", &format!("jwt={attacker}; jwt={victim}")), ("authorization", &bearer(&victim))]).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("1".into())), "{}", r.text());
}

#[tokio::test]
async fn token_only_route_with_only_a_session_cookie() {
    let h = Harness::new().await;
    let cookie = h.login("ana").await;
    let r = h.get("/jwt", &[("cookie", &cookie)]).await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    assert_eq!(h.probe.runs("jwt"), 0);
    // The same through the default guard and the API key route.
    assert_eq!(h.get("/default", &[("cookie", &cookie)]).await.status, 401);
    assert_eq!(h.get("/key", &[("cookie", &cookie)]).await.status, 401);
    assert_eq!(h.probe.runs("default") + h.probe.runs("key"), 0);
}

// ---- routes without an AuthGuard see the session user ----

#[tokio::test]
async fn roles_guard_without_an_auth_guard() {
    let h = Harness::new().await;
    // The decision of the review (docs: "Route posture" and the threat model): the session user is restored on every
    // route, so a guard that reads `Request::user()` without an `AuthGuard` in front of it sees the session's user.
    let admin = h.login("ana").await;
    let r = h.get("/roles", &[("cookie", &admin)]).await;
    assert_eq!((r.status, user_id(&r)), (200, Some("1".into())), "{}", r.text());
    let user = h.login("bob").await;
    assert_eq!(h.get("/roles", &[("cookie", &user)]).await.status, 403);
    assert_eq!(h.get("/roles", &[]).await.status, 403);
    // The route with no guard shows the same user.
    let r = h.get("/open", &[("cookie", &admin)]).await;
    assert_eq!(user_id(&r), Some("1".into()));
    // A token alone is not an identity on a route without an AuthGuard.
    let r = h.get("/open", &[("authorization", &bearer(&token("1")))]).await;
    assert_eq!(user_id(&r), None);
}

// ---- chains fail closed ----

#[tokio::test]
async fn failing_dependency_before_a_weaker_strategy() {
    let h = Harness::with(Config {
        provider: Box::new(|r| if r.path() == "/token" { oauth2_common::Reply::json(500, "{}") } else { oauth2_common::standard(r) }),
        ..Config::default()
    })
    .await;
    // The first strategy of the chain is the OAuth2 callback, whose provider is down, and the request carries a valid
    // API key as well.
    let login = oauth2_common::login(&h.app, "/chain-oauth-key").await;
    let target = format!("/chain-oauth-key?code=c&state={}", login.state());
    let cookie = login.cookie.clone().unwrap();
    let r = h.get(&target, &[("cookie", &cookie), ("x-api-key", API_KEY)]).await;
    assert_eq!(r.status, 500, "the error of the first strategy is the response: {}", r.text());
    assert_eq!(Probe::count(&h.probe.key), 0, "the API key validator did not run");
    assert_eq!(h.probe.runs("chain-oauth-key"), 0);
    assert_eq!(h.mock.hits_to("/token").len(), 1);
}

#[tokio::test]
async fn built_in_strategies_never_pass() {
    // The gate fuzz target (`fuzz_auth.rs`) covers random input; this is the fixed grid: no handler of a non-optional
    // guard runs without a user, whatever the credentials are.
    let h = Harness::new().await;
    let cookie = h.login("ana").await;
    let garbage = [
        vec![],
        vec![("authorization", "Bearer x.y.z".to_string())],
        vec![("authorization", bearer(&expired_token("1")))],
        vec![("authorization", bearer(&forged_token("1")))],
        vec![("x-api-key", "nope".to_string())],
        vec![("cookie", "connect.sid=s%3Anope.nope".to_string())],
        vec![("cookie", cookie.replace('.', "x"))],
    ];
    let routes = ["/jwt", "/key", "/session", "/chain-jwt-key", "/chain-key-jwt", "/chain-jwt-session", "/default", "/skip-private"];
    for headers in &garbage {
        for route in routes {
            let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let r = h.get(route, &borrowed).await;
            assert_eq!(r.status, 401, "{route} {headers:?}: {}", r.text());
        }
    }
    assert!(h.probe.hits().iter().all(|hit| hit.route == "login-session"), "a handler ran: {:?}", h.probe.hits());
}

// ---- one failure response for every reason ----

#[tokio::test]
async fn reasons_are_indistinguishable() {
    let h = Harness::new().await;
    let wrong = h.post_json("/login", &[], json!({ "username": "ana", "password": "wrong" })).await;
    let unknown = h.post_json("/login", &[], json!({ "username": "nobody", "password": "wrong" })).await;
    assert_eq!(wrong.status, 401);
    assert_eq!(shape(&wrong), shape(&unknown));
    assert!(set_cookies(&wrong).is_empty() && set_cookies(&unknown).is_empty(), "no Set-Cookie on a failed login");
    assert_eq!(wrong.json_value(), unauthorized());
    // Every other reason of the local strategy: missing and oversized credentials look the same.
    for body in [json!({}), json!({ "username": "ana" }), json!({ "username": "ana", "password": "x".repeat(5000) })] {
        assert_eq!(shape(&h.post_json("/login", &[], body.clone()).await), shape(&wrong), "{body}");
    }

    // Every reason of the other strategies, one route each.
    let logged_out = {
        let cookie = h.login("bob").await;
        h.post_json("/logout", &[("cookie", &cookie)], json!({})).await;
        cookie
    };
    let tampered = format!("{}x", h.login("bob").await);
    let reasons: Vec<(&str, Vec<(&str, String)>)> = vec![
        ("/jwt", vec![]),
        ("/jwt", vec![("authorization", bearer(&expired_token("1")))]),
        ("/jwt", vec![("authorization", bearer(&forged_token("1")))]),
        ("/jwt", vec![("authorization", "Bearer junk".to_string())]),
        ("/jwt", vec![("authorization", bearer(&token("99")))]),
        ("/key", vec![]),
        ("/key", vec![("x-api-key", "wrong".to_string())]),
        ("/session", vec![]),
        ("/session", vec![("cookie", tampered.clone())]),
        ("/session", vec![("cookie", logged_out.clone())]),
    ];
    let mut shapes = Vec::new();
    for (route, headers) in &reasons {
        let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let r = h.get(route, &borrowed).await;
        assert_eq!(r.status, 401, "{route} {headers:?}");
        assert!(set_cookies(&r).is_empty(), "{route}: {:?}", set_cookies(&r));
        shapes.push(shape(&r));
    }
    assert!(shapes.iter().all(|s| s == &shapes[0]), "{shapes:?}");
}

#[tokio::test]
async fn chain_position_is_not_revealed() {
    let h = Harness::new().await;
    let expired = h.get("/chain-jwt-key", &[("authorization", &bearer(&expired_token("1")))]).await;
    let wrong_key = h.get("/chain-jwt-key", &[("x-api-key", "wrong")]).await;
    let nothing = h.get("/chain-jwt-key", &[]).await;
    assert_eq!(expired.status, 401);
    assert_eq!(shape(&expired), shape(&wrong_key));
    assert_eq!(shape(&expired), shape(&nothing));
}

// ---- no authority-carrying cookie on a failed request ----

#[tokio::test]
async fn failed_login_with_an_anonymous_session() {
    let h = Harness::new().await;
    let touched = h.post_json("/touch", &[], json!({})).await;
    let cookie = session_pair(&touched).expect("the touch route creates a session");
    let id = session_common::id_of(&cookie);
    let before = h.store.records().await;
    assert!(before.contains_key(&id));
    let r = h.post_json("/login-session", &[("cookie", &cookie)], json!({ "username": "ana", "password": "wrong" })).await;
    assert_eq!(r.status, 401);
    assert_eq!(session_pair(&r), None, "no session cookie on a failed login: {:?}", set_cookies(&r));
    let after = h.store.records().await;
    assert_eq!(after.keys().collect::<std::collections::BTreeSet<_>>(), before.keys().collect::<std::collections::BTreeSet<_>>());
    assert!(after.contains_key(&id), "the identifier is unchanged in the store");
    // The same browser can still use its session.
    let r = h.post_json("/login-session", &[("cookie", &cookie)], json!({ "username": "ana", "password": PASSWORD })).await;
    assert_eq!(r.status, 200);
    // A browser with no session at all gets none from a failed login either, and no record is written.
    let records = h.store.len();
    let r = h.post_json("/login-session", &[], json!({ "username": "ana", "password": "wrong" })).await;
    assert_eq!(r.status, 401);
    assert!(set_cookies(&r).is_empty(), "{:?}", set_cookies(&r));
    assert_eq!(h.store.len(), records);
}

#[tokio::test]
async fn failed_oauth2_callback() {
    let h = Harness::new().await;
    let login = oauth2_common::login(&h.app, "/oauth").await;
    let cookie = login.cookie.clone().unwrap();
    let r = h.get("/oauth?code=c&state=forged-state", &[("cookie", &cookie)]).await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    // The only cookie of the response is the state cookie, and it holds no state the browser did not have: a forged
    // callback leaves the browser's genuine flow in place (the cookie is sent back as it was), and a callback that
    // finds no flow deletes the cookie.
    let set = set_cookies(&r);
    assert_eq!(set.len(), 1, "{set:?}");
    assert!(set[0].starts_with("__Host-oauth2.oauth="), "{set:?}");
    assert_eq!(set[0].split(';').next().unwrap(), cookie, "the cookie carries the flows the browser already had");
    assert_eq!(h.mock.count(), 0, "the provider was not contacted");
    // A callback with no state cookie at all sets nothing, or deletes.
    let r = h.get(&format!("/oauth?code=c&state={}", login.state()), &[]).await;
    assert_eq!(r.status, 401);
    assert!(set_cookies(&r).iter().all(|c| c.contains("Max-Age=0") || c.contains("1970")), "{:?}", set_cookies(&r));
    // The genuine flow is still usable after the forged callback.
    let r = h.get(&format!("/oauth?code=c&state={}", login.state()), &[("cookie", &cookie)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

// ---- public routes and guard order ----

#[tokio::test]
async fn invalid_credentials_on_a_public_route() {
    let h = Harness::new().await;
    let r = h.get("/skip-public", &[("authorization", &bearer(&expired_token("1")))]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(h.probe.runs("skip-public"), 1);
    assert_eq!(h.probe.strategy_calls(), 0, "no validator ran");
    assert!(h.probe.jwt_raw.lock().unwrap().is_empty(), "the key provider was not asked about the token");
}

#[tokio::test]
async fn controller_marked_public_handler_not_marked() {
    let h = Harness::new().await;
    // The handler's own guard was built without `.skip_public()`, so the controller's mark does not open it.
    let r = h.get("/pub/own-guard", &[]).await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    assert_eq!(h.probe.runs("pub/own-guard"), 0);
    let r = h.get("/pub/own-guard", &[("authorization", &bearer(&token("1")))]).await;
    assert_eq!(r.status, 200);
    // `.skip_public()` honors the controller's mark, and runs no strategy.
    let before = h.probe.jwt_raw.lock().unwrap().len();
    let r = h.get("/pub/skip", &[("authorization", &bearer(&expired_token("1")))]).await;
    assert_eq!(r.status, 200);
    assert_eq!(h.probe.jwt_raw.lock().unwrap().len(), before);
    // A guard with `.skip_public()` on a route that is not public authenticates.
    assert_eq!(h.get("/skip-private", &[]).await.status, 401);
}

#[tokio::test]
async fn posture_with_only_an_optional_guard() {
    let error = match Harness::try_with(Config { posture: true, ..Config::default() }).await {
        Ok(_) => panic!("the application started with unprotected routes"),
        Err(e) => e,
    };
    assert!(error.contains("Routes::jwt_opt"), "an optional guard is not access control: {error}");
    assert!(!error.contains("Routes::jwt\n") && !error.contains("Routes::jwt,"), "a required guard is: {error}");
}

// ---- credentials are evaluated only by the strategy configured for their place ----

#[tokio::test]
async fn bearer_value_shared_by_two_strategies() {
    let h = Harness::new().await;
    // A token-shaped value, so the JWT strategy reaches its key provider, which records what it extracted.
    let t = forged_token("1");
    let values = [
        format!("Bearer {t}"),
        format!("bearer {t}"),
        format!("BEARER {t}"),
        format!("Bearer   {t}"),
        format!("Bearer\t{t}"),
        format!("Bearer {t} trailing"),
        format!("  Bearer {t}"),
        format!("Bearer {t} "),
        format!("Basic {t}"),
        format!("Bearer{t}"),
        format!("Bearer: {t}"),
        t.to_string(),
        "Bearer".to_string(),
        "Bearer ".to_string(),
        format!("Bearer\u{a0}{t}"),
    ];
    for value in values {
        h.probe.jwt_raw.lock().unwrap().clear();
        h.probe.key_raw.lock().unwrap().clear();
        let r = h.get("/chain-jwt-key", &[("authorization", &value)]).await;
        assert_eq!(r.status, 401, "{value:?}");
        let jwt = h.probe.jwt_raw.lock().unwrap().clone();
        let key = h.probe.key_raw.lock().unwrap().clone();
        assert_eq!(jwt, key, "the two strategies read different values from {value:?}");
    }
    // The one place they differ, and in the safe direction: the API key reader gives up on a header with more than
    // `max_key_bytes + 128` bytes of leading whitespace, the token reader does not. What the API key strategy reads,
    // the token strategy reads too, never the other way.
    h.probe.jwt_raw.lock().unwrap().clear();
    h.probe.key_raw.lock().unwrap().clear();
    let padded = format!("{}Bearer {t}", " ".repeat(700));
    h.get("/chain-jwt-key", &[("authorization", &padded)]).await;
    assert_eq!(h.probe.jwt_raw.lock().unwrap().clone(), vec![t.clone()]);
    assert!(h.probe.key_raw.lock().unwrap().is_empty());
}

#[tokio::test]
async fn token_in_a_place_that_is_not_configured() {
    let h = Harness::new().await;
    let t = token("1");
    let cookie = h.login("ana").await;
    let session_value = cookie.split_once('=').unwrap().1.to_string();
    // A valid token as the session cookie, and as the API key header.
    assert_eq!(h.get("/session", &[("cookie", &format!("connect.sid={t}"))]).await.status, 401);
    assert_eq!(h.get("/key", &[("x-api-key", &t)]).await.status, 401);
    // A valid session cookie value as a bearer token or an API key.
    assert_eq!(h.get("/jwt", &[("authorization", &bearer(&session_value))]).await.status, 401);
    assert_eq!(h.get("/key", &[("x-api-key", &session_value)]).await.status, 401);
    // A valid API key as a bearer token on the token route, and as a token cookie.
    assert_eq!(h.get("/jwt", &[("authorization", &bearer(API_KEY))]).await.status, 401);
    assert_eq!(h.get("/jwt", &[("cookie", &format!("jwt={API_KEY}"))]).await.status, 401);
    // The same API key does authenticate where it is configured, as a header and as a bearer value.
    assert_eq!(h.get("/key", &[("x-api-key", API_KEY)]).await.status, 200);
    assert_eq!(h.get("/key", &[("authorization", &bearer(API_KEY))]).await.status, 200);
    assert_eq!(h.probe.runs("jwt"), 0);
}

// ---- an OAuth2 strategy ends a chain ----

#[tokio::test]
async fn an_oauth2_strategy_in_a_chain_redirects_instead_of_falling_through() {
    // The OAuth2 strategy starts a login for any request that is not a callback, so a chain that lists it first never
    // reaches the strategies after it, and one that lists it last turns every anonymous request into a login start.
    // The threat model tells applications to give OAuth2 its two routes and keep it out of chains.
    let h = Harness::new().await;
    let r = h.get("/chain-oauth-key", &[("x-api-key", API_KEY)]).await;
    assert_eq!(r.status, 302, "a valid key does not help: {}", r.text());
    assert!(r.header("location").unwrap().starts_with(&h.mock.url("/authorize")));
    assert_eq!(Probe::count(&h.probe.key), 0, "the API key validator did not run");
    assert_eq!(h.probe.runs("chain-oauth-key"), 0);
}

// ---- what an anonymous request costs ----

#[tokio::test]
async fn unauthenticated_cost_of_each_route_shape() {
    let h = Harness::new().await;
    let garbage = [
        vec![],
        vec![("authorization", "Bearer not-a-token".to_string())],
        vec![("authorization", bearer(&forged_token("1")))],
        vec![("authorization", bearer(&expired_token("1")))],
        vec![("cookie", "connect.sid=s%3Aunknown.unknown".to_string())],
        vec![("cookie", format!("jwt={}", forged_token("1")))],
    ];
    for route in ["jwt", "jwt-opt", "session", "chain-jwt-session", "default", "open", "roles", "skip-public", "skip-private"] {
        for headers in &garbage {
            let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            h.get(&format!("/{route}"), &borrowed).await;
        }
    }
    // A token or a cookie that does not verify costs no validator call, no session user lookup, no store access and no
    // provider request: the signature is checked first and the store is never asked for an unsigned identifier.
    assert_eq!(h.calls(), 0, "a validator or a lookup ran for a request without a valid credential");
    assert!(h.store.take_ops().is_empty(), "the session store was used");
    assert_eq!(h.mock.count(), 0);
    // The cases that do cost something, by design: the API key validator runs for any well-formed key (it decides), the
    // local strategy derives a key for any username and password (a dummy hash for an unknown user), and a login
    // started with the state in the session writes a record.
    h.get("/key", &[("x-api-key", "anything")]).await;
    assert_eq!(Probe::count(&h.probe.key), 1);
    h.post_json("/login", &[], json!({ "username": "nobody", "password": "x" })).await;
    assert_eq!(Probe::count(&h.probe.local), 1);
    h.get("/oauth-s", &[]).await;
    assert_eq!(h.store.len(), 1);
}

// ---- sockets ----

#[tokio::test]
async fn session_cookie_on_an_upgrade_request() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

    let mut h = Harness::new().await;
    let cookie = h.login("ana").await;
    assert_eq!(user_id(&h.get("/open", &[("cookie", &cookie)]).await), Some("1".into()), "the session is valid over HTTP");
    let calls = h.probe.strategy_calls();
    let addr = h.app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut upgrade = format!("ws://{addr}/ws").into_client_request().unwrap();
    upgrade.headers_mut().insert("cookie", cookie.parse().unwrap());
    upgrade.headers_mut().insert("authorization", bearer(&token("2")).parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(upgrade).await.unwrap();
    ws.send(Message::Text(json!({ "event": "who", "data": {} }).to_string())).await.unwrap();
    let frame = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.expect("a frame") {
            Some(Ok(Message::Text(t))) => break serde_json::from_str::<Value>(&t).unwrap(),
            Some(Ok(_)) => continue,
            other => panic!("no frame: {other:?}"),
        }
    };
    assert_eq!(frame["data"], json!({ "user": null, "session": false }), "{frame}");
    assert_eq!(h.probe.strategy_calls(), calls, "the upgrade ran no strategy and no session user lookup");
}

#[tokio::test]
async fn global_auth_guard_and_a_gateway() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    // An optional global guard changes nothing for HTTP routes that do not need a user, and it must not run a
    // strategy for a socket message, where it denies.
    let mut h = Harness::with(Config { global_guard: true, ..Config::default() }).await;
    let addr = h.app.listen_in_background("127.0.0.1:0").await.unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap();
    ws.send(Message::Text(json!({ "event": "who", "data": {} }).to_string())).await.unwrap();
    let frame = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await.expect("a frame") {
            Some(Ok(Message::Text(t))) => break serde_json::from_str::<Value>(&t).unwrap(),
            Some(Ok(_)) => continue,
            other => panic!("no frame: {other:?}"),
        }
    };
    assert_eq!(frame, json!({ "event": "exception", "data": { "status": "error", "message": "Forbidden resource" } }));
    assert_eq!(h.probe.runs("ws-who"), 0, "the handler did not run");
    assert_eq!(Probe::count(&h.probe.jwt), 0, "no strategy ran for the message");
    assert!(h.probe.jwt_raw.lock().unwrap().is_empty());
}

// ---- secrets never appear in output ----

#[tokio::test]
async fn debug_of_every_public_type() {
    use corvis::auth::api_key::{ApiKey, ApiKeySet, ExpectedApiKey};
    use corvis::auth::jwt::{JwtKey, JwtStrategyOptions};
    use corvis::auth::oauth2::{OAuth2Endpoints, OAuth2HttpOptions, OAuth2Options};
    use corvis::auth::session::SessionOptions;
    use corvis::auth::{AuthFailure, AuthResult, AuthenticatedUser, StrategyOutcome};
    use corvis::http::CookieKeys;
    use corvis::jwt::{JwtModuleOptions, JwtSignOptions};

    let marker = |n: &str| format!("CANARY-{n}-0123456789abcdef0123456789abcdef");
    let mut shown: Vec<(&str, String, String)> = Vec::new();
    let mut check = |name: &'static str, secret: String, text: String| shown.push((name, secret, text));

    let m = marker("jwt-key");
    check("JwtKey", m.clone(), format!("{:?}", JwtKey::secret(m)));
    let m = marker("jwt-options");
    check("JwtStrategyOptions", m.clone(), format!("{:?}", JwtStrategyOptions::new().secret_or_key(JwtKey::secret(m))));
    let m = marker("api-key");
    check("ApiKey", m.clone(), format!("{:?}", ApiKey::new(m)));
    let m = marker("expected-key");
    check("ExpectedApiKey", m.clone(), format!("{:?}", ExpectedApiKey::new(&m).unwrap()));
    let m = marker("key-set");
    let mut set = ApiKeySet::new();
    set.insert(ExpectedApiKey::new(&m).unwrap(), "service");
    check("ApiKeySet", m, format!("{set:?}"));
    let m = marker("cookie-keys");
    check("CookieKeys", m.clone(), format!("{:?}", CookieKeys::new([m]).unwrap()));
    let m = marker("session-secret");
    check("SessionOptions", m.clone(), format!("{:?}", SessionOptions::with_secrets([m]).unwrap()));
    let (secret, state) = (marker("client-secret"), marker("state-keys"));
    let options = OAuth2Options::new(
        OAuth2Endpoints::new("https://p.example/a", "https://p.example/t"),
        "client-id",
        secret.clone(),
        "https://app.example/cb",
    )
    .state_cookie(CookieKeys::new([state.clone()]).unwrap())
    .http(OAuth2HttpOptions::new());
    check("OAuth2Options (client secret)", secret, format!("{options:?}"));
    check("OAuth2Options (state keys)", state, format!("{options:?}"));
    let m = marker("module-secret");
    check("JwtModuleOptions (secret)", m.clone(), format!("{:?}", JwtModuleOptions::new(m)));
    let m = marker("module-private-key");
    check("JwtModuleOptions (private key)", m.clone(), format!("{:?}", JwtModuleOptions::new("").private_key(m)));
    let m = marker("sign-secret");
    check("JwtSignOptions", m.clone(), format!("{:?}", JwtSignOptions { secret: Some(m), ..Default::default() }));
    let m = marker("sign-private-key");
    check("JwtSignOptions (private key)", m.clone(), format!("{:?}", JwtSignOptions { private_key: Some(m), ..Default::default() }));
    let m = marker("user");
    let user = AuthenticatedUser::new(json!({ "password_hash": m })).unwrap();
    check("AuthenticatedUser", m.clone(), format!("{user:?}"));
    check("AuthResult", m.clone(), format!("{:?}", AuthResult::Success { user: user.clone(), info: Some(json!({ "k": m })) }));
    check("StrategyOutcome", m.clone(), format!("{:?}", StrategyOutcome::Success { user, info: Some(json!({ "k": m })) }));
    // Types a request builds: what a validator receives and what the strategies keep.
    let h = Harness::new().await;
    let l = oauth2_common::login(&h.app, "/oauth").await;
    let r = h.get(&format!("/oauth?code=code-1&state={}", l.state()), &[("cookie", l.cookie.as_deref().unwrap())]).await;
    assert_eq!(r.status, 200);
    for grant in h.probe.grants.lock().unwrap().iter() {
        for secret in ["ACCESS-TOKEN-FAKE", "REFRESH-TOKEN-FAKE", "ID-TOKEN-FAKE", "code-1"] {
            check("OAuth2Grant", secret.to_string(), grant.clone());
        }
    }
    let failure = AuthFailure { strategy: "x".into(), challenge: None, status: None };
    shown.push(("AuthFailure", "unused".into(), format!("{failure:?}")));

    for (name, secret, text) in &shown {
        assert!(!text.contains(secret.as_str()), "the Debug output of {name} contains its secret: {text}");
    }
    assert!(shown.len() >= 16);
}

// ---- OAuth2 state and session regeneration ----

/// Starts a login with the state in the session: the browser's cookie, the login redirect.
async fn start_session_login(h: &Harness) -> (oauth2_common::Login, String) {
    let login = oauth2_common::login(&h.app, "/oauth-s-login").await;
    let cookie = login.cookie.clone().expect("the login start creates a session");
    assert!(cookie.starts_with("connect.sid="), "{cookie}");
    (login, cookie)
}

fn callback_target(login: &oauth2_common::Login, code: &str) -> String {
    format!("/oauth-s-login?code={code}&state={}", login.state())
}

/// The keys of the session record that belong to the OAuth2 flow.
fn flow_keys(record: &Value) -> Vec<String> {
    fn walk(value: &Value, out: &mut Vec<String>) {
        if let Value::Object(map) = value {
            for (key, inner) in map {
                if key.to_ascii_lowercase().contains("oauth2") {
                    out.push(key.clone());
                }
                walk(inner, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(record, &mut out);
    out
}

#[tokio::test]
async fn callback_that_logs_in() {
    for keep in [false, true] {
        let h = Harness::with(Config { session: Box::new(move |o| o.keep_session_info_on_login(keep)), ..Config::default() }).await;
        let (login, cookie) = start_session_login(&h).await;
        let old_id = session_common::id_of(&cookie);
        let during = h.store.records().await;
        assert!(!flow_keys(&during[&old_id]).is_empty(), "the state is in the session record while the login is in flight: {during:?}");

        let r = h.get(&callback_target(&login, "code-1"), &[("cookie", &cookie)]).await;
        assert_eq!(r.status, 200, "keep={keep}: {}", r.text());
        let new_cookie = session_pair(&r).expect("the login issues a new session cookie");
        let new_id = session_common::id_of(&new_cookie);
        assert_ne!(new_id, old_id, "keep={keep}: the identifier changed at login");
        let records = h.store.records().await;
        assert!(!records.contains_key(&old_id), "keep={keep}: the old record is destroyed");
        let record = &records[&new_id];
        assert_eq!(record["passport"]["user"], "gh:110169484474386276334", "keep={keep}: {record}");
        assert!(flow_keys(record).is_empty(), "keep={keep}: no oauth2 key in the new session: {record}");
        assert!(!record.to_string().contains(&login.state()), "keep={keep}: the state value is not in the new session");
        // The new session works, the old cookie does not.
        assert_eq!(h.get("/session", &[("cookie", &new_cookie)]).await.status, 200);
        assert_eq!(h.get("/session", &[("cookie", &cookie)]).await.status, 401);
    }
}

#[tokio::test]
async fn callback_replayed_after_logout() {
    let h = Harness::new().await;
    let (login, cookie) = start_session_login(&h).await;
    let target = callback_target(&login, "code-1");
    let r = h.get(&target, &[("cookie", &cookie)]).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let logged_in = session_pair(&r).unwrap();
    assert_eq!(h.mock.hits_to("/token").len(), 1);
    // The user logs out; the browser keeps whatever the logout set.
    let out = h.post_json("/logout", &[("cookie", &logged_in)], json!({})).await;
    assert_eq!(out.status, 200);
    let after_logout = session_pair(&out).unwrap_or_else(|| logged_in.clone());
    for cookies in [cookie.clone(), logged_in.clone(), after_logout] {
        let r = h.get(&target, &[("cookie", &cookies)]).await;
        assert_eq!((r.status, r.json_value()), (401, unauthorized()), "cookies {cookies}");
    }
    let r = h.get(&target, &[]).await;
    assert_eq!(r.status, 401);
    assert_eq!(h.mock.hits_to("/token").len(), 1, "the provider received no further request");
    assert_eq!(Probe::count(&h.probe.oauth), 1);
}

#[tokio::test]
async fn callback_in_flight_twice_reaches_the_provider_once() {
    // The state record is removed before the provider is contacted: a second callback with the same state that arrives
    // while the first waits for the provider is refused without a request of its own.
    let gate = std::sync::Arc::new(tokio::sync::Notify::new());
    let g = gate.clone();
    let h = Harness::with(Config {
        provider: Box::new(move |r| {
            if r.path() == "/token" {
                oauth2_common::Reply::Wait(g.clone(), Box::new(oauth2_common::standard(r)))
            } else {
                oauth2_common::standard(r)
            }
        }),
        ..Config::default()
    })
    .await;
    let (login, cookie) = start_session_login(&h).await;
    let target = callback_target(&login, "code-1");
    let headers = [("cookie", cookie.as_str())];
    let first = h.get(&target, &headers);
    let second = async {
        h.mock.arrived.notified().await;
        let r = h.get(&target, &headers).await;
        gate.notify_one();
        r
    };
    let (first, second) = tokio::join!(first, second);
    assert_eq!(second.status, 401, "{}", second.text());
    assert_eq!(first.status, 200, "{}", first.text());
    assert_eq!(h.mock.hits_to("/token").len(), 1);
    assert_eq!(Probe::count(&h.probe.oauth), 1);
}

#[tokio::test]
async fn forged_callback_on_an_authenticated_session() {
    for keep in [false, true] {
        let h = Harness::with(Config { session: Box::new(move |o| o.keep_session_info_on_login(keep)), ..Config::default() }).await;
        let cookie = h.login("ana").await;
        // Attacker's `code` and `state`, as a link the victim's browser follows, to the session-state callback and to
        // the cookie-state one.
        for target in ["/oauth-s-login?code=ATTACKER&state=attacker-state", "/oauth?code=ATTACKER&state=attacker-state"] {
            let r = h.get(target, &[("cookie", &cookie)]).await;
            assert_eq!((r.status, r.json_value()), (401, unauthorized()), "keep={keep} {target}");
            assert!(session_pair(&r).is_none_or(|c| c == cookie), "keep={keep} {target}: the session cookie is not replaced");
        }
        assert_eq!(h.mock.count(), 0, "keep={keep}: the provider received no request");
        let me = h.get("/session", &[("cookie", &cookie)]).await;
        assert_eq!((me.status, user_id(&me)), (200, Some("1".into())), "keep={keep}: the session user is unchanged");
    }
}
