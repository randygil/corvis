//! The login routes have their own limits, per client and per account, and the limit is checked before the password is.

mod common;

use common::*;
use corvis::prelude::*;

#[corvis::test]
async fn attempts_over_the_account_limit_are_answered_429() {
    let h = Harness::start().await;
    let c = h.client();
    let attempt = |user: &str| c.post("/auth/login").json(&json!({ "username": user, "password": "wrong" }));
    for i in 1..=5 {
        let r = attempt("john").send().await;
        assert_eq!(r.status, 401, "attempt {i}");
    }
    let limited = attempt("john").send().await;
    assert_eq!(limited.status, 429, "{}", limited.text());
    // The account limit is the named throttler `login`, so its header carries the name.
    assert!(limited.header("retry-after-login").is_some(), "{:?}", limited.headers);

    // Even the right password is refused while the account is over its limit, and nothing was hashed for it.
    let right = c.post("/auth/login").json(&json!({ "username": "john", "password": "john-demo-pass" })).send().await;
    assert_eq!(right.status, 429);

    // Another account is counted on its own (the client is under its own limit).
    assert_eq!(attempt("maria").send().await.status, 401);
}

#[corvis::test]
async fn the_account_limit_is_shared_by_both_login_routes() {
    let h = Harness::start().await;
    let c = h.client();
    // Five wrong passwords for one account, alternating the two routes: the sixth attempt is refused on either.
    for i in 0..5 {
        let route = if i % 2 == 0 { "/auth/login" } else { "/session/login" };
        let r = c.post(route).json(&json!({ "username": "john", "password": "wrong" })).send().await;
        assert_eq!(r.status, 401, "attempt {i} on {route}");
    }
    for route in ["/auth/login", "/session/login"] {
        let r = c.post(route).json(&json!({ "username": "john", "password": "john-demo-pass" })).send().await;
        assert_eq!(r.status, 429, "{route}");
    }
}

#[corvis::test]
async fn the_login_limit_applies_to_the_login_routes_only() {
    let h = Harness::start().await;
    let c = h.client();
    let token = token_of(&c, "john", "john-demo-pass").await;
    let cookie = session_of(&c, "maria", "maria-demo-pass").await;
    let calls = [
        c.get("/").send().await,
        c.get("/feed").send().await,
        c.get("/me").bearer(&token).send().await,
        c.get("/reports").bearer(&token).send().await,
        c.get("/admin/users").bearer(&token).send().await,
        c.get("/session/me").header("cookie", &cookie).send().await,
        c.get("/auth/provider").send().await,
        c.get("/auth/provider/callback?code=x&state=y").send().await,
        c.post("/session/logout").header("cookie", &cookie).send().await,
    ];
    for (i, r) in calls.iter().enumerate() {
        assert!(r.headers.get("x-ratelimit-limit-login").is_none(), "route {i} is counted by the login throttler");
        assert!(r.headers.get("x-ratelimit-limit").is_some(), "route {i} keeps the global limit");
    }
    // The login routes are counted by it.
    let login = c.post("/auth/login").json(&json!({ "username": "x", "password": "y" })).send().await;
    assert!(login.headers.get("x-ratelimit-limit-login").is_some());
}

#[corvis::test]
async fn the_session_login_is_limited_too() {
    let h = Harness::start().await;
    let c = h.client();
    let attempt = || c.post("/session/login").json(&json!({ "username": "maria", "password": "wrong" }));
    for _ in 0..5 {
        assert_eq!(attempt().send().await.status, 401);
    }
    let limited = attempt().send().await;
    assert_eq!(limited.status, 429);
    assert!(limited.headers.get("set-cookie").is_none());
}

#[corvis::test]
async fn the_client_limit_covers_many_accounts_from_one_address() {
    let h = Harness::start().await;
    let c = h.client();
    // Ten attempts a minute per client, each for a different account.
    for i in 0..10 {
        let r = c.post("/auth/login").json(&json!({ "username": format!("user{i}"), "password": "wrong" })).send().await;
        assert_eq!(r.status, 401, "attempt {i}");
    }
    let r = c.post("/auth/login").json(&json!({ "username": "user10", "password": "wrong" })).send().await;
    assert_eq!(r.status, 429);
    assert!(r.header("retry-after").is_some(), "the client limit is the `default` throttler");
}

#[corvis::test]
async fn other_routes_keep_the_global_limit() {
    let h = Harness::start().await;
    let c = h.client();
    for _ in 0..6 {
        c.post("/auth/login").json(&json!({ "username": "john", "password": "wrong" })).send().await;
    }
    let r = c.get("/me").send().await;
    assert_eq!(r.status, 401);
    r.expect_header("x-ratelimit-limit", "100");
}

#[corvis::test]
async fn the_provider_routes_are_limited_per_client() {
    let h = Harness::start().await;
    let c = h.client();
    for i in 1..=10 {
        assert_eq!(c.get("/auth/provider").send().await.status, 302, "start {i}");
    }
    assert_eq!(c.get("/auth/provider").send().await.status, 429);
    // The callback has its own counter: a flood of starts does not block it, and a flood of callbacks is limited.
    for i in 1..=10 {
        assert_eq!(c.get("/auth/provider/callback?code=x&state=y").send().await.status, 401, "callback {i}");
    }
    assert_eq!(c.get("/auth/provider/callback?code=x&state=y").send().await.status, 429);
}
