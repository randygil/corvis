//! The scenarios of `auth/login-rate-limiting`: login and callback routes limited with the existing `ThrottlerGuard`,
//! per client and per account (`corvis::auth::local::login_tracker`), and what a throttled request must not cost. The
//! application is the one of `tests/auth_common/`: `ThrottlerGuard` is a global guard, so it runs before every
//! `AuthGuard`. The counters are the validator and provider call counts, never timing. The startup warning for an
//! `AuthGuard` that runs before the throttler is in `auth_guard_order_log.rs` (the logger is process-wide).
#![cfg(all(feature = "jwt", feature = "auth-local", feature = "auth-api-key", feature = "auth-session", feature = "auth-oauth2"))]

mod auth_common;
mod oauth2_common;
mod session_common;

use std::sync::{Arc, Mutex};

use auth_common::*;
use corvis::auth::local::LocalStrategyOptions;
use corvis::prelude::*;
use sha2::{Digest, Sha256};

fn acct(username: &str) -> String {
    let digest = Sha256::digest(username.as_bytes());
    format!("acct:{}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

async fn app(local: LocalStrategyOptions, trust_proxy: bool) -> Harness {
    Harness::with(Config { throttle: Some(recipe_storage(local.clone())), local, trust_proxy, ..Config::default() }).await
}

async fn attempt(h: &Harness, from: &str, username: &str, password: &str) -> corvis::testing::TestResponse {
    let headers: Vec<(&str, &str)> = if from.is_empty() { vec![] } else { vec![("x-forwarded-for", from)] };
    h.post_json("/login", &headers, json!({ "username": username, "password": password })).await
}

// ---- a throttled request runs no authentication ----

#[tokio::test]
async fn password_guessing_over_the_limit() {
    let h = app(LocalStrategyOptions::default(), false).await;
    for i in 1..=5 {
        let r = attempt(&h, "", "ana", "wrong").await;
        assert_eq!(r.status, 401, "attempt {i}");
        assert_eq!(r.header("x-ratelimit-limit-login"), Some("5"));
    }
    assert_eq!(Probe::count(&h.probe.local), 5);
    let r = attempt(&h, "", "ana", "wrong").await;
    assert_eq!(r.status, 429, "{}", r.text());
    assert!(r.header("retry-after-login").is_some(), "{:?}", r.headers);
    assert_eq!(Probe::count(&h.probe.local), 5, "the key derivation counter did not increase");
    assert_eq!(h.calls(), 5, "no strategy ran for the throttled request");
    // The right password is refused as well: the limit does not depend on the outcome.
    assert_eq!(attempt(&h, "", "ana", PASSWORD).await.status, 429);
    assert_eq!(Probe::count(&h.probe.local), 5);
}

#[tokio::test]
async fn password_guessing_by_one_client_over_the_client_limit() {
    let h = app(LocalStrategyOptions::default(), true).await;
    // Ten different accounts from one address: the per-client throttler of the recipe stops it, the per-account one
    // never sees a second attempt for any of them.
    for i in 1..=10 {
        let r = attempt(&h, "203.0.113.9", &format!("user{i}"), "wrong").await;
        assert_eq!(r.status, 401, "attempt {i}");
    }
    let r = attempt(&h, "203.0.113.9", "user11", "wrong").await;
    assert_eq!(r.status, 429);
    assert!(r.header("retry-after").is_some(), "{:?}", r.headers);
    assert_eq!(Probe::count(&h.probe.local), 10);
}

#[tokio::test]
async fn oauth2_login_start_over_the_limit() {
    let h = app(LocalStrategyOptions::default(), false).await;
    for i in 1..=5 {
        let r = h.get("/oauth-s", &[]).await;
        assert_eq!(r.status, 302, "start {i}");
    }
    let records = h.store.len();
    assert_eq!(records, 5, "every login start of a browser without a session creates a record");
    let r = h.get("/oauth-s", &[]).await;
    assert_eq!(r.status, 429, "{}", r.text());
    assert!(r.header("location").is_none(), "no redirect is sent");
    assert!(set_cookies(&r).is_empty());
    assert_eq!(h.store.len(), records, "the session store has no new record");
    assert_eq!(h.mock.count(), 0);
}

#[tokio::test]
async fn callback_over_the_limit() {
    let h = app(LocalStrategyOptions::default(), false).await;
    let login = oauth2_common::login(&h.app, "/oauth").await;
    let cookie = login.cookie.clone().unwrap();
    for i in 2..=5 {
        let r = h.get("/oauth?code=c&state=forged", &[("cookie", &cookie)]).await;
        assert_eq!(r.status, 401, "request {i}");
    }
    // The genuine callback is the sixth request of the client to the route.
    let r = h.get(&format!("/oauth?code=c&state={}", login.state()), &[("cookie", &cookie)]).await;
    assert_eq!(r.status, 429, "{}", r.text());
    assert_eq!(h.mock.count(), 0, "the provider received no request");
    assert_eq!(Probe::count(&h.probe.oauth), 0);
}

// ---- login attempts can be counted per account ----

#[tokio::test]
async fn same_account_from_many_addresses() {
    let h = app(LocalStrategyOptions::default(), true).await;
    for i in 1..=5 {
        let r = attempt(&h, &format!("198.51.100.{i}"), "ana", "wrong").await;
        assert_eq!(r.status, 401, "attempt {i}");
    }
    let r = attempt(&h, "198.51.100.6", "ana", "wrong").await;
    assert_eq!(r.status, 429, "{}", r.text());
    assert_eq!(Probe::count(&h.probe.local), 5);
    // Another account from the same new address is not affected.
    assert_eq!(attempt(&h, "198.51.100.6", "bob", "wrong").await.status, 401);
}

#[tokio::test]
async fn username_moved_to_the_query_string() {
    let h = app(LocalStrategyOptions::default().allow_query_credentials(), true).await;
    for i in 1..=5 {
        let r = if i % 2 == 1 {
            attempt(&h, &format!("198.51.100.{i}"), "ana", "wrong").await
        } else {
            let mut call = corvis::testing::request(&h.app).post("/login?username=ana&password=wrong");
            call = call.header("x-forwarded-for", &format!("198.51.100.{i}"));
            call.send().await
        };
        assert_eq!(r.status, 401, "attempt {i}: {}", r.text());
    }
    assert_eq!(Probe::count(&h.probe.local), 5, "the query attempts reached the validator too");
    let r =
        corvis::testing::request(&h.app).post("/login?username=ana&password=wrong").header("x-forwarded-for", "198.51.100.77").send().await;
    assert_eq!(r.status, 429, "the same account on one counter: {}", r.text());
    // The body wins over the query, as it does for the strategy: this one is counted for bob.
    let r = corvis::testing::request(&h.app)
        .post("/login?username=ana&password=wrong")
        .header("content-type", "application/json")
        .header("x-forwarded-for", "198.51.100.78")
        .body(json!({ "username": "bob", "password": "wrong" }).to_string())
        .send()
        .await;
    assert_eq!(r.status, 401, "{}", r.text());
}

#[tokio::test]
async fn nested_field_name() {
    let h = app(LocalStrategyOptions::default().username_field("user[email]"), true).await;
    let nested = |from: String, name: &'static str| {
        let h = &h;
        async move { h.post_json("/login", &[("x-forwarded-for", &from)], json!({ "user": { "email": name }, "password": "wrong" })).await }
    };
    for i in 1..=5 {
        assert_eq!(nested(format!("198.51.100.{i}"), "ana").await.status, 401, "attempt {i}");
    }
    assert_eq!(nested("198.51.100.6".into(), "ana").await.status, 429);
    // The default field is not what this strategy reads: it is not the account, so it is counted for the client.
    let r = h.post_json("/login", &[("x-forwarded-for", "198.51.100.7")], json!({ "username": "ana", "password": "wrong" })).await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn missing_username() {
    let h = app(LocalStrategyOptions::default(), true).await;
    for i in 1..=5 {
        let r = h.post_json("/login", &[("x-forwarded-for", "198.51.100.1")], json!({ "password": "wrong" })).await;
        assert_eq!((r.status, r.json_value()), (401, unauthorized()), "attempt {i}: answered as missing credentials");
    }
    let r = h.post_json("/login", &[("x-forwarded-for", "198.51.100.1")], json!({ "password": "wrong" })).await;
    assert_eq!(r.status, 429, "counted for the client address");
    let other = h.post_json("/login", &[("x-forwarded-for", "198.51.100.2")], json!({ "password": "wrong" })).await;
    assert_eq!(other.status, 401, "another client has its own counter");
    assert_eq!(Probe::count(&h.probe.local), 0, "the validator never ran without a username");
}

#[tokio::test]
async fn existing_and_unknown_accounts() {
    let h = app(LocalStrategyOptions::default(), true).await;
    let mut series = Vec::new();
    for (who, from) in [("ana", "198.51.100.1"), ("ghost", "198.51.100.2")] {
        let mut headers = Vec::new();
        for _ in 1..=5 {
            let r = attempt(&h, from, who, "wrong").await;
            assert_eq!(r.status, 401);
            headers.push((
                r.header("x-ratelimit-limit-login").map(str::to_string),
                r.header("x-ratelimit-remaining-login").map(str::to_string),
                r.header("x-ratelimit-limit").map(str::to_string),
                r.header("x-ratelimit-remaining").map(str::to_string),
            ));
        }
        let r = attempt(&h, from, who, "wrong").await;
        assert_eq!(r.status, 429);
        headers.push((r.header("retry-after-login").map(|_| "set".to_string()), None, None, None));
        series.push(headers);
    }
    assert_eq!(series[0], series[1], "the rate limit headers do not tell an existing account from an unknown one");
}

#[tokio::test]
async fn storage_keys() {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = seen.clone();
    let storage = recipe_storage(LocalStrategyOptions::default()).with_key_generator_for("login", move |_ctx, tracker, _name| {
        sink.lock().unwrap().push(tracker.to_string());
        format!("login:{tracker}")
    });
    let h = Harness::with(Config { throttle: Some(storage), ..Config::default() }).await;
    let long = "x".repeat(2000);
    let names = ["ana", "O'Brien <script>alert(1)</script>", "ghost@example.com", long.as_str()];
    for name in names {
        attempt(&h, "", name, "wrong").await;
    }
    h.post_json("/login", &[], json!({ "password": "wrong" })).await;
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 5);
    for tracker in &seen {
        for name in names.iter().filter(|n| n.len() > 3) {
            assert!(!tracker.contains(name), "a username reached the throttler: {tracker}");
        }
        assert!(tracker.len() <= 69, "the key is bounded: {} bytes", tracker.len());
        assert!(tracker == &acct("ana") || tracker == "127.0.0.1" || tracker.starts_with("acct:"), "{tracker}");
    }
    assert_eq!(seen[0], acct("ana"));
    assert_eq!(seen[3], "127.0.0.1", "a username over the strategy's limit is counted for the client address");
    assert_eq!(seen[4], "127.0.0.1", "so is a request without one");
    assert!(seen[1].starts_with("acct:") && seen[1].len() == 69);
}

// ---- the tracker reads what the strategy reads ----

mod tracker_matches_strategy {
    use corvis::auth::AuthGuard;
    use corvis::auth::local::{LocalStrategy, LocalStrategyOptions, LocalValidator, login_tracker};
    use corvis::prelude::*;
    use corvis::testing::request;
    use serde::Serialize;

    use super::acct;

    const FIXTURE: &str = include_str!("fixtures/auth/local.json");

    #[derive(Debug, Clone, Serialize)]
    pub struct Echo {
        username: String,
    }

    macro_rules! validator {
        ($name:ident, $strategy:literal, $options:expr) => {
            #[injectable]
            pub struct $name;

            #[async_trait]
            impl LocalValidator for $name {
                const NAME: &'static str = $strategy;
                type User = Echo;
                fn options(&self) -> LocalStrategyOptions {
                    $options
                }
                async fn validate(&self, _req: &Request, username: &str, _password: &str) -> Result<Option<Echo>, HttpException> {
                    Ok(Some(Echo { username: username.to_string() }))
                }
            }
        };
    }

    /// The options of each route of the fixture, as `tests/auth_local.rs` configures them.
    pub fn options_of(route: &str) -> LocalStrategyOptions {
        match route {
            "/login" => LocalStrategyOptions::default(),
            "/nested" => LocalStrategyOptions::default().username_field("user[email]"),
            "/userobj" => LocalStrategyOptions::default().username_field("user"),
            "/indexed" => LocalStrategyOptions::default().username_field("names[1]"),
            "/query" => LocalStrategyOptions::default().allow_query_credentials(),
            "/raised" => LocalStrategyOptions::default().max_password_bytes(4096),
            "/renamed" => LocalStrategyOptions::default().username_field("email").password_field("pass"),
            other => panic!("no strategy for {other}"),
        }
    }

    validator!(Plain, "plain", options_of("/login"));
    validator!(Nested, "nested", options_of("/nested"));
    validator!(UserObject, "userobj", options_of("/userobj"));
    validator!(Indexed, "indexed", options_of("/indexed"));
    validator!(Querying, "querying", options_of("/query"));
    validator!(Raised, "raised", options_of("/raised"));
    validator!(Renamed, "renamed", options_of("/renamed"));

    #[derive(Clone)]
    struct Tracked(String);

    /// Computes the tracker value for the request, as the throttler would, and leaves it for the handler.
    pub struct TrackGuard(pub LocalStrategyOptions);

    #[async_trait]
    impl corvis::CanActivate for TrackGuard {
        async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
            let value = login_tracker(self.0.clone())(ctx.clone()).await?;
            ctx.request().insert(Tracked(value));
            Ok(true)
        }
    }

    #[controller("")]
    pub struct Routes;

    #[routes]
    impl Routes {
        #[post("login")]
        #[use_guards(AuthGuard::new("plain"))]
        fn login(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[post("query")]
        #[use_guards(AuthGuard::new("querying"))]
        fn query(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[post("nested")]
        #[use_guards(AuthGuard::new("nested"))]
        fn nested(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[post("userobj")]
        #[use_guards(AuthGuard::new("userobj"))]
        fn userobj(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[post("indexed")]
        #[use_guards(AuthGuard::new("indexed"))]
        fn indexed(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[post("raised")]
        #[use_guards(AuthGuard::new("raised"))]
        fn raised(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[post("renamed")]
        #[use_guards(AuthGuard::new("renamed"))]
        fn renamed(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[post("track/login")]
        #[use_guards(TrackGuard(options_of("/login")))]
        fn track_login(&self, req: Request) -> Value {
            json!({ "tracker": req.get::<Tracked>().map(|t| t.0) })
        }

        #[post("track/query")]
        #[use_guards(TrackGuard(options_of("/query")))]
        fn track_query(&self, req: Request) -> Value {
            json!({ "tracker": req.get::<Tracked>().map(|t| t.0) })
        }

        #[post("track/nested")]
        #[use_guards(TrackGuard(options_of("/nested")))]
        fn track_nested(&self, req: Request) -> Value {
            json!({ "tracker": req.get::<Tracked>().map(|t| t.0) })
        }

        #[post("track/userobj")]
        #[use_guards(TrackGuard(options_of("/userobj")))]
        fn track_userobj(&self, req: Request) -> Value {
            json!({ "tracker": req.get::<Tracked>().map(|t| t.0) })
        }

        #[post("track/indexed")]
        #[use_guards(TrackGuard(options_of("/indexed")))]
        fn track_indexed(&self, req: Request) -> Value {
            json!({ "tracker": req.get::<Tracked>().map(|t| t.0) })
        }

        #[post("track/raised")]
        #[use_guards(TrackGuard(options_of("/raised")))]
        fn track_raised(&self, req: Request) -> Value {
            json!({ "tracker": req.get::<Tracked>().map(|t| t.0) })
        }

        #[post("track/renamed")]
        #[use_guards(TrackGuard(options_of("/renamed")))]
        fn track_renamed(&self, req: Request) -> Value {
            json!({ "tracker": req.get::<Tracked>().map(|t| t.0) })
        }

        /// Same options as the strategy of `/login`, to count the oversized username.
        #[post("track")]
        #[use_guards(TrackGuard(options_of("/login")))]
        fn track(&self, req: Request) -> Value {
            json!({ "tracker": req.get::<Tracked>().map(|t| t.0) })
        }
    }

    #[module(
        providers = [
            Provider::strategy::<LocalStrategy<Plain>>(),
            Provider::strategy::<LocalStrategy<Nested>>(),
            Provider::strategy::<LocalStrategy<UserObject>>(),
            Provider::strategy::<LocalStrategy<Indexed>>(),
            Provider::strategy::<LocalStrategy<Querying>>(),
            Provider::strategy::<LocalStrategy<Raised>>(),
            Provider::strategy::<LocalStrategy<Renamed>>(),
        ],
        controllers = [Routes],
    )]
    pub struct AppModule;

    async fn post(app: &CorvisApplication, path: &str, case: &Value) -> corvis::testing::TestResponse {
        let req = &case["request"];
        let mut call = request(app).post(path);
        for (k, v) in req["headers"].as_object().unwrap() {
            call = call.header(k, v.as_str().unwrap());
        }
        if let Some(body) = req["body"].as_str() {
            call = call.body(body.to_string());
        }
        call.send().await
    }

    #[corvis::test]
    async fn the_tracker_and_the_strategy_get_the_same_username_from_every_recorded_case() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
        app.silence_route_logs();
        app.init().await.unwrap();
        let (mut matched, mut unreadable) = (0, 0);
        for case in fixture["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let target = case["request"]["path"].as_str().unwrap();
            let (route, query) = target.split_once('?').map(|(r, q)| (r, format!("?{q}"))).unwrap_or((target, String::new()));
            // The route of the case, and the same request against the strategy that reads the query string.
            let mut routes = vec![route];
            if route == "/login" {
                routes.push("/query");
            }
            for login in routes {
                let track = format!("/track{login}");
                let strategy = post(&app, &format!("{login}{query}"), case).await;
                let tracker = post(&app, &format!("{track}{query}"), case).await.json_value()["tracker"].as_str().unwrap().to_string();
                match strategy.status {
                    201 => {
                        let username = strategy.json_value()["user"]["username"].as_str().unwrap().to_string();
                        assert_eq!(tracker, acct(&username), "{name} ({login}): the tracker names the account the strategy used");
                        matched += 1;
                    }
                    _ => {
                        // The strategy refused (a missing or oversized credential): when the username was readable but
                        // the password was not, the account is still the tracker's; otherwise it is the client.
                        assert!(tracker == "127.0.0.1" || tracker.starts_with("acct:"), "{name} ({login}): {tracker}");
                        unreadable += 1;
                    }
                }
            }
        }
        assert!(matched >= 20 && unreadable >= 20, "{matched} matched, {unreadable} refused");
    }

    #[corvis::test]
    async fn an_oversized_username_is_counted_for_the_client() {
        let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
        app.silence_route_logs();
        app.init().await.unwrap();
        let body = json!({ "username": "u".repeat(1025), "password": "p" }).to_string();
        let r = request(&app).post("/track").header("content-type", "application/json").body(body).send().await;
        assert_eq!(r.json_value()["tracker"], "127.0.0.1");
        let body = json!({ "username": "u".repeat(1024), "password": "p" }).to_string();
        let r = request(&app).post("/track").header("content-type", "application/json").body(body).send().await;
        assert_eq!(r.json_value()["tracker"], acct(&"u".repeat(1024)));
    }
}
