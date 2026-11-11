//! `Authenticator`: runs named strategies in order, the port of `passport.authenticate(names, options, callback)`.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use futures::FutureExt;
use serde_json::Value;

use super::strategy::{AuthInfo, AuthenticateOptions, AuthenticatedUser, Strategy, StrategyOutcome};
use crate::common::context::ExecutionContext;
use crate::http::{HttpException, Request, Response};

/// The strategies of the application by name, plus the configured default names.
#[derive(Default)]
pub(crate) struct StrategyRegistry {
    pub(crate) by_name: HashMap<String, Arc<dyn Strategy>>,
    /// Set by `PassportModule::register`, `None` when no default strategy is configured.
    pub(crate) default: Option<Vec<String>>,
}

/// A failed attempt of one strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthFailure {
    /// The name of the strategy that failed.
    pub strategy: String,
    pub challenge: Option<Value>,
    pub status: Option<u16>,
}

/// How [`Authenticator::authenticate`] ended, without an error.
pub enum AuthResult {
    /// A strategy succeeded. The user is not stored on the request: see [`AuthResult::store_user`].
    Success { user: AuthenticatedUser, info: Option<Value> },
    /// Every strategy failed (in order).
    Failed { failures: Vec<AuthFailure> },
    /// A strategy passed: no user, the route may run.
    Pass,
    /// A strategy redirected: the response is stored with `respond_with` and the router sends it.
    Redirected,
}

/// Prints the variant only (a user and an info value can hold personal data).
impl std::fmt::Debug for AuthResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AuthResult::Success { .. } => "AuthResult::Success { .. }",
            AuthResult::Failed { failures } => return write!(f, "AuthResult::Failed {{ failures: {} }}", failures.len()),
            AuthResult::Pass => "AuthResult::Pass",
            AuthResult::Redirected => "AuthResult::Redirected",
        })
    }
}

impl AuthResult {
    /// Stores the user of a [`AuthResult::Success`] on the request (typed value and JSON user, as `AuthGuard` does)
    /// and returns `true`; any other result stores nothing, and neither does a success with an empty user (see
    /// [`AuthenticatedUser::is_empty`]), which the authenticator never returns.
    pub fn store_user(&self, req: &Request) -> bool {
        match self {
            AuthResult::Success { user, .. } => user.install(req),
            _ => false,
        }
    }
}

#[derive(Debug)]
struct AuthError(String);

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AuthError {}

pub(crate) fn internal(message: impl Into<String>) -> HttpException {
    HttpException::internal(AuthError(message.into()))
}

/// The registry of strategies. A core provider: inject it with `Arc<Authenticator>` or fetch it with
/// `ModuleRef::get::<Authenticator>()`.
///
/// It never stores the user of a successful attempt: the original integration assigns `req.user` in the guard, after
/// `handleRequest`, and so does [`AuthGuard`](super::AuthGuard). A guard of your own does what it wants with the
/// [`AuthResult`] ([`AuthResult::store_user`] stores the user).
///
/// It has no public constructor, so a module cannot shadow the core provider with an empty one.
pub struct Authenticator {
    registry: OnceLock<StrategyRegistry>,
    /// Limits the warning about a strategy that panics: a client can trigger it on every request.
    panics: super::warn_limit::WarnLimiter,
    /// Limits the warning about a strategy that succeeds with an empty user: a client can trigger it on every request.
    empty_users: super::warn_limit::WarnLimiter,
}

/// The reason in the auth info of a strategy that succeeded with an empty user. Fixed: it repeats nothing.
const EMPTY_USER: &str = "The strategy returned an empty user";

impl Authenticator {
    pub(crate) fn new() -> Self {
        let limiter = || super::warn_limit::WarnLimiter::new(Duration::from_secs(60));
        Authenticator { registry: OnceLock::new(), panics: limiter(), empty_users: limiter() }
    }

    /// The text of the warning about a strategy that panicked at `now`, at most once a minute per strategy. The panic
    /// payload is never read: it can hold request data.
    fn panic_warning(&self, name: &str, now: Instant) -> Option<String> {
        self.panics.line(name, now, || format!("the strategy \"{name}\" panicked; the request was answered with a 500"))
    }

    /// The text of the warning about a strategy that returned an empty user, at most once a minute per strategy.
    fn empty_user_warning(&self, name: &str, now: Instant) -> Option<String> {
        self.empty_users.line(name, now, || {
            format!(
                "the strategy \"{name}\" returned an empty user (null, false, 0 or \"\") and the attempt was treated as a failure; a validator that finds no user must return None"
            )
        })
    }

    fn warn_panic(&self, name: &str) {
        if let Some(line) = self.panic_warning(name, Instant::now()) {
            crate::logger::Logger::new("Authenticator").warn(line);
        }
    }

    fn warn_empty_user(&self, name: &str) {
        if let Some(line) = self.empty_user_warning(name, Instant::now()) {
            crate::logger::Logger::new("Authenticator").warn(line);
        }
    }

    pub(crate) fn set_registry(&self, registry: StrategyRegistry) {
        let _ = self.registry.set(registry);
    }

    pub(crate) fn registry(&self) -> Option<&StrategyRegistry> {
        self.registry.get()
    }

    /// `true` when a strategy is registered under `name`.
    pub fn has_strategy(&self, name: &str) -> bool {
        self.registry().is_some_and(|r| r.by_name.contains_key(name))
    }

    /// The default strategy names configured with `PassportModule::register`, if any.
    pub fn default_strategies(&self) -> Option<Vec<String>> {
        self.default_names().map(<[String]>::to_vec)
    }

    /// [`default_strategies`](Self::default_strategies), borrowed.
    pub(crate) fn default_names(&self) -> Option<&[String]> {
        self.registry().and_then(|r| r.default.as_deref())
    }

    /// Runs the strategies called `names` in order against the request of `ctx`:
    ///
    /// - the first success wins, and its info becomes the request's auth info;
    /// - a failure moves on to the next strategy;
    /// - an error stops the chain and is returned;
    /// - a pass ends it without touching the auth info;
    /// - a redirect validates the URL as a header value, stores the response with `respond_with` and ends it;
    /// - when all failed, the auth info is the challenge of the failure (one name) or the list of challenges, `null`
    ///   where a strategy gave none (several names).
    ///
    /// A name that is not registered is an internal error (500), as `Unknown authentication strategy "x"` is in the
    /// original; it is raised when the chain reaches that name.
    pub async fn authenticate(
        &self,
        ctx: &ExecutionContext,
        names: &[String],
        options: &AuthenticateOptions,
    ) -> Result<AuthResult, HttpException> {
        let Some(registry) = self.registry() else {
            return Err(internal("the strategy registry is not initialized: call init() before handling requests"));
        };
        let mut failures: Vec<AuthFailure> = Vec::new();
        for name in names {
            let Some(strategy) = registry.by_name.get(name) else {
                return Err(internal(format!("Unknown authentication strategy \"{name}\"")));
            };
            // A panic inside a strategy is an error, never a failure: an optional guard lets a failed attempt
            // through, and a panic must not. The router would also answer a 500, but only for the whole request;
            // here it ends the chain too.
            let outcome = match AssertUnwindSafe(strategy.authenticate(ctx, options)).catch_unwind().await {
                Ok(outcome) => outcome,
                Err(_) => {
                    self.warn_panic(name);
                    // The warning above is the log line, rate limited; the default filter does not add one per request.
                    return Err(internal(format!("the authentication strategy \"{name}\" panicked")).already_logged());
                }
            };
            match outcome {
                // A user that a `!user` test rejects in the original never authenticates: it is a failure of this
                // strategy, and the chain goes on as for any other.
                StrategyOutcome::Success { user, .. } if user.is_empty() => {
                    self.warn_empty_user(name);
                    let challenge = Some(serde_json::json!({ "message": EMPTY_USER }));
                    failures.push(AuthFailure { strategy: name.clone(), challenge, status: None });
                }
                StrategyOutcome::Success { user, info } => {
                    AuthInfo::store(ctx.request(), info.clone());
                    return Ok(AuthResult::Success { user, info });
                }
                StrategyOutcome::Fail { challenge, status } => {
                    failures.push(AuthFailure { strategy: name.clone(), challenge, status });
                }
                StrategyOutcome::Error(e) => return Err(e),
                StrategyOutcome::Pass => return Ok(AuthResult::Pass),
                StrategyOutcome::Redirect { url, status } => {
                    let response = redirect_response(&url, status)?;
                    if !ctx.switch_to_http().respond_with(response) {
                        return Err(internal("a strategy redirected outside an HTTP request"));
                    }
                    return Ok(AuthResult::Redirected);
                }
            }
        }
        let info = if names.len() > 1 {
            Some(Value::Array(failures.iter().map(|f| f.challenge.clone().unwrap_or(Value::Null)).collect()))
        } else {
            failures.first().and_then(|f| f.challenge.clone())
        };
        AuthInfo::store(ctx.request(), info);
        Ok(AuthResult::Failed { failures })
    }
}

/// What passport's `strategy.redirect(url, status)` writes: the status (302 by default), `Location` as given,
/// `Content-Length: 0` and no body. Node's `res.setHeader` throws for a URL with a character it refuses, which is a
/// 500 here.
fn redirect_response(url: &str, status: Option<u16>) -> Result<Response, HttpException> {
    let Some((_, location)) = crate::common::context::validate_response_header("location", url) else {
        return Err(internal("a strategy redirected to a URL that is not a valid header value"));
    };
    let status = status.unwrap_or(302);
    if !(300..=399).contains(&status) {
        return Err(internal(format!("a strategy redirected with the status code {status}, which is not a redirect (3xx)")));
    }
    let mut response = Response::new().with_status(status);
    response.headers_mut().insert(http::header::LOCATION, location);
    response.headers_mut().insert(http::header::CONTENT_LENGTH, http::HeaderValue::from_static("0"));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::common::context::Target;
    use crate::http::UnauthorizedException;

    /// Answers the outcome it was built with and counts its runs.
    struct Fake {
        name: &'static str,
        outcome: Mutex<Option<StrategyOutcome>>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Strategy for Fake {
        fn name(&self) -> &str {
            self.name
        }
        async fn authenticate(&self, _: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcome.lock().unwrap().take().expect("a fake strategy runs once")
        }
    }

    fn authenticator(strategies: Vec<(&'static str, StrategyOutcome)>) -> (Authenticator, Vec<Arc<AtomicUsize>>) {
        let mut registry = StrategyRegistry::default();
        let mut counters = Vec::new();
        for (name, outcome) in strategies {
            let calls = Arc::new(AtomicUsize::new(0));
            counters.push(calls.clone());
            registry.by_name.insert(name.to_string(), Arc::new(Fake { name, outcome: Mutex::new(Some(outcome)), calls }));
        }
        let a = Authenticator::new();
        a.set_registry(registry);
        (a, counters)
    }

    /// A context as the router builds it while a guard runs (the guard phase is on).
    fn ctx() -> ExecutionContext {
        let c = bare_ctx();
        crate::common::context::GuardPhase::install(c.request()).set(true);
        c
    }

    fn bare_ctx() -> ExecutionContext {
        ExecutionContext::new(Request::empty(), Arc::new(Target::new("C")), Arc::new(Target::new("h")), None)
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn fail(challenge: Option<Value>) -> StrategyOutcome {
        StrategyOutcome::Fail { challenge, status: Some(400) }
    }

    fn success(info: Option<Value>) -> StrategyOutcome {
        StrategyOutcome::Success { user: AuthenticatedUser::new(json!({ "id": 1 })).unwrap(), info }
    }

    async fn run(a: &Authenticator, c: &ExecutionContext, list: &[&str]) -> Result<AuthResult, HttpException> {
        a.authenticate(c, &names(list), &AuthenticateOptions::new()).await
    }

    #[tokio::test]
    async fn success_returns_the_user_and_sets_the_auth_info() {
        let (a, _) = authenticator(vec![("a", success(Some(json!({ "scope": "read" }))))]);
        let c = ctx();
        let r = run(&a, &c, &["a"]).await.unwrap();
        assert!(matches!(r, AuthResult::Success { .. }));
        assert_eq!(c.request().auth_info(), Some(json!({ "scope": "read" })));
        assert!(c.request().user().is_none(), "the authenticator does not store the user");
        assert!(r.store_user(c.request()));
        assert_eq!(c.request().user(), Some(json!({ "id": 1 })));
    }

    #[tokio::test]
    async fn the_first_success_wins_and_later_strategies_do_not_run() {
        let (a, calls) = authenticator(vec![("a", fail(None)), ("b", success(None)), ("c", success(None))]);
        let c = ctx();
        assert!(matches!(run(&a, &c, &["a", "b", "c"]).await.unwrap(), AuthResult::Success { .. }));
        let runs: Vec<usize> = calls.iter().map(|n| n.load(Ordering::SeqCst)).collect();
        assert_eq!(runs, [1, 1, 0]);
    }

    #[tokio::test]
    async fn an_error_stops_the_chain() {
        let (a, calls) = authenticator(vec![("a", StrategyOutcome::Error(UnauthorizedException::new("no"))), ("b", success(None))]);
        let e = run(&a, &ctx(), &["a", "b"]).await.unwrap_err();
        assert_eq!(e.get_status(), 401);
        assert_eq!(calls[1].load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn pass_ends_the_chain_without_touching_the_auth_info() {
        let (a, calls) = authenticator(vec![("a", StrategyOutcome::Pass), ("b", success(None))]);
        let c = ctx();
        AuthInfo::store(c.request(), Some(json!("earlier")));
        assert!(matches!(run(&a, &c, &["a", "b"]).await.unwrap(), AuthResult::Pass));
        assert_eq!(calls[1].load(Ordering::SeqCst), 0);
        assert_eq!(c.request().auth_info(), Some(json!("earlier")));
    }

    #[tokio::test]
    async fn a_redirect_stores_the_response_and_ends_the_chain() {
        let (a, calls) = authenticator(vec![
            ("a", StrategyOutcome::Redirect { url: "https://idp.example/authorize?x=1".into(), status: None }),
            ("b", success(None)),
        ]);
        let c = ctx();
        assert!(matches!(run(&a, &c, &["a", "b"]).await.unwrap(), AuthResult::Redirected));
        assert_eq!(calls[1].load(Ordering::SeqCst), 0);
        let stored = crate::common::context::PendingResponse::take(c.request()).expect("a stored response");
        assert_eq!(stored.status_code(), 302);
        assert_eq!(stored.headers().get("location").unwrap(), "https://idp.example/authorize?x=1");
        assert_eq!(stored.headers().get("content-length").unwrap(), "0");
        assert!(matches!(stored.body(), crate::http::response::Body::Empty));
    }

    #[tokio::test]
    async fn a_redirect_status_outside_3xx_is_an_internal_error() {
        for status in [200u16, 401, 500, 299, 400, 0, 1000] {
            let (a, _) = authenticator(vec![("a", StrategyOutcome::Redirect { url: "/x".into(), status: Some(status) })]);
            let c = ctx();
            let e = run(&a, &c, &["a"]).await.unwrap_err();
            assert!(e.is_internal(), "{status}");
            assert!(crate::common::context::PendingResponse::take(c.request()).is_none(), "{status}");
        }
    }

    #[test]
    fn results_debug_does_not_print_the_user() {
        let user = AuthenticatedUser::new(json!({ "ssn": "424242" })).unwrap();
        let r = AuthResult::Success { user, info: Some(json!("secret-info")) };
        let printed = format!("{r:?}");
        assert!(!printed.contains("424242") && !printed.contains("secret-info"), "{printed}");
    }

    #[tokio::test]
    async fn a_redirect_outside_the_guard_phase_is_an_internal_error() {
        let (a, _) = authenticator(vec![("a", StrategyOutcome::Redirect { url: "/x".into(), status: None })]);
        let c = bare_ctx();
        let e = run(&a, &c, &["a"]).await.unwrap_err();
        assert!(e.is_internal());
        assert!(crate::common::context::PendingResponse::take(c.request()).is_none());
    }

    #[tokio::test]
    async fn a_redirect_keeps_its_own_status() {
        let (a, _) = authenticator(vec![("a", StrategyOutcome::Redirect { url: "/x".into(), status: Some(301) })]);
        let c = ctx();
        run(&a, &c, &["a"]).await.unwrap();
        assert_eq!(crate::common::context::PendingResponse::take(c.request()).unwrap().status_code(), 301);
    }

    #[tokio::test]
    async fn a_redirect_url_with_a_line_break_is_an_internal_error() {
        let (a, _) = authenticator(vec![("a", StrategyOutcome::Redirect { url: "/a\r\nSet-Cookie: x=1".into(), status: None })]);
        let c = ctx();
        let e = run(&a, &c, &["a"]).await.unwrap_err();
        assert!(e.is_internal());
        assert!(crate::common::context::PendingResponse::take(c.request()).is_none());
    }

    #[tokio::test]
    async fn the_auth_info_of_one_failure_is_its_challenge() {
        let (a, _) = authenticator(vec![("a", fail(Some(json!({ "message": "Missing credentials" }))))]);
        let c = ctx();
        let AuthResult::Failed { failures } = run(&a, &c, &["a"]).await.unwrap() else { panic!("expected a failure") };
        assert_eq!(
            failures,
            [AuthFailure { strategy: "a".into(), challenge: Some(json!({ "message": "Missing credentials" })), status: Some(400) }]
        );
        assert_eq!(c.request().auth_info(), Some(json!({ "message": "Missing credentials" })));
    }

    #[tokio::test]
    async fn the_auth_info_of_several_failures_is_the_list_of_challenges() {
        let (a, _) = authenticator(vec![("a", fail(Some(json!("no token")))), ("b", fail(None))]);
        let c = ctx();
        run(&a, &c, &["a", "b"]).await.unwrap();
        assert_eq!(c.request().auth_info(), Some(json!(["no token", null])));
    }

    #[tokio::test]
    async fn an_unknown_name_is_an_internal_error_when_the_chain_reaches_it() {
        let (a, _) = authenticator(vec![("a", fail(None))]);
        let e = run(&a, &ctx(), &["a", "typo"]).await.unwrap_err();
        assert!(e.is_internal());
        assert!(e.message().contains("Unknown authentication strategy \"typo\""), "{}", e.message());
        // Reached only after "a" succeeded: never looked up.
        let (a, _) = authenticator(vec![("a", success(None))]);
        assert!(matches!(run(&a, &ctx(), &["a", "typo"]).await.unwrap(), AuthResult::Success { .. }));
    }

    struct Panics;

    #[async_trait]
    impl Strategy for Panics {
        fn name(&self) -> &str {
            "panics"
        }
        async fn authenticate(&self, _: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
            panic!("secret-credential-in-a-panic-message")
        }
    }

    #[tokio::test]
    async fn a_panicking_strategy_is_a_500_that_ends_the_chain() {
        let calls = [Arc::new(AtomicUsize::new(0))];
        let mut registry = StrategyRegistry::default();
        registry.by_name.insert("panics".into(), Arc::new(Panics));
        registry
            .by_name
            .insert("after".into(), Arc::new(Fake { name: "after", outcome: Mutex::new(Some(success(None))), calls: calls[0].clone() }));
        let a = Authenticator::new();
        a.set_registry(registry);
        let c = ctx();
        let e = run(&a, &c, &["panics", "after"]).await.unwrap_err();
        assert!(e.is_internal());
        assert_eq!(e.get_response(), &json!({ "statusCode": 500, "message": "Internal server error" }));
        assert!(!e.message().contains("secret-credential"), "{}", e.message());
        assert_eq!(calls[0].load(Ordering::SeqCst), 0, "the chain stops at the panic");
        assert!(c.request().user().is_none());
    }

    fn user_outcome(user: Value) -> StrategyOutcome {
        StrategyOutcome::Success { user: AuthenticatedUser::new(user).unwrap(), info: Some(json!("info of the empty user")) }
    }

    #[test]
    fn exactly_the_values_a_not_test_rejects_are_empty_users() {
        let empty = [json!(null), json!(false), json!(0), json!(0.0), json!(-0.0), json!(""), json!(0u64), json!(-0i64)];
        for v in empty {
            assert!(AuthenticatedUser::new(v.clone()).unwrap().is_empty(), "{v}");
        }
        // Truthy in JavaScript too: authenticates.
        let real = [
            json!(true),
            json!(1),
            json!(-1),
            json!(0.1),
            json!(" "),
            json!("0"),
            json!("false"),
            json!("null"),
            json!({}),
            json!([]),
            json!([0]),
            json!({ "id": 0 }),
        ];
        for v in real {
            assert!(!AuthenticatedUser::new(v.clone()).unwrap().is_empty(), "{v}");
        }
        // The types an application writes for `return isValid` and `return repo.find(id)`.
        assert!(AuthenticatedUser::new(false).unwrap().is_empty());
        assert!(AuthenticatedUser::new(Option::<u8>::None).unwrap().is_empty());
        assert!(AuthenticatedUser::new(0u32).unwrap().is_empty());
        assert!(AuthenticatedUser::new(String::new()).unwrap().is_empty());
        assert!(AuthenticatedUser::new(f64::NAN).unwrap().is_empty(), "NaN serializes to null");
        assert!(!AuthenticatedUser::new(Some(7u8)).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_success_with_an_empty_user_is_a_failure_of_that_strategy() {
        for user in [json!(null), json!(false), json!(0), json!(0.0), json!(-0.0), json!("")] {
            let (a, _) = authenticator(vec![("a", user_outcome(user.clone()))]);
            let c = ctx();
            let AuthResult::Failed { failures } = run(&a, &c, &["a"]).await.unwrap() else { panic!("{user}: expected a failure") };
            assert_eq!(failures.len(), 1, "{user}");
            assert_eq!(failures[0].strategy, "a");
            assert_eq!(failures[0].challenge, Some(json!({ "message": "The strategy returned an empty user" })), "{user}");
            // The reason is the auth info; the info of the discarded success is not, and no user is stored.
            assert_eq!(c.request().auth_info(), Some(json!({ "message": "The strategy returned an empty user" })), "{user}");
            assert!(c.request().user().is_none(), "{user}");
        }
    }

    #[tokio::test]
    async fn the_chain_goes_on_after_an_empty_user() {
        let (a, calls) = authenticator(vec![("a", user_outcome(json!(false))), ("b", success(None)), ("c", success(None))]);
        let c = ctx();
        assert!(matches!(run(&a, &c, &["a", "b", "c"]).await.unwrap(), AuthResult::Success { .. }));
        let runs: Vec<usize> = calls.iter().map(|n| n.load(Ordering::SeqCst)).collect();
        assert_eq!(runs, [1, 1, 0]);
        // All of them empty: the auth info lists each reason.
        let (a, _) = authenticator(vec![("a", user_outcome(json!(0))), ("b", user_outcome(json!("")))]);
        let c = ctx();
        assert!(matches!(run(&a, &c, &["a", "b"]).await.unwrap(), AuthResult::Failed { .. }));
        let reason = json!({ "message": "The strategy returned an empty user" });
        assert_eq!(c.request().auth_info(), Some(json!([reason, reason])));
    }

    #[tokio::test]
    async fn a_real_user_that_looks_empty_in_another_language_authenticates() {
        for user in [json!("0"), json!({}), json!([]), json!(true), json!(1), json!({ "id": 0 })] {
            let (a, _) = authenticator(vec![("a", user_outcome(user.clone()))]);
            assert!(matches!(run(&a, &ctx(), &["a"]).await.unwrap(), AuthResult::Success { .. }), "{user}");
        }
    }

    #[test]
    fn nothing_stores_an_empty_user() {
        let c = ctx();
        let result = AuthResult::Success { user: AuthenticatedUser::new(json!(false)).unwrap(), info: None };
        assert!(!result.store_user(c.request()));
        assert!(c.request().user().is_none());
        assert!(crate::auth::AuthUserSlot::read::<Value>(c.request()).is_none());
    }

    #[test]
    fn the_warnings_are_limited_per_strategy_and_count_what_they_suppressed() {
        let a = Authenticator::new();
        let start = Instant::now();
        for (first, again, later) in [
            (
                a.empty_user_warning("s", start),
                a.empty_user_warning("s", start + Duration::from_secs(1)),
                a.empty_user_warning("s", start + Duration::from_secs(61)),
            ),
            (
                a.panic_warning("s", start),
                a.panic_warning("s", start + Duration::from_secs(1)),
                a.panic_warning("s", start + Duration::from_secs(61)),
            ),
        ] {
            let first = first.expect("the first one is written");
            assert!(first.contains("\"s\"") && !first.contains("more since"), "{first}");
            assert!(again.is_none());
            assert!(later.expect("one a minute").contains("(1 more since the last warning)"));
        }
        // Another strategy has its own allowance.
        assert!(a.empty_user_warning("t", start + Duration::from_secs(2)).is_some());
        let line = a.empty_user_warning("u", start).unwrap();
        assert!(line.contains("empty user") && line.contains("None"), "{line}");
        assert!(a.panic_warning("u", start).unwrap().contains("panicked"));
        assert!(!a.panic_warning("v", start).unwrap().contains("secret"));
    }

    #[tokio::test]
    async fn the_panic_error_is_not_logged_again_by_the_default_filter() {
        let mut registry = StrategyRegistry::default();
        registry.by_name.insert("panics".into(), Arc::new(Panics));
        let a = Authenticator::new();
        a.set_registry(registry);
        let e = run(&a, &ctx(), &["panics"]).await.unwrap_err();
        assert!(e.is_internal() && e.is_already_logged());
    }

    #[tokio::test]
    async fn an_uninitialized_registry_is_an_internal_error() {
        let e = run(&Authenticator::new(), &ctx(), &["a"]).await.unwrap_err();
        assert!(e.is_internal());
    }
}
