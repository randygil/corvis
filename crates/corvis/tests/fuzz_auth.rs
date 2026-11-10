//! Property tests of the authentication module with random requests (`auth/security-invariants`, "Invariants hold under
//! random input"): the gate compared with a reference model, the session state machine, the OAuth2 callback in front of a
//! counting provider, redirect targets and the canary secrets. They run in `cargo test` with a small case count, in the CI
//! `fuzz` job (`PROPTEST_CASES=20000`) and in the nightly run (`PROPTEST_CASES=100000`), like `src/fuzz_tests.rs`.
//!
//! Every property builds its application once and reuses it for all its cases, except the session state machine (a
//! paused clock per case). Nothing here measures time.
#![cfg(all(feature = "jwt", feature = "auth-local", feature = "auth-api-key", feature = "auth-session", feature = "auth-oauth2"))]

mod auth_common;
mod oauth2_common;
mod session_common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use auth_common::*;
use corvis::prelude::*;
use corvis::{LogLevel, LoggerService};
use proptest::prelude::*;

fn config() -> ProptestConfig {
    ProptestConfig { cases: cases(48), ..ProptestConfig::default() }
}

/// A runtime and an application that live for the whole process, so every case of a property reuses them.
struct Env {
    rt: tokio::runtime::Runtime,
    h: Harness,
    valid: Vec<String>,
    stale: Vec<String>,
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
}

// ---- the gate against a reference model ----

#[derive(Debug, Clone)]
enum Bearer {
    None,
    Valid(u8),
    Expired(u8),
    Forged(u8),
    /// A valid token behind more leading whitespace than the API key reader looks through.
    Padded(u8),
    ApiKey,
    WrongKey,
    Garbage(String),
}

#[derive(Debug, Clone)]
enum XKey {
    None,
    Valid,
    Wrong,
}

#[derive(Debug, Clone)]
enum JwtCookie {
    None,
    Valid(u8),
    Expired(u8),
    Garbage(String),
}

#[derive(Debug, Clone)]
enum Sess {
    None,
    Valid(u8),
    Tampered(u8),
    Stale(u8),
}

#[derive(Debug, Clone)]
struct Creds {
    bearer: Bearer,
    xkey: XKey,
    jwt_cookie: JwtCookie,
    sess: Sess,
}

/// The routes of the grid with the guard of each one.
const ROUTES: [&str; 13] = [
    "jwt",
    "jwt-opt",
    "key",
    "session",
    "chain-jwt-session",
    "chain-jwt-key",
    "chain-key-jwt",
    "chain-jwt-key-opt",
    "open",
    "roles",
    "skip-public",
    "skip-private",
    "default",
];

fn user_ok(u: u8) -> bool {
    (1..=4).contains(&u)
}

/// The model: the user a token strategy finds. The reader takes the `Authorization` token first and the cookie only
/// when there is no such header; an unusable token is not replaced by the next place.
fn model_jwt(c: &Creds) -> Option<u8> {
    match &c.bearer {
        Bearer::None => match &c.jwt_cookie {
            JwtCookie::Valid(u) if user_ok(*u) => Some(*u),
            _ => None,
        },
        Bearer::Valid(u) | Bearer::Padded(u) if user_ok(*u) => Some(*u),
        _ => None,
    }
}

/// The model: the user the API key strategy finds. Two sources with different keys fail the attempt.
fn model_key(c: &Creds) -> Option<u8> {
    let header = match c.xkey {
        XKey::None => None,
        XKey::Valid => Some("valid".to_string()),
        XKey::Wrong => Some("wrong".to_string()),
    };
    let bearer = match &c.bearer {
        Bearer::None | Bearer::Padded(_) => None,
        Bearer::ApiKey => Some("valid".to_string()),
        Bearer::WrongKey => Some("wrong".to_string()),
        // A token or garbage read as a key: never the key, but a source with a value of its own.
        other => Some(format!("token:{other:?}")),
    };
    let key = match (header, bearer) {
        (Some(a), Some(b)) if a != b => return None,
        (Some(a), _) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };
    (key == "valid").then_some(3)
}

fn model_session(c: &Creds) -> Option<u8> {
    match c.sess {
        Sess::Valid(u) if user_ok(u) => Some(u),
        _ => None,
    }
}

/// `(handler runs, the user it sees)` for a route.
fn model(route: &str, c: &Creds) -> (bool, Option<u8>) {
    let (jwt, key, session) = (model_jwt(c), model_key(c), model_session(c));
    let first = |options: &[Option<u8>]| options.iter().find_map(|o| *o);
    match route {
        "jwt" | "default" | "skip-private" => (jwt.is_some(), jwt),
        "jwt-opt" => (true, jwt),
        "key" => (key.is_some(), key),
        "session" => (session.is_some(), session),
        "chain-jwt-session" => {
            let user = first(&[jwt, session]);
            (user.is_some(), user)
        }
        "chain-jwt-key" => {
            let user = first(&[jwt, key]);
            (user.is_some(), user)
        }
        "chain-key-jwt" => {
            let user = first(&[key, jwt]);
            (user.is_some(), user)
        }
        "chain-jwt-key-opt" => (true, first(&[jwt, key])),
        // No guard, and a guard skipped on a public route: the session user, restored by the middleware, is the only
        // user there is.
        "open" | "skip-public" => (true, session),
        // A hand-written guard that trusts `Request::user()`: it admits the administrator's session.
        "roles" => (session == Some(1), session.filter(|u| *u == 1)),
        other => panic!("{other}"),
    }
}

fn arb_creds() -> impl Strategy<Value = Creds> {
    let user = 1u8..=5;
    let junk = "[A-Za-z0-9._~+/-]{1,24}";
    let bearer = prop_oneof![
        3 => Just(Bearer::None),
        3 => user.clone().prop_map(Bearer::Valid),
        1 => user.clone().prop_map(Bearer::Expired),
        1 => user.clone().prop_map(Bearer::Forged),
        1 => user.clone().prop_map(Bearer::Padded),
        1 => Just(Bearer::ApiKey),
        1 => Just(Bearer::WrongKey),
        1 => junk.prop_map(Bearer::Garbage),
    ];
    let xkey = prop_oneof![3 => Just(XKey::None), 2 => Just(XKey::Valid), 1 => Just(XKey::Wrong)];
    let jwt_cookie = prop_oneof![
        3 => Just(JwtCookie::None),
        2 => user.clone().prop_map(JwtCookie::Valid),
        1 => user.clone().prop_map(JwtCookie::Expired),
        1 => junk.prop_map(JwtCookie::Garbage),
    ];
    let sess = prop_oneof![
        3 => Just(Sess::None),
        3 => (1u8..=5).prop_map(Sess::Valid),
        1 => user.clone().prop_map(Sess::Tampered),
        1 => user.prop_map(Sess::Stale),
    ];
    (bearer, xkey, jwt_cookie, sess).prop_map(|(bearer, xkey, jwt_cookie, sess)| Creds { bearer, xkey, jwt_cookie, sess })
}

fn token_of(u: u8, offset: i64) -> String {
    token_with(&u.to_string(), JWT_SECRET, offset)
}

fn headers_of(env: &Env, c: &Creds) -> Vec<(&'static str, String)> {
    let mut headers = Vec::new();
    let authorization = match &c.bearer {
        Bearer::None => None,
        Bearer::Valid(u) => Some(bearer(&token_of(*u, 3600))),
        Bearer::Expired(u) => Some(bearer(&token_of(*u, -3600))),
        Bearer::Forged(u) => Some(bearer(&forged_token(&u.to_string()))),
        Bearer::Padded(u) => Some(format!("{}Bearer {}", " ".repeat(700), token_of(*u, 3600))),
        Bearer::ApiKey => Some(bearer(API_KEY)),
        Bearer::WrongKey => Some(bearer("wrong-key-value")),
        Bearer::Garbage(g) => Some(bearer(g)),
    };
    headers.extend(authorization.map(|v| ("authorization", v)));
    match c.xkey {
        XKey::None => {}
        XKey::Valid => headers.push(("x-api-key", API_KEY.to_string())),
        XKey::Wrong => headers.push(("x-api-key", "wrong-key-value".to_string())),
    }
    let mut cookies = Vec::new();
    match &c.jwt_cookie {
        JwtCookie::None => {}
        JwtCookie::Valid(u) => cookies.push(format!("jwt={}", token_of(*u, 3600))),
        JwtCookie::Expired(u) => cookies.push(format!("jwt={}", token_of(*u, -3600))),
        JwtCookie::Garbage(g) => cookies.push(format!("jwt={g}")),
    }
    let index = |u: u8| (u as usize).clamp(1, 4) - 1;
    match c.sess {
        Sess::None => {}
        // User 5 does not exist: its cookie is the one of user 4 with the user deleted, modeled as no session.
        Sess::Valid(u) if user_ok(u) => cookies.push(env.valid[index(u)].clone()),
        Sess::Valid(_) => cookies.push("connect.sid=s%3Anosuchsession.nosuchsignature".to_string()),
        Sess::Tampered(u) => cookies.push(format!("{}x", env.valid[index(u)])),
        Sess::Stale(u) => cookies.push(env.stale[index(u)].clone()),
    }
    if !cookies.is_empty() {
        headers.push(("cookie", cookies.join("; ")));
    }
    headers
}

fn gate_env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(|| {
        let rt = runtime();
        let h = rt.block_on(Harness::new());
        let (valid, stale) = rt.block_on(async {
            let mut valid = Vec::new();
            let mut stale = Vec::new();
            for name in ["ana", "bob", "cy", "dee"] {
                valid.push(h.login(name).await);
                let cookie = h.login(name).await;
                h.post_json("/logout", &[("cookie", &cookie)], json!({})).await;
                stale.push(cookie);
            }
            (valid, stale)
        });
        Env { rt, h, valid, stale }
    })
}

proptest! {
    #![proptest_config(config())]

    /// A handler runs, and with which user, exactly as the model says; no answer is a 5xx.
    #[test]
    fn gate_matches_model(route in 0..ROUTES.len(), creds in arb_creds()) {
        let env = gate_env();
        let route = ROUTES[route];
        let (runs, user) = model(route, &creds);
        let headers = headers_of(env, &creds);
        let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let r = env.rt.block_on(env.h.get(&format!("/{route}"), &borrowed));
        prop_assert!(r.status < 500, "{route} {creds:?}: {} {}", r.status, r.text());
        let expected_status = if runs { 200 } else if route == "roles" { 403 } else { 401 };
        prop_assert_eq!(r.status, expected_status, "{} {:?}: {}", route, creds, r.text());
        if runs {
            let seen = user_id(&r).and_then(|id| id.parse::<u8>().ok());
            prop_assert_eq!(seen, user, "{} {:?}", route, creds);
        } else {
            prop_assert!(!r.text().contains("\"route\""), "a handler ran");
        }
    }
}

// ---- the session state machine ----

#[derive(Debug, Clone)]
enum Op {
    Login(u8, bool),
    Logout,
    RequestCurrent,
    RequestDead(usize),
    Small,
    Huge,
}

fn arb_ops() -> impl Strategy<Value = Vec<Op>> {
    let op = prop_oneof![
        4 => (1u8..=4, any::<bool>()).prop_map(|(u, ok)| Op::Login(u, ok)),
        2 => Just(Op::Logout),
        4 => Just(Op::RequestCurrent),
        3 => (0usize..8).prop_map(Op::RequestDead),
        2 => Just(Op::Small),
        1 => Just(Op::Huge),
    ];
    prop::collection::vec(op, 1..14)
}

/// What the model knows about the browser.
#[derive(Default)]
struct Browser {
    cookie: Option<String>,
    /// The live login: its cookie, its user and when it was made.
    live: Option<(String, u8, u64)>,
    dead: Vec<String>,
}

const ABSOLUTE_SECONDS: u64 = 7 * 24 * 3600;

async fn run_session_ops(ops: Vec<Op>) -> Result<(), TestCaseError> {
    let h = Harness::new().await;
    let names = ["", "ana", "bob", "cy", "dee"];
    let mut b = Browser::default();
    let mut now = 0u64;
    let session = |cookie: &Option<String>| -> Vec<(&'static str, String)> { cookie.iter().map(|c| ("cookie", c.clone())).collect() };
    for op in ops {
        match op {
            Op::Login(user, ok) => {
                let password = if ok { PASSWORD } else { "wrong" };
                let headers = session(&b.cookie);
                let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let r = h.post_json("/login-session", &borrowed, json!({ "username": names[user as usize], "password": password })).await;
                if ok {
                    prop_assert_eq!(r.status, 200, "{}", r.text());
                    let new = session_pair(&r).expect("a login sets the cookie");
                    if let Some((old, ..)) = b.live.take() {
                        b.dead.push(old);
                    }
                    b.live = Some((new.clone(), user, now));
                    b.cookie = Some(new);
                } else {
                    prop_assert_eq!(r.status, 401);
                    prop_assert!(session_pair(&r).is_none(), "a failed login sets no session cookie");
                }
            }
            Op::Logout => {
                let headers = session(&b.cookie);
                let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let r = h.post_json("/logout", &borrowed, json!({})).await;
                if let Some((old, ..)) = b.live.take() {
                    b.dead.push(old);
                }
                if let Some(new) = session_pair(&r) {
                    b.cookie = Some(new);
                }
            }
            Op::RequestCurrent | Op::RequestDead(_) => {
                let cookie = match &op {
                    Op::RequestDead(i) if !b.dead.is_empty() => Some(b.dead[i % b.dead.len()].clone()),
                    _ => b.cookie.clone(),
                };
                let headers = session(&cookie);
                let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let r = h.get("/session", &borrowed).await;
                let expected = match (&cookie, &b.live) {
                    (Some(c), Some((live, user, _))) if c == live => Some(*user),
                    _ => None,
                };
                prop_assert_eq!(user_id(&r).and_then(|u| u.parse::<u8>().ok()), expected, "cookie {:?}", cookie);
                prop_assert_eq!(r.status, if expected.is_some() { 200 } else { 401 });
            }
            Op::Small => {
                tokio::time::advance(Duration::from_secs(60)).await;
                now += 60;
            }
            Op::Huge => {
                tokio::time::advance(Duration::from_secs(ABSOLUTE_SECONDS + 3600)).await;
                now += ABSOLUTE_SECONDS + 3600;
            }
        }
        // The absolute timeout ends every login older than it.
        if let Some((cookie, _, born)) = &b.live
            && now - born > ABSOLUTE_SECONDS
        {
            b.dead.push(cookie.clone());
            b.live = None;
        }
        // The store never holds a destroyed identifier, nor two logins for one browser.
        let records = h.store.records().await;
        for dead in &b.dead {
            prop_assert!(!records.contains_key(&session_common::id_of(dead)), "a destroyed identifier is still in the store");
        }
        let logged_in = records.values().filter(|r| r["passport"]["user"].is_string()).count();
        prop_assert!(logged_in <= 1, "{logged_in} records hold a login for one browser: {records:?}");
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(24), ..ProptestConfig::default() })]

    /// A destroyed or replaced identifier never authenticates again.
    #[test]
    fn session_state_machine(ops in arb_ops()) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().start_paused(true).build().unwrap();
        rt.block_on(run_session_ops(ops))?;
    }
}

// ---- the OAuth2 callback in front of a counting provider ----

#[derive(Debug, Clone)]
enum Mutation {
    Valid,
    Replayed,
    StateAppended,
    StateOfAnotherLogin,
    NoCookie,
    CookieOfAnotherLogin,
    EmptyCode,
    HugeCode,
    TwoStates,
    ProviderError,
    NoState,
}

#[derive(Debug, Clone)]
enum Callback {
    Real { session_state: bool, mutation: Mutation },
    Random { session_state: bool, query: String, cookie: Option<String> },
}

fn arb_callback() -> impl Strategy<Value = Callback> {
    let mutation = prop_oneof![
        3 => Just(Mutation::Valid),
        2 => Just(Mutation::Replayed),
        1 => Just(Mutation::StateAppended),
        1 => Just(Mutation::StateOfAnotherLogin),
        1 => Just(Mutation::NoCookie),
        1 => Just(Mutation::CookieOfAnotherLogin),
        1 => Just(Mutation::EmptyCode),
        1 => Just(Mutation::HugeCode),
        1 => Just(Mutation::TwoStates),
        1 => Just(Mutation::ProviderError),
        1 => Just(Mutation::NoState),
    ];
    let query = prop::collection::vec(("(code|state|error|code\\[\\]|state\\[a\\]|x)", "[A-Za-z0-9._~%-]{0,40}"), 0..5)
        .prop_map(|pairs| pairs.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"));
    let cookie = prop::option::of("[A-Za-z0-9._%=; -]{0,80}");
    prop_oneof![
        3 => (any::<bool>(), mutation).prop_map(|(session_state, mutation)| Callback::Real { session_state, mutation }),
        2 => (any::<bool>(), query, cookie).prop_map(|(session_state, query, cookie)| Callback::Random { session_state, query, cookie }),
    ]
}

fn oauth_env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(|| {
        let rt = runtime();
        let h = rt.block_on(Harness::new());
        Env { rt, h, valid: Vec::new(), stale: Vec::new() }
    })
}

async fn run_callback(h: &Harness, case: Callback) -> Result<(), TestCaseError> {
    let route = |session_state: bool| if session_state { "/oauth-s" } else { "/oauth" };
    let tokens = |h: &Harness| h.mock.hits_to("/token").len();
    match case {
        Callback::Random { session_state, query, cookie } => {
            let before = tokens(h);
            let headers: Vec<(&str, &str)> = cookie.iter().map(|c| ("cookie", c.as_str())).collect();
            let r = h.get(&format!("{}?{query}", route(session_state)), &headers).await;
            prop_assert!(r.status < 500, "{query} {cookie:?}: {} {}", r.status, r.text());
            prop_assert_eq!(tokens(h), before, "the provider was contacted for {} {:?}", query, cookie);
        }
        Callback::Real { session_state, mutation } => {
            let path = route(session_state);
            let login = oauth2_common::login(&h.app, path).await;
            let other = oauth2_common::login(&h.app, path).await;
            let cookie = login.cookie.clone().unwrap();
            let state = login.state();
            let (code, state, cookie, valid) = match mutation {
                Mutation::Valid | Mutation::Replayed => ("c1".to_string(), state, Some(cookie), true),
                Mutation::StateAppended => ("c1".into(), format!("{state}x"), Some(cookie), false),
                Mutation::StateOfAnotherLogin => ("c1".into(), other.state(), Some(cookie), false),
                Mutation::NoCookie => ("c1".into(), state, None, false),
                Mutation::CookieOfAnotherLogin => ("c1".into(), state, other.cookie.clone(), false),
                Mutation::EmptyCode => (String::new(), state, Some(cookie), false),
                Mutation::HugeCode => ("c".repeat(5000), state, Some(cookie), false),
                Mutation::TwoStates => ("c1".into(), format!("{state}&state=other"), Some(cookie), false),
                Mutation::ProviderError => ("c1".into(), state, Some(cookie), false),
                Mutation::NoState => ("c1".into(), String::new(), Some(cookie), false),
            };
            let query = match mutation {
                Mutation::ProviderError => format!("error=access_denied&state={state}"),
                Mutation::NoState => format!("code={code}"),
                _ => format!("code={code}&state={state}"),
            };
            let headers: Vec<(&str, &str)> = cookie.iter().map(|c| ("cookie", c.as_str())).collect();
            let before = tokens(h);
            let r = h.get(&format!("{path}?{query}"), &headers).await;
            prop_assert!(r.status < 500, "{mutation:?}: {} {}", r.status, r.text());
            prop_assert_eq!(tokens(h) - before, usize::from(valid), "{:?}: provider requests after a callback", mutation);
            prop_assert_eq!(r.status == 200, valid, "{:?}: {}", mutation, r.text());
            if matches!(mutation, Mutation::Replayed) {
                let again = h.get(&format!("{path}?{query}"), &headers).await;
                prop_assert_eq!(again.status, 401, "a state is used once");
                prop_assert_eq!(tokens(h) - before, 1, "the replay reached the provider");
            }
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    /// The provider is contacted only after a valid, unused state, and no answer is a 5xx caused by request data.
    #[test]
    fn oauth2_callback(case in arb_callback()) {
        let env = oauth_env();
        env.rt.block_on(run_callback(&env.h, case))?;
    }
}

// ---- redirect targets ----

fn redirect_env() -> &'static Env {
    static ENV: OnceLock<Env> = OnceLock::new();
    ENV.get_or_init(|| {
        let rt = runtime();
        let h = rt.block_on(Harness::with(Config {
            oauth: Box::new(|o| o.return_to_param("return_to").failure_redirect("/login-failed")),
            ..Config::default()
        }));
        Env { rt, h, valid: Vec::new(), stale: Vec::new() }
    })
}

fn arb_target() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[/\\\\a-z0-9.:%?&=#@ -]{0,40}",
        2 => prop::sample::select(vec![
            "/".to_string(),
            "/dashboard".to_string(),
            "//evil.example".to_string(),
            "/\\evil.example".to_string(),
            "https://evil.example/".to_string(),
            "/%2f/evil.example".to_string(),
            "/a%0d%0aSet-Cookie:x=1".to_string(),
            "/\u{e9}".to_string(),
            "javascript:alert(1)".to_string(),
            format!("/{}", "a".repeat(2000)),
        ]),
    ]
}

proptest! {
    #![proptest_config(config())]

    /// Every `Location` is the provider's authorization URL or the configured failure page, and a stored return path
    /// passes the documented rule.
    #[test]
    fn redirect_targets(target in arb_target(), fail in any::<bool>()) {
        let env = redirect_env();
        let h = &env.h;
        env.rt.block_on(async {
            let encoded = url::form_urlencoded::byte_serialize(target.as_bytes()).collect::<String>();
            let r = h.get(&format!("/oauth?return_to={encoded}"), &[]).await;
            prop_assert_eq!(r.status, 302, "{}", r.text());
            let location = r.header("location").unwrap().to_string();
            prop_assert!(location.starts_with(&h.mock.url("/authorize")), "{}", location);
            let cookie = oauth2_common::set_cookies(&r).first().map(|c| oauth2_common::pair(c)).unwrap();
            let state = url::Url::parse(&location).unwrap().query_pairs().find(|(k, _)| k == "state").unwrap().1.to_string();
            let state = if fail { format!("{state}x") } else { state };
            let r = h.get(&format!("/oauth?code=c1&state={state}"), &[("cookie", &cookie)]).await;
            prop_assert!(r.status < 500, "{}", r.text());
            if let Some(location) = r.header("location") {
                prop_assert!(location == "/login-failed" || location.starts_with(&h.mock.url("/authorize")), "{}", location);
            }
            if r.status == 200 {
                match r.json_value()["return_to"].as_str() {
                    None => {}
                    Some(path) => {
                        prop_assert!(corvis::auth::oauth2::is_safe_relative_path(path), "{path:?} stored for {target:?}");
                        prop_assert_eq!(path, target.as_str());
                    }
                }
            } else {
                prop_assert_eq!(r.status, 302, "a failed callback redirects to the configured page");
            }
            Ok(())
        })?;
    }
}

// ---- canary secrets ----

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

struct CanaryEnv {
    env: Env,
    logs: Captured,
}

fn canary_env() -> &'static CanaryEnv {
    static ENV: OnceLock<CanaryEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let rt = runtime();
        let logs: Captured = Arc::default();
        corvis::logger::set_logger(Capture(logs.clone()));
        corvis::logger::set_log_levels(&LogLevel::all());
        let h = rt.block_on(Harness::with(Config { logging: true, ..Config::default() }));
        let mut valid = Vec::new();
        rt.block_on(async {
            for name in ["ana", "bob"] {
                valid.push(h.login(name).await);
            }
        });
        CanaryEnv { env: Env { rt, h, valid, stale: Vec::new() }, logs }
    })
}

static MARKER: AtomicU64 = AtomicU64::new(1);

fn marker(kind: &str) -> String {
    format!("CANARY{:06}{kind}Z", MARKER.fetch_add(1, Ordering::SeqCst))
}

const CANARY_ROUTES: [&str; 10] = ["jwt", "key", "session", "chain-jwt-key", "default", "open", "oauth", "oauth-s", "nope", "login"];

proptest! {
    #![proptest_config(config())]

    /// No credential given to the module appears in a log line, a body or a header: unique markers in every place a
    /// strategy reads, next to the real secrets of the application, on valid and invalid requests.
    #[test]
    fn canary_secrets(route in 0..CANARY_ROUTES.len(), mask in 0u32..256, with_valid in any::<bool>()) {
        let c = canary_env();
        let route = CANARY_ROUTES[route];
        let mut canaries: Vec<String> = Vec::new();
        let mut headers: Vec<(&str, String)> = Vec::new();
        let mut cookies: Vec<String> = Vec::new();
        let mut query: Vec<String> = Vec::new();
        let mut body = json!({});
        let mut pick = |bit: u32, kind: &str| -> Option<String> {
            (mask & (1 << bit) != 0).then(|| {
                let m = marker(kind);
                canaries.push(m.clone());
                m
            })
        };
        if let Some(m) = pick(0, "bearer") {
            headers.push(("authorization", bearer(&m)));
        }
        if let Some(m) = pick(1, "key") {
            headers.push(("x-api-key", m));
        }
        if let Some(m) = pick(2, "jwtcookie") {
            cookies.push(format!("jwt={m}"));
        }
        if let Some(m) = pick(3, "session") {
            cookies.push(format!("connect.sid=s%3A{m}.{m}"));
        }
        if let Some(m) = pick(4, "code") {
            query.push(format!("code={m}"));
        }
        if let Some(m) = pick(5, "state") {
            query.push(format!("state={m}"));
        }
        if let Some(m) = pick(6, "user") {
            body["username"] = json!(m);
        }
        if let Some(m) = pick(7, "password") {
            body["password"] = json!(m);
        }
        // The secrets the application holds, and the valid credentials a client could hold.
        let mut secrets = vec![
            JWT_SECRET.to_string(),
            SESSION_SECRET.to_string(),
            oauth2_common::CLIENT_SECRET.to_string(),
            oauth2_common::STATE_SECRET.to_string(),
            "ACCESS-TOKEN-FAKE".to_string(),
            "REFRESH-TOKEN-FAKE".to_string(),
            "ID-TOKEN-FAKE".to_string(),
        ];
        let valid_token = token("1");
        if with_valid {
            headers.retain(|(k, _)| *k != "authorization");
            headers.push(("authorization", bearer(&valid_token)));
            secrets.push(valid_token.clone());
            cookies.push(c.env.valid[0].clone());
            secrets.push(session_common::id_of(&c.env.valid[0]));
            secrets.push(API_KEY.to_string());
            headers.push(("x-api-key", API_KEY.to_string()));
        }
        if !cookies.is_empty() {
            headers.push(("cookie", cookies.join("; ")));
        }
        let mut target = format!("/{route}");
        if !query.is_empty() {
            target.push('?');
            target.push_str(&query.join("&"));
        }
        let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let (status, text, response_headers) = c.env.rt.block_on(async {
            let r = if route == "login" {
                c.env.h.post_json(&target, &borrowed, body.clone()).await
            } else {
                c.env.h.get(&target, &borrowed).await
            };
            // Headers of the response: every one but a session cookie in its own `Set-Cookie`.
            let mut shown = String::new();
            for (name, value) in r.headers.iter() {
                if name == "set-cookie" && value.to_str().unwrap_or("").starts_with("connect.sid=") {
                    continue;
                }
                shown.push_str(&format!("{name}: {}\n", value.to_str().unwrap_or("")));
            }
            (r.status, r.text(), shown)
        });
        prop_assert!(status < 500 || route == "nope", "{route}: {status} {text}");
        let logs: Vec<String> = c.logs.lock().unwrap().iter().map(|(_, m, _)| m.clone()).collect();
        for secret in canaries.iter().chain(secrets.iter()) {
            prop_assert!(!text.contains(secret), "{secret} in the body of {target}: {text}");
            prop_assert!(!response_headers.contains(secret), "{secret} in the headers of {target}: {response_headers}");
            if let Some(line) = logs.iter().find(|l| l.contains(secret)) {
                prop_assert!(false, "{secret} in a log line for {target}: {line}");
            }
        }
        // Keep the capture small over a long run.
        let mut guard = c.logs.lock().unwrap();
        if guard.len() > 5000 {
            guard.clear();
        }
    }
}
