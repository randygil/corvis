//! The JWT strategy: the port of `PassportStrategy(Strategy)` of `passport-jwt` over `jsonwebtoken` 9.
//!
//! A [`JwtValidator`] provides the options and the `validate(request, payload)` of the original class; registering
//! `Provider::strategy::<JwtStrategy<MyValidator>>()` makes it available to `AuthGuard::new("jwt")`.
//!
//! ```ignore
//! #[injectable]
//! pub struct JwtAuth { config: Arc<ConfigService> }
//!
//! #[async_trait]
//! impl JwtValidator for JwtAuth {
//!     type User = Claims;
//!
//!     fn options(&self) -> JwtStrategyOptions {
//!         JwtStrategyOptions::new()
//!             .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
//!             .secret_or_key(JwtKey::secret(self.config.get("JWT_SECRET").unwrap_or_default()))
//!     }
//!
//!     async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Claims>, HttpException> {
//!         Ok(serde_json::from_value(payload).ok())
//!     }
//! }
//!
//! #[module(providers = [JwtAuth::provider(), Provider::strategy::<JwtStrategy<JwtAuth>>()])]
//! pub struct AuthModule;
//! ```
//!
//! Where this differs from `passport-jwt` (each one is on purpose, and the docs list them): a token without `exp` is
//! refused unless [`allow_missing_exp`](JwtStrategyOptions::allow_missing_exp) is set, an `iat` that is not a number
//! is refused, a payload that is not an object is refused, header names of
//! [`from_header`](ExtractJwt::from_header) are case-insensitive, a token longer than
//! [`max_token_length`](JwtStrategyOptions::max_token_length) is refused, clock skew is bounded and an HMAC secret
//! under 32 bytes does not start.

mod extract;
mod key;
mod options;

use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt;
use futures::future::BoxFuture;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, errors::ErrorKind};
use serde::Serialize;
use serde_json::{Value, json};

pub use extract::{ExtractJwt, JwtExtractor};
pub use key::{JwtKey, JwtKeyProvider};
pub use options::{DEFAULT_MAX_TOKEN_LENGTH, JwtStrategyOptions, MAX_CLOCK_TOLERANCE, MIN_SECRET_LENGTH, StringList};

pub use crate::jwt::JwtAlgorithm;

use self::key::allowed_for;
use self::options::{KeySource, Resolved, check_secret};
use super::{AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome};
use crate::common::context::ExecutionContext;
use crate::di::{DiError, Hooks, Injectable, ResolveCtx};
use crate::http::{HttpException, Request};
use crate::jwt::{HeaderProblem, VerifyError, audience_matches, check_header, has_three_segments, issuer_matches, verify_claims};
use crate::logger::Logger;

/// The class a `PassportStrategy(Strategy)` of `passport-jwt` is written as: the options and `validate`.
/// `validate` runs only for a token that was extracted and verified; the payload is a JSON object.
#[async_trait]
pub trait JwtValidator: Injectable {
    /// The name routes use: `AuthGuard::new("jwt")`. Set another one to have several JWT strategies.
    const NAME: &'static str = "jwt";

    /// The user handed to the handler (`User<Self::User>`).
    type User: Serialize + Clone + Send + Sync + 'static;

    /// The options, read once when the application starts.
    fn options(&self) -> JwtStrategyOptions;

    /// `validate(payload)`: the user for a verified payload, `None` to refuse it (401), or an exception (passed on
    /// as it is).
    async fn validate(&self, req: &Request, payload: Value) -> Result<Option<Self::User>, HttpException>;

    /// The auth info of a successful attempt (`Request::auth_info`), none by default.
    fn info(&self, _user: &Self::User) -> Option<Value> {
        None
    }
}

/// The strategy of a [`JwtValidator`]: register it with `Provider::strategy::<JwtStrategy<V>>()`.
pub struct JwtStrategy<V: JwtValidator> {
    validator: Arc<V>,
    resolved: Resolved,
    _marker: PhantomData<fn() -> V>,
}

/// Prints nothing but the strategy name.
impl<V: JwtValidator> std::fmt::Debug for JwtStrategy<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JwtStrategy({})", V::NAME)
    }
}

impl<V: JwtValidator> Injectable for JwtStrategy<V> {
    fn construct(ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
        async move {
            let validator = ctx.get_or_construct::<V>().await?;
            let resolved = validator.options().resolve().map_err(|problem| DiError::Factory {
                token: format!("JwtStrategy \"{}\" ({})", V::NAME, std::any::type_name::<V>()),
                cause: Arc::new(anyhow::anyhow!("invalid JWT strategy options: {problem}")),
            })?;
            Ok(JwtStrategy { validator, resolved, _marker: PhantomData })
        }
        .boxed()
    }

    fn hooks() -> Hooks<Self> {
        Hooks {
            on_module_init: Some(|strategy| {
                async move {
                    let log = Logger::new("JwtStrategy");
                    for warning in &strategy.resolved.warnings {
                        log.warn(format!("JWT strategy \"{}\": {warning}", V::NAME));
                    }
                    Ok(())
                }
                .boxed()
            }),
            ..Hooks::default()
        }
    }
}

fn fail(reason: &str) -> StrategyOutcome {
    StrategyOutcome::Fail { challenge: Some(json!({ "message": reason })), status: None }
}

/// node-jsonwebtoken's message for a decoding error. It never repeats anything from the token.
fn decode_reason(kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::InvalidSignature => "invalid signature",
        ErrorKind::InvalidAlgorithm | ErrorKind::InvalidAlgorithmName | ErrorKind::MissingAlgorithm | ErrorKind::InvalidKeyFormat => {
            "invalid algorithm"
        }
        ErrorKind::InvalidToken | ErrorKind::Base64(_) | ErrorKind::Json(_) | ErrorKind::Utf8(_) => "jwt malformed",
        _ => "invalid token",
    }
}

/// `jsonwebtoken`'s `Validation` for `algorithms`: the crate checks only the signature and the algorithm, the claims
/// are checked by the strategy with node-jsonwebtoken's rules.
fn validation_for(algorithms: Vec<Algorithm>) -> Validation {
    let mut validation = Validation::new(algorithms[0]);
    validation.algorithms = algorithms;
    validation.required_spec_claims = Default::default();
    validation.leeway = 0;
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    validation
}

impl Resolved {
    /// Verifies a token and returns its payload, or the reason it was refused.
    fn verify(&self, token: &str, key: &DecodingKey, allowed: Vec<Algorithm>) -> Result<Value, &'static str> {
        if allowed.is_empty() {
            return Err("invalid algorithm");
        }
        let claims = verify_claims(token, key, &validation_for(allowed), &self.rules).map_err(|e| match e {
            VerifyError::Decode(e) => decode_reason(e.kind()),
            VerifyError::Claim(message) => message,
        })?;
        if !claims.is_object() {
            return Err("jwt payload must be an object");
        }
        if let Some(audience) = &self.audience
            && !audience_matches(claims.get("aud"), audience)
        {
            return Err("jwt audience invalid");
        }
        if let Some(issuer) = &self.issuer
            && !issuer_matches(claims.get("iss"), issuer)
        {
            return Err("jwt issuer invalid");
        }
        Ok(claims)
    }

    /// The payload of a verified token, with the key chosen by the options.
    async fn payload(&self, req: &Request, token: &str, log: &Logger) -> Result<Value, &'static str> {
        match &self.key {
            KeySource::Static { key, allowed, .. } => self.verify(token, key, allowed.clone()),
            KeySource::Provider(provider) => {
                let jwt_key = match provider.key(req, token).await {
                    Ok(k) => k,
                    Err(e) => {
                        log.debug(format!("the key provider failed: {e}"));
                        return Err("key provider error");
                    }
                };
                if let Err(problem) = check_secret(&jwt_key) {
                    log.debug(format!("the key provider returned a key that is not accepted: {problem}"));
                    return Err("key provider error");
                }
                let decoding = jwt_key.decoding_key().map_err(|problem| {
                    log.debug(format!("the key provider returned a key that is not accepted: {problem}"));
                    "key provider error"
                })?;
                self.verify(token, &decoding, allowed_for(jwt_key.family(), self.algorithms.as_deref()))
            }
        }
    }
}

#[async_trait]
impl<V: JwtValidator> Strategy for JwtStrategy<V> {
    fn name(&self) -> &str {
        V::NAME
    }

    fn credential_query_params(&self) -> Vec<String> {
        self.resolved.extractor.query_names()
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _options: &AuthenticateOptions) -> StrategyOutcome {
        let req = ctx.request();
        let Some(token) = self.resolved.extractor.extract(req) else { return fail("No auth token") };
        if token.len() > self.resolved.max_token_length {
            return fail("token too long");
        }
        if !has_three_segments(&token) {
            return fail("jwt malformed");
        }
        match check_header(&token) {
            Some(HeaderProblem::Crit) => return fail("unsupported critical header"),
            Some(HeaderProblem::Malformed) => return fail("jwt malformed"),
            None => {}
        }
        let log = Logger::new("JwtStrategy");
        let payload = match self.resolved.payload(req, &token, &log).await {
            Ok(p) => p,
            Err(reason) => return fail(reason),
        };
        match self.validator.validate(req, payload).await {
            Err(e) => StrategyOutcome::Error(e),
            Ok(None) => StrategyOutcome::Fail { challenge: None, status: None },
            Ok(Some(user)) => {
                let info = self.validator.info(&user);
                match AuthenticatedUser::new(user) {
                    Ok(user) => StrategyOutcome::Success { user, info },
                    Err(e) => StrategyOutcome::Error(e),
                }
            }
        }
    }
}
