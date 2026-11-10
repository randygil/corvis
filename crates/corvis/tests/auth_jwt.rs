//! The JWT strategy against the original: every case recorded from `passport-jwt` 4.0.1 over `jsonwebtoken` 9.0.x
//! (`tests/fixtures/auth/jwt.mjs`) is replayed against the same routes with equivalent Corvis validators. The tokens
//! are signed again from the recorded token specs (time claims relative to now, with margins of tens of seconds), so no
//! Node is needed at test time. A case with a `divergence` is a behavior Corvis changes on purpose: it is asserted
//! against the answer the divergence records.
//!
//! Generated from the strategies of `jwt.mjs`: one validator and one `GET /<name>` route per strategy.

#![cfg(feature = "jwt")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine as _;
use corvis::auth::AuthGuard;
use corvis::auth::jwt::{ExtractJwt, JwtAlgorithm, JwtKey, JwtKeyProvider, JwtStrategy, JwtStrategyOptions, JwtValidator};
use corvis::prelude::*;
use corvis::testing::request;
use jsonwebtoken::{Algorithm, EncodingKey};
use serde::Serialize;

const FIXTURE: &str = include_str!("fixtures/auth/jwt.json");
const RSA_PUBLIC: &str = include_str!("fixtures/jwt_rsa_public.pem");
const RSA_PRIVATE: &str = include_str!("fixtures/jwt_rsa_private.pem");
const EC_PUBLIC: &str = include_str!("fixtures/jwt_ec_public.pem");
const EC_PRIVATE: &str = include_str!("fixtures/jwt_ec_private.pem");

const S1: &str = "corvis-jwt-secret-one-0123456789abcdef";
const S2: &str = "corvis-jwt-secret-two-0123456789abcdef";
const SA: &str = "corvis-jwt-secret-a-0123456789abcdef!!";
const SB: &str = "corvis-jwt-secret-b-0123456789abcdef!!";
const OTHER: &str = "corvis-jwt-secret-other-0123456789ab";

static VALIDATOR_CALLS: AtomicUsize = AtomicUsize::new(0);

/// The user of `jwt.mjs`: `{ id: payload.sub, role: payload.role }`, absent fields left out.
#[derive(Debug, Clone, Serialize)]
pub struct Claims {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<Value>,
}

/// `sub` "nobody" is refused, anything else is a user.
fn accept(payload: &Value) -> Option<Claims> {
    VALIDATOR_CALLS.fetch_add(1, Ordering::SeqCst);
    if payload.get("sub").and_then(Value::as_str) == Some("nobody") {
        return None;
    }
    Some(Claims { id: payload.get("sub").cloned(), role: payload.get("role").cloned() })
}

macro_rules! validator {
    ($ty:ident, $name:literal, $options:expr) => {
        #[injectable]
        pub struct $ty;

        #[async_trait]
        impl JwtValidator for $ty {
            const NAME: &'static str = $name;
            type User = Claims;

            fn options(&self) -> JwtStrategyOptions {
                $options
            }

            async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Claims>, HttpException> {
                Ok(accept(&payload))
            }
        }
    };
}

/// `secretOrKeyProvider` of the generator: the secret depends on the unverified `iss`.
struct ByIssuer;

#[async_trait]
impl JwtKeyProvider for ByIssuer {
    async fn key(&self, _req: &Request, raw_token: &str) -> anyhow::Result<JwtKey> {
        let payload = raw_token.split('.').nth(1).ok_or_else(|| anyhow::anyhow!("not a token"))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload)?;
        let claims: Value = serde_json::from_slice(&bytes)?;
        match claims["iss"].as_str() {
            Some("a") => Ok(JwtKey::secret(SA)),
            Some("b") => Ok(JwtKey::secret(SB)),
            _ => anyhow::bail!("unknown issuer"),
        }
    }
}

struct Failing;

#[async_trait]
impl JwtKeyProvider for Failing {
    async fn key(&self, _req: &Request, _raw_token: &str) -> anyhow::Result<JwtKey> {
        anyhow::bail!("no key")
    }
}

struct RsaProvider;

#[async_trait]
impl JwtKeyProvider for RsaProvider {
    async fn key(&self, _req: &Request, _raw_token: &str) -> anyhow::Result<JwtKey> {
        Ok(JwtKey::rsa_pem(RSA_PUBLIC))
    }
}

validator!(
    Bearer,
    "bearer",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key(JwtKey::secret(S1))
);
validator!(
    Refresh,
    "refresh",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key(JwtKey::secret(S2))
);
validator!(
    Scheme,
    "scheme",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_with_scheme("JWT")).secret_or_key(JwtKey::secret(S1))
);
validator!(
    Header,
    "header",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_header("x-token")).secret_or_key(JwtKey::secret(S1))
);
validator!(
    HeaderUpper,
    "header-upper",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_header("X-Token")).secret_or_key(JwtKey::secret(S1))
);
validator!(Cookie, "cookie", JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_cookie("jwt")).secret_or_key(JwtKey::secret(S1)));
validator!(
    Query,
    "query",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_url_query_parameter("token")).secret_or_key(JwtKey::secret(S1))
);
validator!(
    Body,
    "body",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_body_field("token")).secret_or_key(JwtKey::secret(S1))
);
validator!(
    Extractors,
    "extractors",
    JwtStrategyOptions::new()
        .jwt_from_request(ExtractJwt::from_extractors([ExtractJwt::from_cookie("jwt"), ExtractJwt::from_auth_header_as_bearer_token()]))
        .secret_or_key(JwtKey::secret(S1))
);
validator!(
    Audience,
    "audience",
    JwtStrategyOptions::new()
        .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
        .secret_or_key(JwtKey::secret(S1))
        .audience(["web", "mobile"])
);
validator!(
    AudienceWeb,
    "audience-web",
    JwtStrategyOptions::new()
        .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
        .secret_or_key(JwtKey::secret(S1))
        .audience("web")
);
validator!(
    Issuer,
    "issuer",
    JwtStrategyOptions::new()
        .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
        .secret_or_key(JwtKey::secret(S1))
        .issuer("https://auth.example")
);
validator!(
    ProviderStrategy,
    "provider",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key_provider(ByIssuer)
);
validator!(
    ProviderError,
    "provider-error",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key_provider(Failing)
);
validator!(
    ProviderRsa,
    "provider-rsa",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key_provider(RsaProvider)
);
validator!(
    Rs256,
    "rs256",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key(JwtKey::rsa_pem(RSA_PUBLIC))
);
validator!(
    Es256,
    "es256",
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token()).secret_or_key(JwtKey::ec_pem(EC_PUBLIC))
);
validator!(
    Hs256,
    "hs256",
    JwtStrategyOptions::new()
        .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
        .secret_or_key(JwtKey::secret(S1))
        .algorithms([JwtAlgorithm::HS256])
);
validator!(
    IgnoreExp,
    "ignore-exp",
    JwtStrategyOptions::new()
        .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
        .secret_or_key(JwtKey::secret(S1))
        .ignore_expiration(true)
);
validator!(
    Tolerance,
    "tolerance",
    JwtStrategyOptions::new()
        .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
        .secret_or_key(JwtKey::secret(S1))
        .clock_tolerance(Duration::from_secs(30))
);
validator!(
    Big,
    "big",
    JwtStrategyOptions::new()
        .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
        .secret_or_key(JwtKey::secret(S1))
        .max_token_length(16384)
);

fn show(req: &Request) -> Value {
    json!({ "user": req.user().unwrap_or(Value::Null) })
}

#[controller("")]
pub struct ApiController;

#[routes]
impl ApiController {
    #[get("bearer")]
    #[use_guards(AuthGuard::new("bearer"))]
    fn r_bearer(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("refresh")]
    #[use_guards(AuthGuard::new("refresh"))]
    fn r_refresh(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("scheme")]
    #[use_guards(AuthGuard::new("scheme"))]
    fn r_scheme(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("header")]
    #[use_guards(AuthGuard::new("header"))]
    fn r_header(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("header-upper")]
    #[use_guards(AuthGuard::new("header-upper"))]
    fn r_header_upper(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("cookie")]
    #[use_guards(AuthGuard::new("cookie"))]
    fn r_cookie(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("query")]
    #[use_guards(AuthGuard::new("query"))]
    fn r_query(&self, req: Request) -> Value {
        show(&req)
    }

    #[post("body")]
    #[use_guards(AuthGuard::new("body"))]
    fn r_body(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("extractors")]
    #[use_guards(AuthGuard::new("extractors"))]
    fn r_extractors(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("audience")]
    #[use_guards(AuthGuard::new("audience"))]
    fn r_audience(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("audience-web")]
    #[use_guards(AuthGuard::new("audience-web"))]
    fn r_audience_web(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("issuer")]
    #[use_guards(AuthGuard::new("issuer"))]
    fn r_issuer(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("provider")]
    #[use_guards(AuthGuard::new("provider"))]
    fn r_provider(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("provider-error")]
    #[use_guards(AuthGuard::new("provider-error"))]
    fn r_provider_error(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("provider-rsa")]
    #[use_guards(AuthGuard::new("provider-rsa"))]
    fn r_provider_rsa(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("rs256")]
    #[use_guards(AuthGuard::new("rs256"))]
    fn r_rs256(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("es256")]
    #[use_guards(AuthGuard::new("es256"))]
    fn r_es256(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("hs256")]
    #[use_guards(AuthGuard::new("hs256"))]
    fn r_hs256(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("ignore-exp")]
    #[use_guards(AuthGuard::new("ignore-exp"))]
    fn r_ignore_exp(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("tolerance")]
    #[use_guards(AuthGuard::new("tolerance"))]
    fn r_tolerance(&self, req: Request) -> Value {
        show(&req)
    }

    #[get("big")]
    #[use_guards(AuthGuard::new("big"))]
    fn r_big(&self, req: Request) -> Value {
        show(&req)
    }
}

#[module(
    providers = [
    Provider::strategy::<JwtStrategy<Bearer>>(),
    Provider::strategy::<JwtStrategy<Refresh>>(),
    Provider::strategy::<JwtStrategy<Scheme>>(),
    Provider::strategy::<JwtStrategy<Header>>(),
    Provider::strategy::<JwtStrategy<HeaderUpper>>(),
    Provider::strategy::<JwtStrategy<Cookie>>(),
    Provider::strategy::<JwtStrategy<Query>>(),
    Provider::strategy::<JwtStrategy<Body>>(),
    Provider::strategy::<JwtStrategy<Extractors>>(),
    Provider::strategy::<JwtStrategy<Audience>>(),
    Provider::strategy::<JwtStrategy<AudienceWeb>>(),
    Provider::strategy::<JwtStrategy<Issuer>>(),
    Provider::strategy::<JwtStrategy<ProviderStrategy>>(),
    Provider::strategy::<JwtStrategy<ProviderError>>(),
    Provider::strategy::<JwtStrategy<ProviderRsa>>(),
    Provider::strategy::<JwtStrategy<Rs256>>(),
    Provider::strategy::<JwtStrategy<Es256>>(),
    Provider::strategy::<JwtStrategy<Hs256>>(),
    Provider::strategy::<JwtStrategy<IgnoreExp>>(),
    Provider::strategy::<JwtStrategy<Tolerance>>(),
    Provider::strategy::<JwtStrategy<Big>>()
    ],
    controllers = [ApiController],
)]
pub struct ApiModule;

// ---- token specs ----

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Signs the token a spec describes (see `jwt.mjs`), with `now` for the claims relative to it.
fn sign(spec: &Value, now: i64) -> String {
    if let Some(raw) = spec["raw"].as_str() {
        return raw.to_string();
    }
    let alg_name = spec["alg"].as_str().unwrap_or("HS256");
    let alg: Algorithm = alg_name.parse().unwrap();
    let payload = match spec["payloadText"].as_str() {
        Some(text) => text.to_string(),
        None => {
            let mut claims = serde_json::Map::new();
            for (k, v) in spec["claims"].as_object().unwrap() {
                let value = match v.get("$now").and_then(Value::as_i64) {
                    Some(offset) => json!(now + offset),
                    None => v.clone(),
                };
                claims.insert(k.clone(), value);
            }
            if let Some(pad) = spec["pad"].as_u64() {
                claims.insert("pad".into(), json!("x".repeat(pad as usize)));
            }
            Value::Object(claims).to_string()
        }
    };
    let key = match spec["key"].as_str().unwrap() {
        "rsa" => EncodingKey::from_rsa_pem(RSA_PRIVATE.as_bytes()).unwrap(),
        "ec" => EncodingKey::from_ec_pem(EC_PRIVATE.as_bytes()).unwrap(),
        "rsa-public" => EncodingKey::from_secret(RSA_PUBLIC.as_bytes()),
        "s1" => EncodingKey::from_secret(S1.as_bytes()),
        "s2" => EncodingKey::from_secret(S2.as_bytes()),
        "sa" => EncodingKey::from_secret(SA.as_bytes()),
        "sb" => EncodingKey::from_secret(SB.as_bytes()),
        "other" => EncodingKey::from_secret(OTHER.as_bytes()),
        other => panic!("unknown key {other}"),
    };
    let message = format!("{}.{}", b64(format!(r#"{{"alg":"{alg_name}","typ":"JWT"}}"#).as_bytes()), b64(payload.as_bytes()));
    let signature = jsonwebtoken::crypto::sign(message.as_bytes(), &key, alg).unwrap();
    format!("{message}.{signature}")
}

fn fill(text: &str, token: &str, wrong: &str) -> String {
    text.replace("{{token}}", token).replace("{{wrong}}", wrong)
}

fn fill_value(value: &Value, token: &str, wrong: &str) -> Value {
    match value {
        Value::String(s) => Value::String(fill(s, token, wrong)),
        Value::Array(a) => Value::Array(a.iter().map(|v| fill_value(v, token, wrong)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), fill_value(v, token, wrong))).collect()),
        other => other.clone(),
    }
}

async fn app() -> CorvisApplication {
    let mut app = CorvisFactory::create::<ApiModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn every_recorded_case_is_replayed() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let versions = fixture["versions"].as_object().unwrap();
    assert_eq!(versions["passport-jwt"], "4.0.1");
    assert!(versions["jsonwebtoken"].as_str().unwrap().starts_with("9.0."), "{versions:?}");
    let cases = fixture["cases"].as_array().unwrap();
    assert!(cases.len() >= 60, "{} cases", cases.len());

    let app = app().await;
    let now = chrono_now();
    let mut diverged = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let token = case["token"].as_object().map(|_| sign(&case["token"], now)).unwrap_or_default();
        let wrong = case["wrong"].as_object().map(|_| sign(&case["wrong"], now)).unwrap_or_default();
        let req = &case["request"];
        let path = fill(req["path"].as_str().unwrap(), &token, &wrong);
        let mut call = match req["method"].as_str().unwrap() {
            "GET" => request(&app).get(&path),
            "POST" => request(&app).post(&path),
            other => panic!("{name}: method {other}"),
        };
        for (k, v) in req["headers"].as_object().unwrap() {
            call = call.header(k, &fill(v.as_str().unwrap(), &token, &wrong));
        }
        if !req["body"].is_null() {
            call = call.json(&fill_value(&req["body"], &token, &wrong));
        }
        let before = VALIDATOR_CALLS.load(Ordering::SeqCst);
        let r = call.send().await;
        let calls = VALIDATOR_CALLS.load(Ordering::SeqCst) - before;

        let recorded = &case["response"];
        let (status, body) = if case["divergence"].is_null() {
            (recorded["status"].as_u64().unwrap(), recorded["body"].clone())
        } else {
            diverged += 1;
            let divergence = &case["divergence"];
            assert!(!divergence["reason"].as_str().unwrap().is_empty(), "{name}: a divergence needs a reason");
            let status = divergence["status"].as_u64().unwrap();
            // Every divergent case uses the claims `{ sub: "7", role: "admin" }`.
            let body = match status {
                401 => json!({ "statusCode": 401, "message": "Unauthorized" }),
                _ => json!({ "user": { "id": "7", "role": "admin" } }),
            };
            (status, body)
        };
        assert_eq!(r.status as u64, status, "{name}: status, body {}", r.text());
        assert_eq!(r.json_value(), body, "{name}: body");
        assert_eq!(r.header("www-authenticate"), recorded["headers"]["www-authenticate"].as_str(), "{name}: www-authenticate");
        if status == 401 && name != "validator returns no user" {
            assert_eq!(calls, 0, "{name}: the validator ran for a token that was not verified");
        }
    }
    assert_eq!(diverged, 5);
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[test]
fn the_fixture_marks_every_divergence_with_its_corvis_answer() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let marked: Vec<&str> =
        fixture["cases"].as_array().unwrap().iter().filter(|c| !c["divergence"].is_null()).map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(marked, ["uppercase header name", "oversized token", "token without exp", "string iat", "string payload"]);
}
