//! What the JWT strategy logs: one startup warning per risky option, naming the strategy, and nothing that holds a
//! token or a key. Its own test binary: the logger is process-wide, so the tests share one capturing sink and are
//! serialized.

#![cfg(feature = "jwt")]

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use corvis::auth::jwt::{ExtractJwt, JwtKey, JwtKeyProvider, JwtStrategy, JwtStrategyOptions, JwtValidator};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;
use corvis::testing::request;
use corvis::{LogLevel, LoggerService};
use serde::Serialize;

const SECRET: &str = "corvis-jwt-secret-one-0123456789abcdef";

#[derive(Debug, Clone, Serialize)]
pub struct Claims {
    sub: String,
}

fn base() -> JwtStrategyOptions {
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key(JwtKey::secret(SECRET))
}

macro_rules! jwt_app {
    ($validator:ident, $module:ident, $name:literal, $options:expr) => {
        #[injectable]
        pub struct $validator;

        #[async_trait]
        impl JwtValidator for $validator {
            const NAME: &'static str = $name;
            type User = Claims;

            fn options(&self) -> JwtStrategyOptions {
                $options
            }

            async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Claims>, HttpException> {
                Ok(payload["sub"].as_str().map(|sub| Claims { sub: sub.to_string() }))
            }
        }

        #[module(providers = [Provider::strategy::<JwtStrategy<$validator>>()], controllers = [ApiController])]
        pub struct $module;
    };
}

#[controller("")]
pub struct ApiController;

#[routes]
impl ApiController {
    #[get("me")]
    #[use_guards(AuthGuard::chain(["clean", "ignoring", "lenient", "url", "failing", "cookie", "hostcookie"]))]
    fn me(&self, user: User<Claims>) -> Value {
        json!({ "sub": user.sub })
    }
}

struct FailingKeys;

#[async_trait]
impl JwtKeyProvider for FailingKeys {
    async fn key(&self, _req: &Request, _raw_token: &str) -> anyhow::Result<JwtKey> {
        anyhow::bail!("the key store is down")
    }
}

jwt_app!(Clean, CleanModule, "clean", base());
jwt_app!(Ignoring, IgnoringModule, "ignoring", base().ignore_expiration(true));
jwt_app!(Lenient, LenientModule, "lenient", base().allow_missing_exp());
jwt_app!(
    Url,
    UrlModule,
    "url",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_url_query_parameter("token")).secret_or_key(JwtKey::secret(SECRET))
);
jwt_app!(
    Failing,
    FailingModule,
    "failing",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key_provider(FailingKeys)
);
jwt_app!(
    Plain,
    PlainModule,
    "cookie",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_cookie("jwt")).secret_or_key(JwtKey::secret(SECRET))
);
jwt_app!(
    Prefixed,
    PrefixedModule,
    "hostcookie",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_cookie("__Host-jwt")).secret_or_key(JwtKey::secret(SECRET))
);

#[module(
    providers = [
        Provider::strategy::<JwtStrategy<Clean>>(),
        Provider::strategy::<JwtStrategy<Ignoring>>(),
        Provider::strategy::<JwtStrategy<Lenient>>(),
        Provider::strategy::<JwtStrategy<Url>>(),
        Provider::strategy::<JwtStrategy<Failing>>(),
        Provider::strategy::<JwtStrategy<Plain>>(),
        Provider::strategy::<JwtStrategy<Prefixed>>(),
    ],
    controllers = [ApiController],
)]
pub struct AllModule;

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn capture() -> Captured {
    let captured: Captured = Arc::default();
    corvis::logger::set_logger(Capture(captured.clone()));
    corvis::logger::set_log_levels(&LogLevel::all());
    captured
}

fn warnings(captured: &Captured) -> Vec<String> {
    captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(level, _, ctx)| *level == LogLevel::Warn && ctx.as_deref() == Some("JwtStrategy"))
        .map(|(_, m, _)| m.clone())
        .collect()
}

#[tokio::test]
async fn risky_options_log_one_warning_each_naming_the_strategy() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let mut app = CorvisFactory::create::<AllModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let found = warnings(&captured);
    assert_eq!(found.len(), 4, "{found:?}");
    let has = |name: &str, what: &str| found.iter().any(|w| w.contains(&format!("\"{name}\"")) && w.contains(what));
    assert!(has("ignoring", "ignore_expiration"), "{found:?}");
    assert!(has("lenient", "allow_missing_exp"), "{found:?}");
    assert!(has("url", "URL query"), "{found:?}");
    // A cookie extractor whose name has no `__Host-` prefix warns, naming the strategy and the cookie; one with it does not.
    assert!(has("cookie", "the cookie \"jwt\", which has no __Host- prefix"), "{found:?}");
    assert!(!found.iter().any(|w| w.contains("\"hostcookie\"")), "{found:?}");
    assert!(!found.iter().any(|w| w.contains("\"clean\"")), "{found:?}");
}

#[tokio::test]
async fn logs_never_hold_a_token_or_a_key() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let mut app = CorvisFactory::create::<AllModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();

    let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let marker = "tokenmarker0123456789";
    let token =
        format!("{}.{}.{}", b64(br#"{"alg":"HS256","typ":"JWT"}"#), b64(format!(r#"{{"sub":"{marker}"}}"#).as_bytes()), "sigmarker");
    for call in [
        request(&app).get("/me").bearer(&token),
        request(&app).get("/me").query("token", &token),
        request(&app).get("/me").bearer("not.a.token"),
    ] {
        assert_eq!(call.send().await.status, 401);
    }
    let all = captured.lock().unwrap().iter().map(|(_, m, _)| m.clone()).collect::<Vec<_>>().join("\n");
    for secret in [marker, "sigmarker", SECRET, "not.a.token"] {
        assert!(!all.contains(secret), "a log line holds {secret}: {all}");
    }
}
