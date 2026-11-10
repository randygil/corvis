//! The strategy contract: what a strategy receives and the five ways an attempt can end.

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
}

/// How an authentication attempt ended, the five calls a passport strategy can make.
#[derive(Debug)]
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
/// value is kept so a handler can read it back with `Request::get::<T>()` or a [`User<T>`](super::User) parameter.
#[derive(Clone)]
pub struct AuthenticatedUser {
    json: Value,
    install: Arc<dyn Fn(&Request) + Send + Sync>,
}

impl std::fmt::Debug for AuthenticatedUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthenticatedUser").field("json", &self.json).finish()
    }
}

impl AuthenticatedUser {
    /// Wraps `user`. A user that cannot be serialized to JSON is an internal error (500 `Internal server error`):
    /// the request must not continue with a typed user and no JSON one.
    pub fn new<T: Serialize + Clone + Send + Sync + 'static>(user: T) -> Result<Self, HttpException> {
        let json = serde_json::to_value(&user).map_err(HttpException::internal)?;
        let install = Arc::new(move |req: &Request| req.insert(user.clone()));
        Ok(AuthenticatedUser { json, install })
    }

    /// The JSON form of the user (`Request::user`).
    pub fn json(&self) -> &Value {
        &self.json
    }

    /// Stores the user on the request: the typed value and the JSON user.
    pub(crate) fn install(&self, req: &Request) {
        (self.install)(req);
        req.set_user_value(self.json.clone());
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
        assert_eq!(req.get::<Payload>(), Some(Payload { id: 7 }));
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
    fn auth_info_is_stored_and_cleared() {
        let req = Request::empty();
        AuthInfo::store(&req, Some(serde_json::json!({ "scope": "read" })));
        assert_eq!(req.auth_info(), Some(serde_json::json!({ "scope": "read" })));
        AuthInfo::store(&req, None);
        assert_eq!(req.auth_info(), None);
    }
}
