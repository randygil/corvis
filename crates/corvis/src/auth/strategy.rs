//! The strategy contract: what a strategy receives and the five ways an attempt can end.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::common::context::ExecutionContext;
use crate::http::{HttpException, Request};

/// A named way to authenticate a request, the equivalent of a `passport-strategy` subclass. Register it with
/// [`Provider::strategy`](crate::di::Provider::strategy); an [`AuthGuard`](super::AuthGuard) runs it by name.
///
/// ```ignore
/// pub struct HeaderStrategy { users: Arc<UsersService> }
///
/// #[async_trait]
/// impl Strategy for HeaderStrategy {
///     fn name(&self) -> &str { "header" }
///
///     async fn authenticate(&self, ctx: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
///         let Some(id) = ctx.request().header("x-user") else {
///             return StrategyOutcome::Fail { challenge: None, status: None };
///         };
///         match self.users.find(id).await.map(AuthenticatedUser::new) {
///             Some(Ok(user)) => StrategyOutcome::Success { user, info: None },
///             Some(Err(e)) => StrategyOutcome::Error(e),
///             None => StrategyOutcome::Fail { challenge: None, status: None },
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait Strategy: Send + Sync + 'static {
    /// The name routes use (`AuthGuard::new("jwt")`). Two strategies with the same name stop startup.
    fn name(&self) -> &str;

    /// Tries to authenticate the request of `ctx`. `ctx.switch_to_http()` gives the strategy
    /// [`append_response_header`](crate::common::context::HttpArgumentsHost::append_response_header) for cookies it
    /// has to set.
    async fn authenticate(&self, ctx: &ExecutionContext, options: &AuthenticateOptions) -> StrategyOutcome;

    /// The query parameters this strategy reads credentials from, none by default. They are collected once when the
    /// application starts, and `LoggingInterceptor` and [`Request::redacted_url`] hide their values on every
    /// request, whether or not the strategy runs for it. A strategy that reads a credential from the URL must list
    /// it here.
    fn credential_query_params(&self) -> Vec<String> {
        Vec::new()
    }
}

/// How an authentication attempt ended, the five calls a passport strategy can make.
pub enum StrategyOutcome {
    /// `this.success(user, info)`.
    Success { user: AuthenticatedUser, info: Option<Value> },
    /// `this.fail(challenge, status)`: not authenticated, the next strategy of a chain may still succeed.
    Fail { challenge: Option<Value>, status: Option<u16> },
    /// `this.redirect(url, status)`: end the request with a redirect (302 unless `status` is set). The URL is sent
    /// as given, so build it from trusted data.
    Redirect { url: String, status: Option<u16> },
    /// `this.pass()`: let the request through without a user.
    Pass,
    /// `this.error(err)`: stop and answer with the exception.
    Error(HttpException),
}

/// Prints the variant only: a user, a challenge or an exception can hold credentials or personal data.
impl std::fmt::Debug for StrategyOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StrategyOutcome::Success { .. } => "StrategyOutcome::Success { .. }",
            StrategyOutcome::Fail { .. } => "StrategyOutcome::Fail { .. }",
            StrategyOutcome::Redirect { .. } => "StrategyOutcome::Redirect { .. }",
            StrategyOutcome::Pass => "StrategyOutcome::Pass",
            StrategyOutcome::Error(_) => "StrategyOutcome::Error(..)",
        })
    }
}

/// Strategy specific options of one guard, the options object `passport.authenticate(name, options)` hands to
/// `strategy.authenticate(req, options)`. The core reads none of them.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct AuthenticateOptions {
    /// The options as a JSON object.
    pub options: Map<String, Value>,
}

impl AuthenticateOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The option `key`, if the guard set it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.options.get(key)
    }
}

/// The user a strategy authenticated. Its JSON form is serialized once, when the strategy builds it, and the typed
/// value is kept so a handler can read it back with a [`User<T>`](super::User) parameter. The typed value lives in a
/// slot of its own on the request, so no other extension of the same type can pass for a user.
#[derive(Clone)]
pub struct AuthenticatedUser {
    json: Value,
    typed: Arc<dyn Any + Send + Sync>,
}

/// Does not print the user: it is personal data and `Debug` output ends up in logs.
impl std::fmt::Debug for AuthenticatedUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthenticatedUser { .. }")
    }
}

/// The typed user of the request, set by `AuthGuard` and read only by `User<T>`.
#[derive(Clone)]
pub(crate) struct AuthUserSlot(pub(crate) Arc<dyn Any + Send + Sync>);

impl AuthUserSlot {
    pub(crate) fn read<T: Clone + 'static>(req: &Request) -> Option<T> {
        req.get::<AuthUserSlot>().and_then(|slot| slot.0.downcast_ref::<T>().cloned())
    }
}

impl AuthenticatedUser {
    /// Wraps `user`. A user that cannot be serialized to JSON is an internal error (500 `Internal server error`):
    /// the request must not continue with a typed user and no JSON one.
    pub fn new<T: Serialize + Clone + Send + Sync + 'static>(user: T) -> Result<Self, HttpException> {
        let json = serde_json::to_value(&user).map_err(HttpException::internal)?;
        Ok(AuthenticatedUser { json, typed: Arc::new(user) })
    }

    /// The typed user, when it is a `T`.
    #[cfg(feature = "auth-session")]
    pub(crate) fn typed<T: 'static>(&self) -> Option<&T> {
        self.typed.downcast_ref::<T>()
    }

    /// The JSON form of the user (`Request::user`).
    pub fn json(&self) -> &Value {
        &self.json
    }

    /// `true` when the user is an empty value: JSON `null`, `false`, the number `0` (also `-0` and `0.0`) or the empty
    /// string, what a JavaScript `!user` test rejects. Such a user never authenticates a request: the
    /// [`Authenticator`](super::Authenticator) turns it into a failure of the strategy and nothing stores it. A
    /// strategy whose users can legitimately be one of these wraps them in a struct.
    pub fn is_empty(&self) -> bool {
        is_empty_json(&self.json)
    }

    /// Stores the user on the request: the typed value and the JSON user. An empty user is not stored (`false`).
    pub(crate) fn install(&self, req: &Request) -> bool {
        if self.is_empty() {
            return false;
        }
        req.insert(AuthUserSlot(self.typed.clone()));
        req.set_user_value(self.json.clone());
        true
    }
}

/// `true` for the values a JavaScript `!value` test rejects, the ones that never stand for a user: `null`, `false`,
/// the number `0` (also `-0` and `0.0`) and the empty string.
pub(crate) fn is_empty_json(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

/// What the last authentication attempt of the request produced (`req.authInfo`), see [`Request::auth_info`].
#[derive(Clone, Debug)]
pub struct AuthInfo(pub(crate) Value);

impl AuthInfo {
    /// Sets the request's auth info, or clears it when there is none.
    pub(crate) fn store(req: &Request, info: Option<Value>) {
        match info {
            Some(v) => req.insert(AuthInfo(v)),
            None => {
                req.remove::<AuthInfo>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::ser::Error;

    use super::*;

    #[derive(Clone, serde::Serialize, PartialEq, Debug)]
    struct Payload {
        id: u32,
    }

    #[derive(Clone)]
    struct Broken;

    impl Serialize for Broken {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(S::Error::custom("no json"))
        }
    }

    #[test]
    fn a_user_is_installed_as_a_typed_value_and_as_json() {
        let req = Request::empty();
        let user = AuthenticatedUser::new(Payload { id: 7 }).unwrap();
        assert_eq!(user.json(), &serde_json::json!({ "id": 7 }));
        user.install(&req);
        assert_eq!(AuthUserSlot::read::<Payload>(&req), Some(Payload { id: 7 }));
        assert_eq!(req.get::<Payload>(), None, "the typed user is not a plain request extension");
        assert_eq!(req.user(), Some(serde_json::json!({ "id": 7 })));
    }

    #[test]
    fn a_user_that_does_not_serialize_is_an_internal_error() {
        let e = AuthenticatedUser::new(Broken).unwrap_err();
        assert_eq!(e.status().as_u16(), 500);
        assert_eq!(e.get_response(), &serde_json::json!({ "statusCode": 500, "message": "Internal server error" }));
    }

    #[test]
    fn every_outcome_can_be_built() {
        let user = AuthenticatedUser::new(1u8).unwrap();
        let outcomes = [
            StrategyOutcome::Success { user, info: Some(serde_json::json!("x")) },
            StrategyOutcome::Fail { challenge: Some(serde_json::json!("c")), status: Some(400) },
            StrategyOutcome::Redirect { url: "/x".into(), status: None },
            StrategyOutcome::Pass,
            StrategyOutcome::Error(crate::http::UnauthorizedException::empty()),
        ];
        assert_eq!(outcomes.len(), 5);
    }

    #[test]
    fn debug_output_does_not_print_the_user() {
        let user = AuthenticatedUser::new(Payload { id: 424242 }).unwrap();
        assert!(!format!("{user:?}").contains("424242"));
        let outcome = StrategyOutcome::Success { user, info: Some(serde_json::json!("secret-info")) };
        let printed = format!("{outcome:?}");
        assert!(!printed.contains("424242") && !printed.contains("secret-info"), "{printed}");
        let fail = StrategyOutcome::Fail { challenge: Some(serde_json::json!("secret-challenge")), status: None };
        assert!(!format!("{fail:?}").contains("secret-challenge"));
    }

    #[test]
    fn auth_info_is_stored_and_cleared() {
        let req = Request::empty();
        AuthInfo::store(&req, Some(serde_json::json!({ "scope": "read" })));
        assert_eq!(req.auth_info(), Some(serde_json::json!({ "scope": "read" })));
        AuthInfo::store(&req, None);
        assert_eq!(req.auth_info(), None);
    }
}
