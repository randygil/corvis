//! The defaults table of `docs/security/authentication-threat-model.md`: the block of `key = value` lines under the
//! table is read and compared, row by row, with what the code reports (constants, `Debug` of the default options, and
//! behavior where nothing else shows a default). A default that changes in the code and not in the page, or the other way
//! round, fails here and names the row. Also the three scenarios of "Defaults are the same rule in every strategy":
//! secret minimums, cookies set by the module, credentials in a URL.
#![cfg(all(feature = "jwt", feature = "auth-local", feature = "auth-api-key", feature = "auth-session", feature = "auth-oauth2"))]

mod auth_common;
mod oauth2_common;
mod session_common;

use std::collections::BTreeMap;
use std::time::Duration;

use auth_common::*;
use corvis::auth::api_key::{self, ApiKeyStrategyOptions, ExpectedApiKey};
use corvis::auth::jwt::{self, JwtStrategyOptions};
use corvis::auth::local::{
    Argon2Params, DEFAULT_ACQUIRE_TIMEOUT, DEFAULT_MAX_CREDENTIAL_BYTES, LocalStrategyOptions, MIN_BCRYPT_COST, VerifyLimits,
};
use corvis::auth::oauth2::{
    DEFAULT_MAX_CODE_BYTES, DEFAULT_REDEEMED_STATES_MAX, DEFAULT_STATE_TTL, OAuth2Endpoints, OAuth2HttpOptions, OAuth2Options,
};
use corvis::auth::session::{
    DEFAULT_ABSOLUTE_TIMEOUT, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_SESSION_BYTES, DEFAULT_STORE_TIMEOUT, SessionCookie, SessionOptions,
};
use corvis::http::CookieKeys;
use corvis::http::signed_cookie;
use corvis::prelude::*;

const PAGE: &str = include_str!("../../../docs/security/authentication-threat-model.md");

/// The `key = value` lines of the first fenced block after the "Defaults" heading.
fn documented() -> BTreeMap<String, String> {
    let section = PAGE.split("\n## Defaults").nth(1).expect("the page has a Defaults section");
    let block = section.split("```ini\n").nth(1).expect("the Defaults section has an ini block").split("```").next().unwrap();
    let mut rows = BTreeMap::new();
    for line in block.lines().filter(|l| !l.trim().is_empty()) {
        let (key, value) = line.split_once(" = ").unwrap_or_else(|| panic!("not a `key = value` line: {line:?}"));
        assert!(rows.insert(key.trim().to_string(), value.trim().to_string()).is_none(), "{key} appears twice");
    }
    rows
}

/// The text after `field: ` in a `Debug` string, up to the next `,` or ` }`.
fn field(debug: &str, name: &str) -> String {
    // A whole field name: `timeout` is not the end of `connect_timeout`.
    let start = debug
        .match_indices(&format!("{name}: "))
        .find(|(at, _)| *at == 0 || debug[..*at].ends_with([' ', '{']))
        .unwrap_or_else(|| panic!("{name} is not in {debug}"))
        .0
        + name.len()
        + 2;
    let rest = &debug[start..];
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    rest[..end].trim().to_string()
}

/// Seconds of a `Duration` `Debug` form: `0ns`, `5s`, `10s`.
fn seconds(text: &str) -> String {
    match text {
        "0ns" => "0".to_string(),
        other => other.strip_suffix('s').unwrap_or_else(|| panic!("{other}")).to_string(),
    }
}

fn secs(d: Duration) -> String {
    d.as_secs().to_string()
}

/// The attributes of a `Set-Cookie` header without its name, value, lifetime and expiry, in the order of the page.
fn attributes(set_cookie: &str) -> String {
    let parts: Vec<&str> =
        set_cookie.split(';').skip(1).map(str::trim).filter(|a| !a.starts_with("Max-Age") && !a.starts_with("Expires")).collect();
    let mut ordered = Vec::new();
    for wanted in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
        if parts.contains(&wanted) {
            ordered.push(wanted);
        }
    }
    if ordered.len() == parts.len() { ordered.join("; ") } else { format!("unexpected attributes: {parts:?}") }
}

async fn actual() -> BTreeMap<String, String> {
    let mut rows: BTreeMap<String, String> = BTreeMap::new();
    let mut put = |key: &str, value: String| {
        rows.insert(key.to_string(), value);
    };

    let jwt_options = format!("{:?}", JwtStrategyOptions::default());
    put("jwt.min_secret_bytes", jwt::MIN_SECRET_LENGTH.to_string());
    put("jwt.max_token_length", field(&jwt_options, "max_token_length"));
    put("jwt.clock_tolerance_seconds", seconds(&field(&jwt_options, "clock_tolerance")));
    put("jwt.max_clock_tolerance_seconds", secs(jwt::MAX_CLOCK_TOLERANCE));
    put("jwt.require_exp", (field(&jwt_options, "allow_missing_exp") == "false").to_string());
    put("jwt.key_provider_timeout_seconds", seconds(&field(&jwt_options, "key_provider_timeout")));
    put("jwt.validator_timeout_seconds", seconds(&field(&jwt_options, "validator_timeout")));

    let key_options = format!("{:?}", ApiKeyStrategyOptions::default());
    put("api_key.min_secret_bytes", api_key::MIN_SECRET_BYTES.to_string());
    put("api_key.max_key_bytes", field(&key_options, "max_key_bytes"));
    put("api_key.validator_timeout_seconds", seconds(&field(&key_options, "validator_timeout")));
    put(
        "api_key.default_source",
        key_options.split("ApiKeySource::header(\"").nth(1).and_then(|s| s.split('"').next()).expect("a header source").to_string(),
    );

    put("cookie_keys.min_secret_bytes", signed_cookie::MIN_SECRET_BYTES.to_string());
    put("cookie_keys.max_secrets", signed_cookie::MAX_SECRETS.to_string());

    let params = Argon2Params::default();
    let limits = VerifyLimits::default();
    put("local.max_credential_bytes", DEFAULT_MAX_CREDENTIAL_BYTES.to_string());
    put("local.validator_timeout_seconds", seconds(&field(&format!("{:?}", LocalStrategyOptions::default()), "validator_timeout")));
    put("local.argon2_memory_kib", params.memory_kib.to_string());
    put("local.argon2_iterations", params.iterations.to_string());
    put("local.argon2_parallelism", params.parallelism.to_string());
    put("local.bcrypt_min_cost", MIN_BCRYPT_COST.to_string());
    put("local.verify_max_bcrypt_cost", limits.max_bcrypt_cost.to_string());
    put("local.verify_max_argon2_memory_kib", limits.max_argon2_memory_kib.to_string());
    put("local.verify_max_argon2_iterations", limits.max_argon2_iterations.to_string());
    put("local.verify_max_argon2_parallelism", limits.max_argon2_parallelism.to_string());
    put("local.verify_max_hash_bytes", limits.max_hash_bytes.to_string());
    put("local.acquire_timeout_seconds", secs(DEFAULT_ACQUIRE_TIMEOUT));

    let cookie = format!("{:?}", SessionCookie::default());
    let session = format!("{:?}", SessionOptions::with_secrets([SESSION_SECRET]).unwrap());
    put("session.cookie_name", field(&session, "name").trim_matches('"').to_string());
    put("session.cookie_path", field(&cookie, "path").trim_matches('"').to_string());
    put("session.cookie_http_only", field(&cookie, "http_only"));
    put("session.cookie_secure", field(&cookie, "secure"));
    put("session.cookie_same_site", field(&cookie, "same_site").trim_start_matches("Some(").trim_end_matches(')').to_string());
    put("session.idle_timeout_seconds", secs(DEFAULT_IDLE_TIMEOUT));
    put("session.absolute_timeout_seconds", secs(DEFAULT_ABSOLUTE_TIMEOUT));
    put("session.max_session_bytes", DEFAULT_MAX_SESSION_BYTES.to_string());
    put("session.store_timeout_seconds", secs(DEFAULT_STORE_TIMEOUT));

    let oauth = format!(
        "{:?}",
        OAuth2Options::new(
            OAuth2Endpoints::new("https://p.example/a", "https://p.example/t"),
            "id",
            "secret-secret",
            "https://app.example/cb"
        )
    );
    let http = format!("{:?}", OAuth2HttpOptions::default());
    put("oauth2.state_ttl_seconds", secs(DEFAULT_STATE_TTL));
    put("oauth2.pkce", field(&oauth, "pkce"));
    put("oauth2.max_code_bytes", DEFAULT_MAX_CODE_BYTES.to_string());
    put("oauth2.connect_timeout_seconds", seconds(&field(&http, "connect_timeout")));
    put("oauth2.timeout_seconds", seconds(&field(&http, "timeout")));
    put("oauth2.max_token_bytes", field(&http, "max_token_bytes"));
    put("oauth2.max_profile_bytes", field(&http, "max_profile_bytes"));
    put("oauth2.max_in_flight", field(&http, "max_in_flight"));
    put("oauth2.redeemed_states_max", DEFAULT_REDEEMED_STATES_MAX.to_string());

    // What nothing but the running module shows: renewal, the data kept at login, the state cookie, the URL.
    let h = Harness::new().await;
    let touched = h.post_json("/touch", &[], json!({})).await;
    let cookie = session_pair(&touched).expect("a session");
    let read = h.get("/open", &[("cookie", &cookie)]).await;
    put("session.rolling", (!set_cookies(&read).is_empty()).to_string());
    let login = h.post_json("/login-session", &[("cookie", &cookie)], json!({ "username": "ana", "password": PASSWORD })).await;
    let new_cookie = session_pair(&login).expect("a login cookie");
    let records = h.store.records().await;
    put("session.keep_session_info_on_login", records[&session_common::id_of(&new_cookie)]["seen"].is_boolean().to_string());
    let start = oauth2_common::login(&h.app, "/oauth").await;
    let state_cookie = oauth2_common::set_cookies(&start.response).into_iter().next().expect("a state cookie");
    put("oauth2.state_cookie", attributes(&state_cookie));

    let h = Harness::new().await;
    let r = h.post_json("/login?username=ana&password=secret", &[], json!({})).await;
    put("url_credentials.local", if r.status != 401 || Probe::count(&h.probe.local) != 0 { "on" } else { "off" }.to_string());
    let t = token("1");
    let r = h.get(&format!("/key?x-api-key={API_KEY}&api_key={API_KEY}&apikey={API_KEY}&key={API_KEY}"), &[]).await;
    put("url_credentials.api_key", if r.status != 401 || Probe::count(&h.probe.key) != 0 { "on" } else { "off" }.to_string());
    let r = h.get(&format!("/jwt?access_token={t}&token={t}&jwt={t}&id_token={t}"), &[]).await;
    put("url_credentials.jwt", if r.status != 401 || !h.probe.jwt_raw.lock().unwrap().is_empty() { "on" } else { "off" }.to_string());
    rows
}

#[tokio::test]
async fn every_row_of_the_defaults_table_matches_the_code() {
    let documented = documented();
    let actual = actual().await;
    let mut problems = Vec::new();
    for (key, value) in &documented {
        match actual.get(key) {
            None => problems.push(format!("row {key} is in the page and the test has no value for it")),
            Some(code) if code != value => problems.push(format!("row {key}: the page says {value:?}, the code says {code:?}")),
            Some(_) => {}
        }
    }
    for key in actual.keys().filter(|k| !documented.contains_key(*k)) {
        problems.push(format!("row {key} is in the code and not in the page"));
    }
    assert!(problems.is_empty(), "the defaults table is out of date:\n{}", problems.join("\n"));
    assert!(documented.len() >= 40, "{} rows", documented.len());
}

// ---- the scenarios of "Defaults are the same rule in every strategy" ----

#[tokio::test]
async fn secret_minimum() {
    for len in [31usize, 32] {
        let secret = "k".repeat(len);
        let accepted = len == 32;
        assert_eq!(CookieKeys::new([secret.as_bytes()]).is_ok(), accepted, "cookie keys of {len} bytes");
        assert_eq!(ExpectedApiKey::new(&secret).is_ok(), accepted, "an expected API key of {len} bytes");
        assert_eq!(SessionOptions::with_secrets([secret.as_bytes()]).is_ok(), accepted, "session secret of {len} bytes");
        assert_eq!(jwt_secret_starts(&secret).await, accepted, "a JWT HMAC secret of {len} bytes");
    }
}

/// Whether an application with a JWT strategy that signs with `secret` starts.
async fn jwt_secret_starts(secret: &str) -> bool {
    use corvis::auth::jwt::{ExtractJwt, JwtKey, JwtStrategy, JwtValidator};

    struct Secret(String);

    #[injectable]
    pub struct Validator {
        secret: std::sync::Arc<Secret>,
    }

    #[async_trait]
    impl JwtValidator for Validator {
        const NAME: &'static str = "jwt";
        type User = Value;

        fn options(&self) -> corvis::auth::jwt::JwtStrategyOptions {
            corvis::auth::jwt::JwtStrategyOptions::new()
                .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
                .secret_or_key(JwtKey::secret(self.secret.0.clone()))
        }

        async fn validate(&self, _: &Request, payload: Value) -> Result<Option<Value>, HttpException> {
            Ok(Some(payload))
        }
    }

    let module = corvis::testing::Test::create_testing_module()
        .with_logger(false)
        .providers(vec![
            Provider::value(Secret(secret.to_string())),
            Provider::class::<Validator>(),
            Provider::strategy::<JwtStrategy<Validator>>(),
        ])
        .compile()
        .await;
    match module {
        Err(_) => false,
        Ok(module) => {
            let mut app = module.create_corvis_application();
            app.init().await.is_ok()
        }
    }
}

#[tokio::test]
async fn cookies_set_by_the_module() {
    let h = Harness::new().await;
    let login = h.post_json("/login-session", &[], json!({ "username": "ana", "password": PASSWORD })).await;
    let session_cookie = set_cookies(&login).into_iter().find(|c| c.starts_with("connect.sid=")).expect("a session cookie");
    let start = oauth2_common::login(&h.app, "/oauth").await;
    let state_cookie = oauth2_common::set_cookies(&start.response).into_iter().next().expect("a state cookie");
    for (what, cookie) in [("session", session_cookie), ("OAuth2 state", state_cookie)] {
        for attribute in ["HttpOnly", "Secure", "SameSite=Lax"] {
            assert!(cookie.split(';').map(str::trim).any(|a| a == attribute), "the {what} cookie lacks {attribute}: {cookie}");
        }
        assert!(!cookie.contains("Domain="), "the {what} cookie has a Domain: {cookie}");
    }
}

#[tokio::test]
async fn credentials_in_a_url() {
    // Every strategy with its default options ignores credentials in the query string; the startup warning for the ones
    // that can be told to read them is tested in `auth_jwt_log.rs`, `auth_api_key_log.rs` and `auth_local.rs`.
    let h = Harness::new().await;
    let t = token("1");
    assert_eq!(h.post_json("/login?username=ana&password=secret", &[], json!({})).await.status, 401);
    assert_eq!(h.get(&format!("/key?api_key={API_KEY}&x-api-key={API_KEY}"), &[]).await.status, 401);
    assert_eq!(h.get(&format!("/jwt?access_token={t}&token={t}&jwt={t}"), &[]).await.status, 401);
    assert_eq!(h.probe.strategy_calls(), 0, "no validator ran");
}
