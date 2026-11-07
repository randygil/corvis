//! JWT: `JwtModule.register()` / `registerAsync()` and `JwtService`.
//!
//! ```ignore
//! #[module(imports = [JwtModule::register(JwtModuleOptions::new("secret").expires_in("60s").global(true))])]
//! pub struct AuthModule;
//!
//! let token = self.jwt.sign(&json!({ "sub": user.id, "username": user.name }))?;
//! let claims: Claims = self.jwt.verify(&token)?;
//! ```

use std::collections::HashSet;
use std::future::Future;

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Validation};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, ResolveCtx, Token};
use crate::http::{HttpException, UnauthorizedException};

pub use jsonwebtoken::Algorithm as JwtAlgorithm;

/// Options for `JwtModule::register`.
#[derive(Debug, Clone)]
pub struct JwtModuleOptions {
    pub secret: String,
    /// `signOptions.expiresIn`: `"60s"`, `"15m"`, `"1h"`, `"7d"` or seconds (`"3600"`).
    pub expires_in: Option<String>,
    pub algorithm: Algorithm,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub global: bool,
}

impl JwtModuleOptions {
    pub fn new(secret: impl Into<String>) -> Self {
        JwtModuleOptions { secret: secret.into(), expires_in: None, algorithm: Algorithm::HS256, issuer: None, audience: None, global: false }
    }
    pub fn expires_in(mut self, v: impl Into<String>) -> Self {
        self.expires_in = Some(v.into());
        self
    }
    pub fn algorithm(mut self, a: Algorithm) -> Self {
        self.algorithm = a;
        self
    }
    pub fn issuer(mut self, i: impl Into<String>) -> Self {
        self.issuer = Some(i.into());
        self
    }
    pub fn audience(mut self, a: impl Into<String>) -> Self {
        self.audience = Some(a.into());
        self
    }
    pub fn global(mut self, g: bool) -> Self {
        self.global = g;
        self
    }
}

/// Per-call sign options (`jwtService.sign(payload, { expiresIn, secret })`).
#[derive(Debug, Clone, Default)]
pub struct JwtSignOptions {
    pub expires_in: Option<String>,
    pub secret: Option<String>,
    pub subject: Option<String>,
}

/// Parses `"60s"`, `"15m"`, `"2h"`, `"7d"`, `"1w"` or a number of seconds.
pub fn parse_duration_secs(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return Some(n);
    }
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = num.parse().ok()?;
    let mult = match unit.trim().to_lowercase().as_str() {
        "ms" => return Some(n / 1000),
        "s" | "sec" | "secs" | "seconds" => 1,
        "m" | "min" | "mins" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        "w" | "week" | "weeks" => 604_800,
        "y" | "year" | "years" => 31_557_600,
        _ => return None,
    };
    Some(n * mult)
}

/// Signs and verifies JSON Web Tokens.
#[derive(Clone)]
pub struct JwtService {
    options: JwtModuleOptions,
}

impl JwtService {
    pub fn new(options: JwtModuleOptions) -> Self {
        JwtService { options }
    }

    /// `jwtService.sign(payload)`. Adds `iat` (and `exp` when `expires_in` is configured).
    pub fn sign(&self, payload: &impl Serialize) -> crate::Result<String> {
        self.sign_with(payload, JwtSignOptions::default())
    }

    pub fn sign_with(&self, payload: &impl Serialize, opts: JwtSignOptions) -> crate::Result<String> {
        let mut claims = serde_json::to_value(payload)?;
        if let Value::Object(map) = &mut claims {
            let now = chrono::Utc::now().timestamp();
            map.entry("iat").or_insert(Value::from(now));
            let exp = opts.expires_in.as_deref().or(self.options.expires_in.as_deref()).and_then(parse_duration_secs);
            if let Some(exp) = exp {
                map.insert("exp".into(), Value::from(now + exp));
            }
            if let Some(iss) = &self.options.issuer {
                map.entry("iss").or_insert(Value::from(iss.clone()));
            }
            if let Some(aud) = &self.options.audience {
                map.entry("aud").or_insert(Value::from(aud.clone()));
            }
            if let Some(sub) = opts.subject {
                map.insert("sub".into(), Value::from(sub));
            }
        }
        let secret = opts.secret.unwrap_or_else(|| self.options.secret.clone());
        // node-jsonwebtoken (jws) writes the header as `{"alg":..,"typ":"JWT"}`, in that order;
        // the `jsonwebtoken` crate serializes `typ` first, so the header is built by hand.
        use base64::Engine as _;
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let header = format!("{{\"alg\":\"{:?}\",\"typ\":\"JWT\"}}", self.options.algorithm);
        let message = format!("{}.{}", b64(header.as_bytes()), b64(&serde_json::to_vec(&claims)?));
        let key = EncodingKey::from_secret(secret.as_bytes());
        let signature = jsonwebtoken::crypto::sign(message.as_bytes(), &key, self.options.algorithm)?;
        Ok(format!("{message}.{signature}"))
    }

    /// `await jwtService.signAsync(payload)`.
    pub async fn sign_async(&self, payload: &impl Serialize) -> crate::Result<String> {
        self.sign(payload)
    }

    /// `jwtService.verify(token)` – validates signature and expiration.
    pub fn verify<T: DeserializeOwned>(&self, token: &str) -> crate::Result<T> {
        let mut validation = Validation::new(self.options.algorithm);
        validation.required_spec_claims = HashSet::new();
        validation.leeway = 0;
        if let Some(iss) = &self.options.issuer {
            validation.set_issuer(&[iss]);
        }
        match &self.options.audience {
            Some(aud) => validation.set_audience(&[aud]),
            None => validation.validate_aud = false,
        }
        let data = jsonwebtoken::decode::<T>(token, &DecodingKey::from_secret(self.options.secret.as_bytes()), &validation)?;
        Ok(data.claims)
    }

    /// `await jwtService.verifyAsync(token)`.
    pub async fn verify_async<T: DeserializeOwned>(&self, token: &str) -> crate::Result<T> {
        self.verify(token)
    }

    /// Verifies a token mapping failures to `401 Unauthorized`.
    pub fn verify_or_unauthorized<T: DeserializeOwned>(&self, token: &str) -> Result<T, HttpException> {
        self.verify(token).map_err(|_| UnauthorizedException.into())
    }

    /// `jwtService.decode(token)` – no signature verification.
    pub fn decode(&self, token: &str) -> Option<Value> {
        let payload = token.split('.').nth(1)?;
        use base64::Engine;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

/// Extracts the bearer token from the `Authorization` header.
pub fn extract_bearer_token(req: &crate::http::Request) -> Option<String> {
    let h = req.header("authorization")?;
    let (kind, token) = h.split_once(' ')?;
    kind.eq_ignore_ascii_case("bearer").then(|| token.trim().to_string())
}

/// `JwtModule`.
#[derive(Default)]
pub struct JwtModule;
crate::default_injectable!(JwtModule);

impl Module for JwtModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<JwtModule>("JwtModule").module_provider(Provider::class::<JwtModule>())
    }
}

impl JwtModule {
    /// `JwtModule.register({ secret, signOptions: { expiresIn } })`.
    pub fn register(options: JwtModuleOptions) -> DynamicModule {
        let global = options.global;
        DynamicModule::new::<JwtModule>()
            .provider(Provider::use_value(Token::of::<JwtService>(), JwtService::new(options)))
            .export(Token::of::<JwtService>())
            .global(global)
    }

    /// `JwtModule.registerAsync({ useFactory: async (config) => ({ ... }), inject: [ConfigService] })`.
    pub fn register_async<F, Fut>(global: bool, factory: F) -> DynamicModule
    where
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<JwtModuleOptions>> + Send + 'static,
    {
        let factory = std::sync::Arc::new(factory);
        DynamicModule::new::<JwtModule>()
            .provider(Provider::use_factory(Token::of::<JwtService>(), move |ctx| {
                let f = factory.clone();
                async move { Ok(JwtService::new(f(ctx).await?)) }
            }))
            .export(Token::of::<JwtService>())
            .global(global)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let jwt = JwtService::new(JwtModuleOptions::new("s3cr3t").expires_in("1h"));
        let token = jwt.sign(&serde_json::json!({ "sub": 1, "username": "john" })).unwrap();
        // Same header bytes as node-jsonwebtoken: {"alg":"HS256","typ":"JWT"}.
        assert!(token.starts_with("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9."));
        let claims: Value = jwt.verify(&token).unwrap();
        assert_eq!(claims["username"], "john");
        assert!(claims["exp"].as_i64().unwrap() > claims["iat"].as_i64().unwrap());
        assert_eq!(jwt.decode(&token).unwrap()["sub"], 1);
        let other = JwtService::new(JwtModuleOptions::new("other"));
        assert!(other.verify::<Value>(&token).is_err());
        assert_eq!(parse_duration_secs("15m"), Some(900));
        assert_eq!(parse_duration_secs("7d"), Some(604_800));
    }
}
