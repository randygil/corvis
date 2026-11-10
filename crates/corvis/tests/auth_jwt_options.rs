//! The JWT strategy's options: the startup checks, the opt-outs, the limits and the validator's results. What the
//! strategy shares with `passport-jwt` is replayed from recorded fixtures by `tests/auth_jwt.rs`.

#![cfg(feature = "jwt")]

use std::time::Duration;

use base64::Engine as _;
use corvis::auth::jwt::{ExtractJwt, JwtAlgorithm, JwtKey, JwtKeyProvider, JwtStrategy, JwtStrategyOptions, JwtValidator};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;
use corvis::testing::request;
use jsonwebtoken::{Algorithm, EncodingKey};
use serde::{Deserialize, Serialize};

const SECRET: &str = "corvis-jwt-secret-one-0123456789abcdef";
const RSA_PUBLIC: &str = include_str!("fixtures/jwt_rsa_public.pem");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Claims {
    sub: String,
}

fn base() -> JwtStrategyOptions {
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key(JwtKey::secret(SECRET))
}

macro_rules! jwt_app {
    ($validator:ident, $module:ident, $options:expr) => {
        #[injectable]
        pub struct $validator;

        #[async_trait]
        impl JwtValidator for $validator {
            type User = Claims;

            fn options(&self) -> JwtStrategyOptions {
                $options
            }

            async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Claims>, HttpException> {
                match payload["sub"].as_str() {
                    Some("forbidden") => Err(ForbiddenException::new("Account locked")),
                    Some(sub) => Ok(Some(Claims { sub: sub.to_string() })),
                    None => Ok(None),
                }
            }

            fn info(&self, user: &Claims) -> Option<Value> {
                Some(json!({ "sub": user.sub }))
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
    #[use_guards(AuthGuard::new("jwt"))]
    fn me(&self, user: User<Claims>, req: Request) -> Value {
        json!({ "sub": user.sub, "user": req.user(), "info": req.auth_info() })
    }
}

fn sign(claims: &Value) -> String {
    let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let message = format!("{}.{}", b64(br#"{"alg":"HS256","typ":"JWT"}"#), b64(claims.to_string().as_bytes()));
    let signature = jsonwebtoken::crypto::sign(message.as_bytes(), &EncodingKey::from_secret(SECRET.as_bytes()), Algorithm::HS256).unwrap();
    format!("{message}.{signature}")
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

async fn start<M: corvis::Module>() -> CorvisApplication {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

async fn startup_error<M: corvis::Module>() -> String {
    match CorvisFactory::create::<M>().await {
        Ok(mut app) => match app.init().await {
            Ok(()) => panic!("the application started"),
            Err(e) => e.to_string(),
        },
        Err(e) => e.to_string(),
    }
}

// ---- startup checks ----

jwt_app!(NoExtractor, NoExtractorModule, JwtStrategyOptions::new().secret_or_key(JwtKey::secret(SECRET)));
jwt_app!(NoKey, NoKeyModule, JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_cookie("jwt")));
jwt_app!(Both, BothModule, base().secret_or_key_provider(Always));
jwt_app!(EmptySecret, EmptySecretModule, base().secret_or_key(JwtKey::secret("")));
jwt_app!(ShortSecret, ShortSecretModule, base().secret_or_key(JwtKey::secret("changeme")));
jwt_app!(BadPem, BadPemModule, base().secret_or_key(JwtKey::rsa_pem("not a pem")));
jwt_app!(EmptyAlgorithms, EmptyAlgorithmsModule, base().algorithms([]));
jwt_app!(HmacWithPublicKey, HmacWithPublicKeyModule, base().secret_or_key(JwtKey::rsa_pem(RSA_PUBLIC)).algorithms([JwtAlgorithm::HS256]));
jwt_app!(Mixed, MixedModule, base().secret_or_key(JwtKey::rsa_pem(RSA_PUBLIC)).algorithms([JwtAlgorithm::RS256, JwtAlgorithm::ES256]));
jwt_app!(
    EdWithoutAlgorithms,
    EdWithoutAlgorithmsModule,
    base().secret_or_key(JwtKey::ed_pem(
        "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAGb9ECWmEzf6FQbrBZ9w7lshQhqowtrbLDFw4rXAxZuE=\n-----END PUBLIC KEY-----\n"
    ))
);
jwt_app!(EmptyAudience, EmptyAudienceModule, base().audience(""));
jwt_app!(EmptyIssuerList, EmptyIssuerListModule, base().issuer(Vec::<String>::new()));
jwt_app!(ToleranceTooLarge, ToleranceTooLargeModule, base().clock_tolerance(Duration::from_secs(301)));
jwt_app!(ZeroLength, ZeroLengthModule, base().max_token_length(0));

struct Always;

#[async_trait]
impl JwtKeyProvider for Always {
    async fn key(&self, _req: &Request, _raw_token: &str) -> anyhow::Result<JwtKey> {
        Ok(JwtKey::secret(SECRET))
    }
}

#[corvis::test]
async fn invalid_options_stop_startup_with_an_error_naming_the_strategy_and_the_problem() {
    let cases = [
        (startup_error::<NoExtractorModule>().await, "no token extractor"),
        (startup_error::<NoKeyModule>().await, "no key"),
        (startup_error::<BothModule>().await, "both secret_or_key and secret_or_key_provider"),
        (startup_error::<EmptySecretModule>().await, "the HMAC secret is empty"),
        (startup_error::<ShortSecretModule>().await, "8 bytes long; use at least 32 bytes"),
        (startup_error::<BadPemModule>().await, "not valid PEM"),
        (startup_error::<EmptyAlgorithmsModule>().await, "algorithms is an empty list"),
        (startup_error::<HmacWithPublicKeyModule>().await, "HMAC algorithms but the key is RSA"),
        (startup_error::<MixedModule>().await, "mixes key families"),
        (startup_error::<EdWithoutAlgorithmsModule>().await, "explicit algorithms list"),
        (startup_error::<EmptyAudienceModule>().await, "audience has an empty value"),
        (startup_error::<EmptyIssuerListModule>().await, "issuer is an empty list"),
        (startup_error::<ToleranceTooLargeModule>().await, "clock_tolerance is 301s"),
        (startup_error::<ZeroLengthModule>().await, "max_token_length is 0"),
    ];
    for (error, problem) in cases {
        assert!(error.contains("JwtStrategy \"jwt\"") && error.contains(problem), "{problem}: {error}");
        assert!(!error.contains("changeme") && !error.contains(SECRET), "the error must not print the secret: {error}");
    }
}

// ---- behavior ----

jwt_app!(Defaults, DefaultsModule, base());
jwt_app!(MissingExpAllowed, MissingExpAllowedModule, base().allow_missing_exp());
jwt_app!(Tolerant, TolerantModule, base().clock_tolerance(Duration::from_secs(300)));
jwt_app!(
    ShortKeyProvider,
    ShortKeyProviderModule,
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_cookie("jwt")).secret_or_key_provider(ShortKeys)
);

struct ShortKeys;

#[async_trait]
impl JwtKeyProvider for ShortKeys {
    async fn key(&self, _req: &Request, _raw_token: &str) -> anyhow::Result<JwtKey> {
        Ok(JwtKey::secret("short"))
    }
}

#[corvis::test]
async fn the_handler_gets_the_typed_user_the_json_user_and_the_info() {
    let app = start::<DefaultsModule>().await;
    let token = sign(&json!({ "sub": "7", "exp": now() + 3600 }));
    let r = request(&app).get("/me").bearer(&token).send().await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(r.json_value(), json!({ "sub": "7", "user": { "sub": "7" }, "info": { "sub": "7" } }));
}

#[corvis::test]
async fn an_exception_of_the_validator_is_passed_on() {
    let app = start::<DefaultsModule>().await;
    let token = sign(&json!({ "sub": "forbidden", "exp": now() + 3600 }));
    let r = request(&app).get("/me").bearer(&token).send().await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json_value()["message"], "Account locked");
    // A payload the validator does not recognize is a 401.
    let r = request(&app).get("/me").bearer(&sign(&json!({ "exp": now() + 3600 }))).send().await;
    assert_eq!(r.status, 401);
}

#[corvis::test]
async fn a_token_without_exp_needs_the_opt_out() {
    let token = sign(&json!({ "sub": "7" }));
    let strict = start::<DefaultsModule>().await;
    assert_eq!(request(&strict).get("/me").bearer(&token).send().await.status, 401);
    let lenient = start::<MissingExpAllowedModule>().await;
    assert_eq!(request(&lenient).get("/me").bearer(&token).send().await.status, 200);
}

#[corvis::test]
async fn the_token_length_limit_is_in_bytes_and_inclusive() {
    let app = start::<DefaultsModule>().await;
    let exp = now() + 3600;
    // Find the padding that gives a token of exactly 8192 bytes.
    let length = |pad: usize| sign(&json!({ "sub": "7", "exp": exp, "pad": "x".repeat(pad) })).len();
    let base_len = length(0);
    // The payload is base64 encoded, so the length grows by 4 bytes per 3 padding bytes; search the exact value.
    let pad = (0..9000).find(|p| length(*p) == 8192).unwrap_or_else(|| panic!("no padding gives 8192 bytes from {base_len}"));
    let at_limit = sign(&json!({ "sub": "7", "exp": exp, "pad": "x".repeat(pad) }));
    assert_eq!(at_limit.len(), 8192);
    assert_eq!(request(&app).get("/me").bearer(&at_limit).send().await.status, 200);
    let over = (pad..pad + 10).map(|p| sign(&json!({ "sub": "7", "exp": exp, "pad": "x".repeat(p) }))).find(|t| t.len() > 8192).unwrap();
    assert_eq!(request(&app).get("/me").bearer(&over).send().await.status, 401);
}

#[corvis::test]
async fn the_clock_tolerance_is_applied_to_exp_and_nbf() {
    let app = start::<TolerantModule>().await;
    let expired = sign(&json!({ "sub": "7", "exp": now() - 200 }));
    assert_eq!(request(&app).get("/me").bearer(&expired).send().await.status, 200);
    let way_expired = sign(&json!({ "sub": "7", "exp": now() - 1000 }));
    assert_eq!(request(&app).get("/me").bearer(&way_expired).send().await.status, 401);
    let early = sign(&json!({ "sub": "7", "exp": now() + 3600, "nbf": now() + 200 }));
    assert_eq!(request(&app).get("/me").bearer(&early).send().await.status, 200);
}

#[corvis::test]
async fn a_short_secret_from_a_key_provider_fails_the_request() {
    let app = start::<ShortKeyProviderModule>().await;
    let token = sign(&json!({ "sub": "7", "exp": now() + 3600 }));
    let r = request(&app).get("/me").header("cookie", &format!("jwt={token}")).send().await;
    assert_eq!(r.status, 401);
}

#[corvis::test]
async fn the_answer_of_a_failed_token_does_not_say_why() {
    let app = start::<DefaultsModule>().await;
    for bearer in ["abc", "a.b.c", "", &sign(&json!({ "sub": "7", "exp": now() - 100 }))] {
        let r = request(&app).get("/me").header("authorization", &format!("Bearer {bearer}")).send().await;
        assert_eq!(r.status, 401, "{bearer}");
        assert_eq!(r.json_value(), json!({ "statusCode": 401, "message": "Unauthorized" }));
        assert_eq!(r.header("www-authenticate"), None);
    }
}

/// Random text in the `Authorization` header, the cookie, the query and the body ends in 401 or a user, never a panic
/// or a 500. (Bytes that are not UTF-8 are covered by the extractor's own property test.)
#[test]
fn random_credentials_end_in_401_or_a_user() {
    use proptest::prelude::*;
    use proptest::test_runner::{Config, TestRunner};

    let rt = tokio::runtime::Runtime::new().unwrap();
    let app = rt.block_on(start::<DefaultsModule>());
    let mut runner = TestRunner::new(Config::with_cases(64));
    runner
        .run(&"[A-Za-z0-9._ \t\u{a0}\u{e9}=;,-]{0,300}", |text| {
            let status = rt.block_on(async {
                let r = request(&app).get("/me").header("authorization", &format!("Bearer {text}")).send().await;
                let c = request(&app).get("/me").header("authorization", &text).send().await;
                (r.status, c.status)
            });
            prop_assert!(status.0 == 401 || status.0 == 200, "{}", status.0);
            prop_assert!(status.1 == 401 || status.1 == 200, "{}", status.1);
            Ok(())
        })
        .unwrap();
}
