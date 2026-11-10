//! Logging in and out through the session: `Authenticator::log_in` and `log_out`, the port of `req.logIn` and
//! `req.logOut` of `passport` 0.7.0.

use serde_json::{Map, Value};

use super::middleware::SessionUser;
use super::session::{PASSPORT_KEY, Session, USER_KEY};
use crate::auth::authenticator::internal;
use crate::auth::{AuthenticatedUser, Authenticator};
use crate::http::{HttpException, Request};

/// `true` for the values a JavaScript `!value` test rejects, the ones that never stand for a user: `null`, `false`,
/// the number `0` and the empty string.
pub(crate) fn is_empty_json(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

impl Session {
    /// A login: a new identifier (the old record is destroyed when the request ends), the data of before dropped
    /// unless `keep`, and `passport.user` set to `stored`.
    fn log_in(&self, stored: Value, keep: bool, from_guard: bool) {
        let mut s = self.state();
        if from_guard {
            // Only the first login of the request is remembered: that is the state a rejection goes back to.
            if s.before_guard_login.is_none() {
                let mut before = s.clone();
                before.before_guard_login = None;
                s.before_guard_login = Some(Box::new(before));
            }
        } else {
            // A login the handler made is the handler's decision: it stands whatever the status.
            s.before_guard_login = None;
        }
        let previous = std::mem::take(&mut s.data);
        s.regenerate(super::store::now_ms());
        if keep {
            // `passport` merges the previous session into the new one, then sets its own key.
            s.data = previous;
        } else {
            s.fresh_login = true;
        }
        let mut passport = match s.data.shift_remove(PASSPORT_KEY) {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };
        passport.insert(USER_KEY.to_string(), stored);
        s.data.insert(PASSPORT_KEY.to_string(), Value::Object(passport));
    }

    /// A logout: `passport.user` removed, a new identifier (the old record is destroyed when the request ends) and the
    /// data of before dropped unless `keep`. A session that has no record yet needs no new identifier.
    fn log_out(&self, keep: bool) {
        let mut s = self.state();
        if let Some(Value::Object(passport)) = s.data.get_mut(PASSPORT_KEY) {
            passport.shift_remove(USER_KEY);
        }
        if s.persisted {
            let previous = std::mem::take(&mut s.data);
            s.regenerate(super::store::now_ms());
            if keep {
                s.data = previous;
            }
        } else if !keep {
            s.data.clear();
        }
    }
}

/// What a login or logout needs and does not have.
fn unsupported(what: &str) -> HttpException {
    internal(format!("{what}: the request has no session; import SessionModule and register a SessionStrategy"))
}

impl Authenticator {
    /// `req.logIn(user)`: stores `user` in the session of the request so later requests carry it, and makes it the
    /// request's user.
    ///
    /// It generates a new session identifier (the old record is destroyed when the request ends), so an identifier
    /// planted before the login is worthless (session fixation), and drops the data of before the login unless
    /// `keep_session_info_on_login` is set. The user is stored through the application's `SessionSerializer`.
    ///
    /// Fails with a 500 and a fixed message, storing nothing, when the request has no session, no `SessionStrategy`
    /// is registered, or the user is empty (JSON `null`, `false`, `0` or `""`: such a user can never authenticate).
    /// An `AuthGuard` with `.log_in()` calls it after a successful authentication.
    ///
    /// A login made here, by a handler, stands whatever the response is: the handler decides. A login made by an
    /// `AuthGuard` with `.log_in()` is kept only when the pipeline does not reject the request; see
    /// [`AuthGuard::log_in`](crate::auth::AuthGuard::log_in).
    pub async fn log_in(&self, req: &Request, user: &AuthenticatedUser) -> Result<(), HttpException> {
        self.log_in_with(req, user, false).await
    }

    /// The login of an `AuthGuard`: like [`log_in`](Self::log_in), but undone at commit if the request is rejected.
    pub(crate) async fn log_in_from_guard(&self, req: &Request, user: &AuthenticatedUser) -> Result<(), HttpException> {
        self.log_in_with(req, user, true).await
    }

    async fn log_in_with(&self, req: &Request, user: &AuthenticatedUser, from_guard: bool) -> Result<(), HttpException> {
        let Some(session) = req.get::<Session>() else { return Err(unsupported("cannot log in")) };
        let manager = session.manager().clone();
        let Some(codec) = manager.codec() else { return Err(unsupported("cannot log in")) };
        if user.is_empty() {
            return Err(internal("cannot log in an empty user (null, false, 0 or \"\")"));
        }
        let stored = match tokio::time::timeout(manager.options.store_timeout, codec.serialize(user)).await {
            Ok(result) => result?,
            Err(_) => return Err(manager.fault("the session user serializer did not answer in time", None)),
        };
        // What is stored must come back as a user: a value a JavaScript `!value` test rejects never does.
        if is_empty_json(&stored) {
            return Err(internal("the session serializer returned an empty value (null, false, 0 or \"\") for the user"));
        }
        session.log_in(stored, manager.options.keep_session_info_on_login, from_guard);
        user.install(req);
        req.insert(SessionUser(user.clone()));
        Ok(())
    }

    /// `req.logOut()`: removes the user from the session, generates a new identifier (the old record is destroyed
    /// when the request ends), drops the data of before unless `keep_session_info_on_login` is set, and clears the
    /// request's user. A 500 with a fixed message when the request has no session or no `SessionStrategy` is
    /// registered.
    pub async fn log_out(&self, req: &Request) -> Result<(), HttpException> {
        let Some(session) = req.get::<Session>() else { return Err(unsupported("cannot log out")) };
        if session.manager().codec().is_none() {
            return Err(unsupported("cannot log out"));
        }
        session.log_out(session.manager().options.keep_session_info_on_login);
        req.clear_user();
        req.remove::<SessionUser>();
        Ok(())
    }
}
