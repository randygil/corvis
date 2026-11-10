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
use corvis::testing::request;

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
