//! A validator that returns an empty user (`false`, `null`, `0`, `""`) never authenticates a request, for every kind of
//! strategy. The originals fail closed twice: the strategies call `fail()` for a falsy user and the guard throws a 401
//! for a missing one (`tests/fixtures/auth/custom.json` records it for `passport-custom`). A validator written as
//! `type User = bool; Ok(Some(expected.matches(key)))` is the case this protects.

// Each strategy kind is behind its own feature, so a build with only some of them leaves shared helpers unused.
#![cfg_attr(not(all(feature = "auth-api-key", feature = "auth-local", feature = "jwt")), allow(unused_imports, dead_code))]

use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;
use corvis::testing::request;
use serde::Serialize;

/// What the validators hand back, picked by the header `x-pick`: the empty values and the values that only look
/// empty in another language.
fn pick(req: &Request) -> Value {
    match req.header("x-pick") {
        Some("false") => json!(false),
        Some("null") => Value::Null,
        Some("zero") => json!(0),
        Some("float-zero") => json!(0.0),
        Some("negative-zero") => json!(-0.0),
        Some("empty") => json!(""),
        Some("string-zero") => json!("0"),
        Some("string-false") => json!("false"),
        Some("object") => json!({}),
        Some("array") => json!([]),
        Some("true") => json!(true),
        Some("one") => json!(1),
        _ => json!({ "id": "real" }),
    }
}

const EMPTY: [&str; 6] = ["false", "null", "zero", "float-zero", "negative-zero", "empty"];
const REAL: [&str; 7] = ["string-zero", "string-false", "object", "array", "true", "one", "real"];

const KEY: &str = "test-key-000000000000000000000000000001";
const SECRET: &str = "empty-user-secret-0123456789abcdef-01";

fn unauthorized() -> Value {
    json!({ "statusCode": 401, "message": "Unauthorized" })
}

// ---- the natural translations of `return isValid` and `return repo.find(id)` ----

#[derive(Debug, Clone, Serialize)]
pub struct Service {
    name: &'static str,
}

#[cfg(feature = "auth-api-key")]
mod api_key {
    use corvis::auth::api_key::{ApiKey, ApiKeyStrategy, ApiKeyValidator};

    use super::*;

    /// `Ok(Some(expected.matches(key)))`: a boolean user.
    #[injectable]
    pub struct Boolean;

    #[async_trait]
    impl ApiKeyValidator for Boolean {
        const NAME: &'static str = "boolean";
        type User = bool;
        async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<bool>, HttpException> {
            Ok(Some(key.expose() == super::KEY))
        }
    }

    /// `Ok(Some(repo.find(id)))`: an optional user inside `Some`.
    #[injectable]
    pub struct Optional;

    #[async_trait]
    impl ApiKeyValidator for Optional {
        const NAME: &'static str = "optional";
        type User = Option<Service>;
        async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Option<Service>>, HttpException> {
            Ok(Some((key.expose() == super::KEY).then_some(Service { name: "billing" })))
        }
    }

    #[injectable]
    pub struct Picking;

    #[async_trait]
    impl ApiKeyValidator for Picking {
        const NAME: &'static str = "picking";
        type User = Value;
        async fn validate(&self, req: &Request, _key: &ApiKey) -> Result<Option<Value>, HttpException> {
            Ok(Some(super::pick(req)))
        }
    }

    #[controller("key")]
    pub struct KeyRoutes;

    #[routes]
    impl KeyRoutes {
        #[get("boolean")]
        #[use_guards(AuthGuard::new("boolean"))]
        fn boolean(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[get("optional")]
        #[use_guards(AuthGuard::new("optional"))]
        fn optional(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[get("picking")]
        #[use_guards(AuthGuard::new("picking"))]
        fn picking(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[get("picking-optional")]
        #[use_guards(AuthGuard::new("picking").optional())]
        fn picking_optional(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }

        #[get("picking-typed")]
        #[use_guards(AuthGuard::new("picking"))]
        fn picking_typed(&self, user: User<Value>) -> Value {
            json!({ "user": user.0 })
        }
    }

    #[module(
        providers = [
            Provider::strategy::<ApiKeyStrategy<Boolean>>(),
            Provider::strategy::<ApiKeyStrategy<Optional>>(),
            Provider::strategy::<ApiKeyStrategy<Picking>>(),
        ],
        controllers = [KeyRoutes],
    )]
    pub struct KeyModule;
}

#[cfg(feature = "auth-api-key")]
#[corvis::test]
async fn an_api_key_validator_that_returns_an_empty_user_does_not_authenticate() {
    let mut app = CorvisFactory::create::<api_key::KeyModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    // The boolean user: `true` is a user, `false` is not.
    let r = request(&app).get("/key/boolean").header("x-api-key", KEY).send().await;
    assert_eq!((r.status, r.json_value()), (200, json!({ "user": true })));
    let r = request(&app).get("/key/boolean").header("x-api-key", "test-key-wrong-0000000000000000000000001").send().await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    // `Some(None)`.
    let r = request(&app).get("/key/optional").header("x-api-key", KEY).send().await;
    assert_eq!((r.status, r.json_value()), (200, json!({ "user": { "name": "billing" } })));
    let r = request(&app).get("/key/optional").header("x-api-key", "test-key-wrong-0000000000000000000000001").send().await;
    assert_eq!((r.status, r.json_value()), (401, unauthorized()));
    for p in EMPTY {
        let r = request(&app).get("/key/picking").header("x-api-key", KEY).header("x-pick", p).send().await;
        assert_eq!((r.status, r.json_value()), (401, unauthorized()), "{p}");
        let r = request(&app).get("/key/picking-typed").header("x-api-key", KEY).header("x-pick", p).send().await;
        assert_eq!(r.status, 401, "{p}");
        // An optional guard lets the request through with no user, not with the empty one.
        let r = request(&app).get("/key/picking-optional").header("x-api-key", KEY).header("x-pick", p).send().await;
        assert_eq!((r.status, r.json_value()), (200, json!({ "user": null })), "{p}");
    }
    for p in REAL {
        let r = request(&app).get("/key/picking").header("x-api-key", KEY).header("x-pick", p).send().await;
        assert_eq!(r.status, 200, "{p}");
    }
}

#[cfg(feature = "jwt")]
mod jwt {
    use base64::Engine as _;
    use corvis::auth::jwt::{ExtractJwt, JwtKey, JwtStrategy, JwtStrategyOptions, JwtValidator};

    use super::*;

    #[injectable]
    pub struct Picking;

    #[async_trait]
    impl JwtValidator for Picking {
        const NAME: &'static str = "jwt-picking";
        type User = Value;
        fn options(&self) -> JwtStrategyOptions {
            JwtStrategyOptions::new()
                .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
                .secret_or_key(JwtKey::secret(super::SECRET))
        }
        async fn validate(&self, req: &Request, _payload: Value) -> Result<Option<Value>, HttpException> {
            Ok(Some(super::pick(req)))
        }
    }

    #[controller("jwt")]
    pub struct JwtRoutes;

    #[routes]
    impl JwtRoutes {
        #[get("picking")]
        #[use_guards(AuthGuard::new("jwt-picking"))]
        fn picking(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }
    }

    #[module(providers = [Provider::strategy::<JwtStrategy<Picking>>()], controllers = [JwtRoutes])]
    pub struct JwtModule;

    pub fn token() -> String {
        let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        let exp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600;
        let message = format!("{}.{}", b64(br#"{"alg":"HS256","typ":"JWT"}"#), b64(format!(r#"{{"sub":"u","exp":{exp}}}"#).as_bytes()));
        let key = jsonwebtoken::EncodingKey::from_secret(super::SECRET.as_bytes());
        let signature = jsonwebtoken::crypto::sign(message.as_bytes(), &key, jsonwebtoken::Algorithm::HS256).unwrap();
        format!("{message}.{signature}")
    }
}

#[cfg(feature = "jwt")]
#[corvis::test]
async fn a_jwt_validator_that_returns_an_empty_user_does_not_authenticate() {
    let mut app = CorvisFactory::create::<jwt::JwtModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let token = jwt::token();
    for p in EMPTY {
        let r = request(&app).get("/jwt/picking").bearer(&token).header("x-pick", p).send().await;
        assert_eq!((r.status, r.json_value()), (401, unauthorized()), "{p}");
    }
    for p in REAL {
        let r = request(&app).get("/jwt/picking").bearer(&token).header("x-pick", p).send().await;
        assert_eq!(r.status, 200, "{p}");
    }
}

#[cfg(feature = "auth-local")]
mod local {
    use corvis::auth::local::{LocalStrategy, LocalValidator};

    use super::*;

    #[injectable]
    pub struct Picking;

    #[async_trait]
    impl LocalValidator for Picking {
        const NAME: &'static str = "local-picking";
        type User = Value;
        async fn validate(&self, req: &Request, _username: &str, _password: &str) -> Result<Option<Value>, HttpException> {
            Ok(Some(super::pick(req)))
        }
    }

    #[controller("local")]
    pub struct LocalRoutes;

    #[routes]
    impl LocalRoutes {
        #[post("picking")]
        #[use_guards(AuthGuard::new("local-picking"))]
        fn picking(&self, req: Request) -> Value {
            json!({ "user": req.user() })
        }
    }

    #[module(providers = [Provider::strategy::<LocalStrategy<Picking>>()], controllers = [LocalRoutes])]
    pub struct LocalModule;
}

#[cfg(feature = "auth-local")]
#[corvis::test]
async fn a_local_validator_that_returns_an_empty_user_does_not_authenticate() {
    let mut app = CorvisFactory::create::<local::LocalModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    let login =
        |p: &'static str| request(&app).post("/local/picking").header("x-pick", p).json(&json!({ "username": "u", "password": "p" }));
    for p in EMPTY {
        let r = login(p).send().await;
        assert_eq!((r.status, r.json_value()), (401, unauthorized()), "{p}");
    }
    for p in REAL {
        assert_eq!(login(p).send().await.status, 201, "{p}");
    }
}
