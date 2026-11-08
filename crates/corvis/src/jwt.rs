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
    /// HMAC secret (`HS256`/`HS384`/`HS512`). As in `@nestjs/jwt`, a non-empty `secret` takes
    /// precedence over `private_key`/`public_key`: leave it empty (`JwtModuleOptions::new("")`)
    /// to use PEM keys with `RS*`/`PS*`/`ES*`/`EdDSA`. Signing and verifying fail without a key.
    pub secret: String,
    /// `signOptions.expiresIn`, with the grammar of the `ms` package: `"60s"`, `"15m"`,
    /// `"1.5h"`, `"2 days"`, `"-1h"`, `"1y"`... A bare number is a number of **seconds**
    /// (`"3600"`). A value that cannot be parsed makes `sign` fail.
    pub expires_in: Option<String>,
    pub algorithm: Algorithm,
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub global: bool,
    /// `privateKey`: PEM key used to sign with `RS*`/`PS*`/`ES*`/`EdDSA`.
    pub private_key: Option<String>,
    /// `publicKey`: PEM key used to verify with `RS*`/`PS*`/`ES*`/`EdDSA`.
    pub public_key: Option<String>,
}

impl JwtModuleOptions {
    pub fn new(secret: impl Into<String>) -> Self {
        JwtModuleOptions {
            secret: secret.into(),
            expires_in: None,
            algorithm: Algorithm::HS256,
            issuer: None,
            audience: None,
            global: false,
            private_key: None,
            public_key: None,
        }
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
    /// `privateKey` (PEM) for asymmetric algorithms.
    pub fn private_key(mut self, pem: impl Into<String>) -> Self {
        self.private_key = Some(pem.into());
        self
    }
    /// `publicKey` (PEM) for asymmetric algorithms.
    pub fn public_key(mut self, pem: impl Into<String>) -> Self {
        self.public_key = Some(pem.into());
        self
    }
}

/// Per-call sign options (`jwtService.sign(payload, { expiresIn, secret, privateKey })`).
#[derive(Debug, Clone, Default)]
pub struct JwtSignOptions {
    pub expires_in: Option<String>,
    pub secret: Option<String>,
    pub subject: Option<String>,
    pub private_key: Option<String>,
}

/// Parses a timespan with the grammar of the `ms` package and returns milliseconds:
/// an optional `-`, a decimal number (`1`, `1.5`, `.5`), optional spaces and an optional
/// case-insensitive unit (`ms`, `s`, `m`, `h`, `d`, `w`, `y` and their long forms such as
/// `msecs`, `seconds`, `mins`, `hrs`, `days`, `weeks`, `years`). Without a unit the number is
/// taken as seconds (Corvis keeps its documented behaviour; `ms` would read milliseconds).
fn parse_timespan_ms(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() || s.len() > 100 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut i = usize::from(bytes[0] == b'-');
    let int_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - int_start;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac_start {
            return None;
        }
        digits += i - frac_start;
    }
    if digits == 0 {
        return None;
    }
    let value: f64 = s[..i].parse().ok()?;
    let unit = s[i..].trim_start_matches(' ').to_ascii_lowercase();
    let mult = match unit.as_str() {
        "" => 1_000.0,
        "milliseconds" | "millisecond" | "msecs" | "msec" | "ms" => 1.0,
        "seconds" | "second" | "secs" | "sec" | "s" => 1_000.0,
        "minutes" | "minute" | "mins" | "min" | "m" => 60_000.0,
        "hours" | "hour" | "hrs" | "hr" | "h" => 3_600_000.0,
        "days" | "day" | "d" => 86_400_000.0,
        "weeks" | "week" | "w" => 604_800_000.0,
        "years" | "year" | "yrs" | "yr" | "y" => 31_557_600_000.0,
        _ => return None,
    };
    let ms = value * mult;
    ms.is_finite().then_some(ms)
}

/// Largest `exp` accepted: `Number.MAX_SAFE_INTEGER`, so the claim survives a JSON round trip.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Parses `"60s"`, `"15m"`, `"1.5h"`, `"2 days"`, `"-1h"`, `"1y"` (the `ms` grammar) or a
/// number of seconds, and returns whole seconds (rounded down, like node-jsonwebtoken).
/// `None` when the value cannot be parsed or is out of range.
pub fn parse_duration_secs(s: &str) -> Option<i64> {
    let secs = (parse_timespan_ms(s)? / 1000.0).floor();
    (secs.abs() <= MAX_SAFE_INTEGER).then_some(secs as i64)
}

const EXPIRES_IN_ERROR: &str = "\"expiresIn\" should be a number of seconds or string representing a timespan eg: \"1d\", \"20h\", 60";

fn encoding_key(alg: Algorithm, key: &str) -> crate::Result<EncodingKey> {
    let pem = key.as_bytes();
    Ok(match alg {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => EncodingKey::from_secret(pem),
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 | Algorithm::PS256 | Algorithm::PS384 | Algorithm::PS512 => {
            EncodingKey::from_rsa_pem(pem)?
        }
        Algorithm::ES256 | Algorithm::ES384 => EncodingKey::from_ec_pem(pem)?,
        Algorithm::EdDSA => EncodingKey::from_ed_pem(pem)?,
    })
}

fn decoding_key(alg: Algorithm, key: &str) -> crate::Result<DecodingKey> {
    let pem = key.as_bytes();
    Ok(match alg {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => DecodingKey::from_secret(pem),
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 | Algorithm::PS256 | Algorithm::PS384 | Algorithm::PS512 => {
            DecodingKey::from_rsa_pem(pem)?
        }
        Algorithm::ES256 | Algorithm::ES384 => DecodingKey::from_ec_pem(pem)?,
        Algorithm::EdDSA => DecodingKey::from_ed_pem(pem)?,
    })
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

    /// Fails like node-jsonwebtoken when the key is empty (`secretOrPrivateKey must have a
    /// value`) or `expires_in` is not a valid timespan.
    pub fn sign_with(&self, payload: &impl Serialize, opts: JwtSignOptions) -> crate::Result<String> {
        let alg = self.options.algorithm;
        // `@nestjs/jwt`: `options.secret || this.options.secret || options.privateKey || this.options.privateKey`.
        let key = [opts.secret.as_deref(), Some(self.options.secret.as_str()), opts.private_key.as_deref(), self.options.private_key.as_deref()]
            .into_iter()
            .flatten()
            .find(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("secretOrPrivateKey must have a value"))?
            .to_string();
        let mut claims = serde_json::to_value(payload)?;
        if let Value::Object(map) = &mut claims {
            let now = chrono::Utc::now().timestamp();
            map.entry("iat").or_insert(Value::from(now));
            if let Some(expires_in) = opts.expires_in.as_deref().or(self.options.expires_in.as_deref()) {
                let secs = parse_duration_secs(expires_in).ok_or_else(|| anyhow::anyhow!(EXPIRES_IN_ERROR))?;
                let exp = now.checked_add(secs).filter(|e| (*e as f64).abs() <= MAX_SAFE_INTEGER);
                let exp = exp.ok_or_else(|| anyhow::anyhow!(EXPIRES_IN_ERROR))?;
                map.insert("exp".into(), Value::from(exp));
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
        // node-jsonwebtoken (jws) writes the header as `{"alg":..,"typ":"JWT"}`, in that order;
        // the `jsonwebtoken` crate serializes `typ` first, so the header is built by hand.
        use base64::Engine as _;
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let header = format!("{{\"alg\":\"{alg:?}\",\"typ\":\"JWT\"}}");
        let message = format!("{}.{}", b64(header.as_bytes()), b64(&serde_json::to_vec(&claims)?));
        let signature = jsonwebtoken::crypto::sign(message.as_bytes(), &encoding_key(alg, &key)?, alg)?;
        Ok(format!("{message}.{signature}"))
    }

    /// `await jwtService.signAsync(payload)`.
    pub async fn sign_async(&self, payload: &impl Serialize) -> crate::Result<String> {
        self.sign(payload)
    }

    /// `jwtService.verify(token)`: validates the signature (only with the configured
    /// algorithm), then, like node-jsonwebtoken, `nbf` (`jwt not active`), `exp` (`jwt
    /// expired` once `now >= exp`), `iss`/`aud` when configured, and that `exp`, `nbf` and
    /// `iat` are numbers when present.
    pub fn verify<T: DeserializeOwned>(&self, token: &str) -> crate::Result<T> {
        let alg = self.options.algorithm;
        // `@nestjs/jwt`: `this.options.secret || this.options.publicKey`.
        let key = [Some(self.options.secret.as_str()), self.options.public_key.as_deref()]
            .into_iter()
            .flatten()
            .find(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("secret or public key must be provided"))?;
        let mut validation = Validation::new(alg);
        validation.required_spec_claims = HashSet::new();
        validation.leeway = 0;
        // `exp` and `nbf` are checked below with node-jsonwebtoken's rules.
        validation.validate_exp = false;
        validation.validate_nbf = false;
        if let Some(iss) = &self.options.issuer {
            validation.set_issuer(&[iss]);
        }
        match &self.options.audience {
            Some(aud) => validation.set_audience(&[aud]),
            None => validation.validate_aud = false,
        }
        let data = jsonwebtoken::decode::<Value>(token, &decoding_key(alg, key)?, &validation)?;
        let claims = data.claims;
        let now = chrono::Utc::now().timestamp() as f64;
        let number = |name: &str| -> crate::Result<Option<f64>> {
            match claims.get(name) {
                None => Ok(None),
                Some(v) => v.as_f64().map(Some).ok_or_else(|| anyhow::anyhow!("invalid {name} value")),
            }
        };
        number("iat")?;
        if let Some(nbf) = number("nbf")?
            && nbf > now {
                anyhow::bail!("jwt not active");
            }
        if let Some(exp) = number("exp")?
            && now >= exp {
                anyhow::bail!("jwt expired");
            }
        Ok(serde_json::from_value(claims)?)
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

    fn forge(header: &str, claims: &Value, secret: &[u8]) -> String {
        use base64::Engine as _;
        let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        let message = format!("{}.{}", b64(header.as_bytes()), b64(claims.to_string().as_bytes()));
        let key = EncodingKey::from_secret(secret);
        let sig = jsonwebtoken::crypto::sign(message.as_bytes(), &key, Algorithm::HS256).unwrap();
        format!("{message}.{sig}")
    }
    const HS256: &str = r#"{"alg":"HS256","typ":"JWT"}"#;

    /// node-jsonwebtoken: `secretOrPrivateKey must have a value` / `secret or public key must be provided`.
    #[test]
    fn empty_secret_is_rejected() {
        let jwt = JwtService::new(JwtModuleOptions::new(""));
        assert!(jwt.sign(&serde_json::json!({ "sub": 1 })).is_err());
        let opts = JwtSignOptions { secret: Some(String::new()), ..Default::default() };
        assert!(jwt.sign_with(&serde_json::json!({ "sub": 1 }), opts).is_err());
        // As in `@nestjs/jwt`, an empty per-call secret falls back to the module secret.
        let ok = JwtService::new(JwtModuleOptions::new("s"));
        let opts = JwtSignOptions { secret: Some(String::new()), ..Default::default() };
        let token = ok.sign_with(&serde_json::json!({ "sub": 1 }), opts).unwrap();
        assert!(ok.verify::<Value>(&token).is_ok());
        let forged = forge(HS256, &serde_json::json!({ "username": "admin" }), b"");
        assert!(jwt.verify::<Value>(&forged).is_err());
    }

    /// `expires_in` follows the `ms` grammar; anything else is an error, never an eternal token.
    #[test]
    fn expires_in_uses_the_ms_grammar() {
        assert_eq!(parse_duration_secs("1.5h"), Some(5400));
        assert_eq!(parse_duration_secs("2.5 days"), Some(216_000));
        assert_eq!(parse_duration_secs("-1h"), Some(-3600));
        assert_eq!(parse_duration_secs(".5m"), Some(30));
        assert_eq!(parse_duration_secs("15mins"), Some(900));
        assert_eq!(parse_duration_secs("1 Year"), Some(31_557_600));
        assert_eq!(parse_duration_secs("100ms"), Some(0));
        assert_eq!(parse_duration_secs("3600"), Some(3600));
        assert_eq!(parse_duration_secs("1d"), Some(86_400));
        assert_eq!(parse_duration_secs("8h"), Some(28_800));
        for bad in ["1h30m", "", "abc", "1.h", "+1h", "1e3s", "99999999999999999999999y"] {
            assert_eq!(parse_duration_secs(bad), None, "{bad}");
        }
        let jwt = JwtService::new(JwtModuleOptions::new("s3cr3t").expires_in("1h30m"));
        assert!(jwt.sign(&serde_json::json!({ "sub": 1 })).is_err());
        let jwt = JwtService::new(JwtModuleOptions::new("s3cr3t").expires_in("1.5h"));
        let claims: Value = jwt.verify(&jwt.sign(&serde_json::json!({ "sub": 1 })).unwrap()).unwrap();
        assert_eq!(claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(), 5400);
        let opts = JwtSignOptions { expires_in: Some("-1h".into()), ..Default::default() };
        let expired = jwt.sign_with(&serde_json::json!({ "sub": 1 }), opts).unwrap();
        assert!(jwt.verify::<Value>(&expired).is_err());
        let opts = JwtSignOptions { expires_in: Some("forever".into()), ..Default::default() };
        assert!(jwt.sign_with(&serde_json::json!({ "sub": 1 }), opts).is_err());
    }

    /// `nbf` in the future and non-numeric `exp`/`nbf`/`iat` are rejected, like node-jsonwebtoken.
    #[test]
    fn registered_claims_are_validated() {
        let jwt = JwtService::new(JwtModuleOptions::new("k"));
        let now = chrono::Utc::now().timestamp();
        let check = |claims: Value| jwt.verify::<Value>(&forge(HS256, &claims, b"k"));
        assert!(check(serde_json::json!({ "sub": 1, "nbf": now + 3600 })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "nbf": now - 10 })).is_ok());
        assert!(check(serde_json::json!({ "sub": 1, "exp": "1" })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "exp": -1 })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "exp": null })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "exp": now })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "exp": now + 60 })).is_ok());
        assert!(check(serde_json::json!({ "sub": 1, "nbf": "0" })).is_err());
        assert!(check(serde_json::json!({ "sub": 1, "iat": "now" })).is_err());
    }

    /// `publicKey` / `privateKey` for RS256, as in `@nestjs/jwt`.
    #[test]
    fn asymmetric_keys() {
        let jwt = JwtService::new(
            JwtModuleOptions::new("")
                .algorithm(Algorithm::RS256)
                .private_key(TEST_RSA_PRIVATE)
                .public_key(TEST_RSA_PUBLIC)
                .expires_in("1h"),
        );
        let token = jwt.sign(&serde_json::json!({ "sub": 7 })).unwrap();
        assert!(token.starts_with("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9."));
        let claims: Value = jwt.verify(&token).unwrap();
        assert_eq!(claims["sub"], 7);
        // An HS256 token "signed" with the public key is not accepted (no algorithm confusion).
        let confused = forge(HS256, &serde_json::json!({ "sub": 7 }), TEST_RSA_PUBLIC.as_bytes());
        assert!(jwt.verify::<Value>(&confused).is_err());
    }

    const TEST_RSA_PRIVATE: &str = include_str!("../tests/fixtures/jwt_rsa_private.pem");
    const TEST_RSA_PUBLIC: &str = include_str!("../tests/fixtures/jwt_rsa_public.pem");
}
