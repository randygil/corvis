//! The work the strategy does once per key and once per token, counted (never timed).

use std::sync::atomic::{AtomicUsize, Ordering};

use jsonwebtoken::Algorithm;

use super::key::KeyFamily;
use super::*;
use crate::common::context::Target;
use crate::jwt::{JwtModuleOptions, JwtService};

const SECRET: &str = "0123456789abcdef0123456789abcdef";
const RSA_PRIVATE: &str = include_str!("../../../tests/fixtures/jwt_rsa_private.pem");
const RSA_PUBLIC: &str = include_str!("../../../tests/fixtures/jwt_rsa_public.pem");

#[derive(Default)]
struct Claims;
crate::default_injectable!(Claims);

#[async_trait]
impl JwtValidator for Claims {
    type User = Value;

    fn options(&self) -> JwtStrategyOptions {
        unreachable!("the tests build the resolved options themselves")
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Value>, HttpException> {
        Ok(Some(json!({ "sub": payload["sub"] })))
    }
}

fn strategy(options: JwtStrategyOptions) -> JwtStrategy<Claims> {
    let resolved = options.resolve().unwrap();
    JwtStrategy { validator: Arc::new(Claims), resolved, warnings: WarnLimiter::new(Duration::from_secs(60)), _marker: PhantomData }
}

fn bearer() -> JwtStrategyOptions {
    JwtStrategyOptions::new().jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
}

async fn run(strategy: &JwtStrategy<Claims>, token: &str) -> StrategyOutcome {
    let mut req = Request::empty();
    req.headers_mut().insert("authorization", format!("Bearer {token}").parse().unwrap());
    let ctx = ExecutionContext::new(req, Arc::new(Target::new("C")), Arc::new(Target::new("h")), None);
    strategy.authenticate(&ctx, &AuthenticateOptions::default()).await
}

fn hs256_token() -> String {
    JwtService::new(JwtModuleOptions::new(SECRET).expires_in("1h")).sign(&json!({ "sub": "7" })).unwrap()
}

fn rs256_token() -> String {
    JwtService::new(JwtModuleOptions::new("").algorithm(Algorithm::RS256).private_key(RSA_PRIVATE).expires_in("1h"))
        .sign(&json!({ "sub": "7" }))
        .unwrap()
}

fn header_checks() -> usize {
    crate::jwt::HEADER_CHECKS.with(|c| c.get())
}

fn validation_builds() -> usize {
    options::VALIDATION_BUILDS.with(|c| c.get())
}

fn key_parses() -> usize {
    key::KEY_PARSES.with(|c| c.get())
}

fn challenge(outcome: &StrategyOutcome) -> Option<Value> {
    match outcome {
        StrategyOutcome::Fail { challenge, .. } => challenge.clone(),
        _ => None,
    }
}

/// A provider that hands back clones of one cached key and counts its calls.
struct Cached {
    key: JwtKey,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl JwtKeyProvider for Cached {
    async fn key(&self, _req: &Request, _raw_token: &str) -> anyhow::Result<JwtKey> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.key.clone())
    }
}

fn cached(key: JwtKey) -> (Cached, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (Cached { key, calls: calls.clone() }, calls)
}

/// Spec "Header checked once": once per well-formed token, and a token with `crit` never reaches the key provider.
#[tokio::test]
async fn the_header_is_checked_once_per_token() {
    let static_key = strategy(bearer().secret_or_key(JwtKey::secret(SECRET)));
    let token = hs256_token();
    let before = header_checks();
    assert!(matches!(run(&static_key, &token).await, StrategyOutcome::Success { .. }));
    assert_eq!(header_checks() - before, 1);

    let (provider, calls) = cached(JwtKey::secret(SECRET));
    let with_provider = strategy(bearer().secret_or_key_provider(provider));
    let before = header_checks();
    assert!(matches!(run(&with_provider, &token).await, StrategyOutcome::Success { .. }));
    assert_eq!(header_checks() - before, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    use base64::Engine as _;
    let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let crit = format!("{}.{}", b64(br#"{"alg":"HS256","crit":["urn:x"],"urn:x":true}"#), token.split_once('.').unwrap().1);
    assert_eq!(challenge(&run(&with_provider, &crit).await), Some(json!({ "message": "unsupported critical header" })));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the provider is not called for a token with crit");
}

/// The validations are built when the strategy is built, one per key family for a provider, narrowed to the family as
/// the per-request narrowing did; a request builds none.
#[tokio::test]
async fn prepared_validations_follow_the_key_family() {
    let (provider, _) = cached(JwtKey::secret(SECRET));
    let resolved = bearer().secret_or_key_provider(provider).algorithms([Algorithm::HS256, Algorithm::RS256]).resolve().unwrap();
    let KeySource::Provider { validations, .. } = &resolved.key else { panic!("a provider") };
    assert_eq!(validations.get(KeyFamily::Hmac).unwrap().algorithms, vec![Algorithm::HS256]);
    assert_eq!(validations.get(KeyFamily::Rsa).unwrap().algorithms, vec![Algorithm::RS256]);
    assert!(validations.get(KeyFamily::Ec).is_none(), "no EC algorithm is allowed");
    assert!(validations.get(KeyFamily::Ed).is_none());

    let (provider, _) = cached(JwtKey::secret(SECRET));
    let resolved = bearer().secret_or_key_provider(provider).resolve().unwrap();
    let KeySource::Provider { validations, .. } = &resolved.key else { panic!("a provider") };
    assert_eq!(validations.get(KeyFamily::Hmac).unwrap().algorithms.len(), 3);
    assert_eq!(validations.get(KeyFamily::Rsa).unwrap().algorithms.len(), 6);
    assert_eq!(validations.get(KeyFamily::Ec).unwrap().algorithms, vec![Algorithm::ES256, Algorithm::ES384]);
    assert!(validations.get(KeyFamily::Ed).is_none(), "Ed25519 needs an explicit list");

    let resolved = bearer().secret_or_key(JwtKey::rsa_pem(RSA_PUBLIC)).algorithms([Algorithm::RS256, Algorithm::PS256]).resolve().unwrap();
    let KeySource::Static { validation, .. } = &resolved.key else { panic!("a static key") };
    assert_eq!(validation.algorithms, vec![Algorithm::RS256, Algorithm::PS256]);

    // An HMAC token never meets an RSA key: the provider's RSA key narrows the list to RSA algorithms.
    let (provider, _) = cached(JwtKey::rsa_pem(RSA_PUBLIC));
    let rsa_provider = strategy(bearer().secret_or_key_provider(provider));
    let builds = validation_builds();
    assert_eq!(challenge(&run(&rsa_provider, &hs256_token()).await), Some(json!({ "message": "invalid algorithm" })));
    assert!(matches!(run(&rsa_provider, &rs256_token()).await, StrategyOutcome::Success { .. }));
    let static_key = strategy(bearer().secret_or_key(JwtKey::secret(SECRET)));
    let builds_after_startup = validation_builds();
    assert_eq!(builds_after_startup - builds, 1, "the static strategy built its one validation at startup");
    for _ in 0..10 {
        assert!(matches!(run(&static_key, &hs256_token()).await, StrategyOutcome::Success { .. }));
    }
    assert_eq!(validation_builds(), builds_after_startup, "no request builds a validation");
}

/// Spec "Cached provider key": a provider that returns clones of one RSA key costs one parse for 100 requests.
#[tokio::test]
async fn a_cached_provider_key_is_parsed_once() {
    let (provider, calls) = cached(JwtKey::rsa_pem(RSA_PUBLIC));
    let strategy = strategy(bearer().secret_or_key_provider(provider));
    let token = rs256_token();
    let before = key_parses();
    for _ in 0..100 {
        assert!(matches!(run(&strategy, &token).await, StrategyOutcome::Success { .. }));
    }
    assert_eq!(key_parses() - before, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 100);
}

/// Spec "Short secret from a cache": the length check still runs on every request, whatever is cached.
#[tokio::test]
async fn a_cached_short_secret_is_refused_on_every_request() {
    let (provider, calls) = cached(JwtKey::secret(vec![b'x'; 31]));
    let strategy = strategy(bearer().secret_or_key_provider(provider));
    let token = hs256_token();
    for _ in 0..3 {
        assert_eq!(challenge(&run(&strategy, &token).await), Some(json!({ "message": "key provider error" })));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}
