//! The per-request store of request-scoped providers.
//!
//! `router::dispatch` installs a [`RequestScopeSlot`] in the extensions of the request it handles and removes it when
//! it returns, panics or is cancelled. The instances live in the slot's [`RequestScope`], so they are released with
//! the request and never reach the container.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::OnceCell;

use super::container::ModuleId;
use super::error::DiError;
use super::provider::Instance;
use super::token::Token;
use crate::http::Request;

type Key = (ModuleId, Token);

/// The request-scoped instances of one request, one lazily filled cell per (module, provider).
#[derive(Default)]
pub(crate) struct RequestScope {
    cells: Mutex<HashMap<Key, Arc<OnceCell<Instance>>>>,
}

impl RequestScope {
    /// The instance for `key`, built by `init` the first time. Two concurrent callers of one key run `init` once and
    /// both get its result; a failed `init` leaves the cell empty so the next caller tries again.
    pub(crate) async fn get_or_try_init<F, Fut>(&self, key: Key, init: F) -> Result<Instance, DiError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Instance, DiError>>,
    {
        let cell = self.cells.lock().unwrap().entry(key).or_default().clone();
        cell.get_or_try_init(init).await.cloned()
    }
}

/// The extension a request carries while it is being dispatched.
#[derive(Clone)]
pub(crate) struct RequestScopeSlot(pub(crate) Arc<RequestScope>);

/// A request together with its store, handed to the providers resolved for it.
#[derive(Clone)]
pub(crate) struct RequestCtx {
    pub(crate) request: Request,
    pub(crate) scope: Arc<RequestScope>,
}

impl RequestCtx {
    /// The context of a request that is being dispatched, `None` for any other request.
    pub(crate) fn of(request: &Request) -> Option<RequestCtx> {
        let slot = request.get::<RequestScopeSlot>()?;
        Some(RequestCtx { request: request.clone(), scope: slot.0 })
    }
}

/// Installs a store in a request for the duration of its dispatch and removes it on drop (response, error, panic or
/// cancellation), which also breaks the `Request -> store -> instance -> Request` cycle a request-scoped provider
/// holding its request would create.
pub(crate) struct RequestScopeGuard {
    request: Option<Request>,
}

impl RequestScopeGuard {
    /// Installs a store unless the request already has one (a nested dispatch shares the outer request's instances;
    /// only the installer removes it).
    pub(crate) fn install(request: &Request) -> RequestScopeGuard {
        let mut extensions = request.extensions.write().unwrap_or_else(|e| e.into_inner());
        if extensions.get::<RequestScopeSlot>().is_some() {
            return RequestScopeGuard { request: None };
        }
        extensions.insert(RequestScopeSlot(Arc::new(RequestScope::default())));
        RequestScopeGuard { request: Some(request.clone()) }
    }
}

impl Drop for RequestScopeGuard {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            // A poisoned lock means a panic while the extensions were held; the store must go either way.
            let mut extensions = request.extensions.write().unwrap_or_else(|e| e.into_inner());
            extensions.remove::<RequestScopeSlot>();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn key() -> Key {
        (0, Token::named("K"))
    }

    fn instance() -> Instance {
        Arc::new(Arc::new(1u8))
    }

    #[tokio::test]
    async fn two_concurrent_callers_of_one_key_run_the_constructor_once() {
        let scope = RequestScope::default();
        let runs = AtomicUsize::new(0);
        let init = || async {
            runs.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            Ok(instance())
        };
        let (a, b) = tokio::join!(scope.get_or_try_init(key(), init), scope.get_or_try_init(key(), init));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&a.unwrap(), &b.unwrap()));
    }

    #[tokio::test]
    async fn a_failed_attempt_is_retried() {
        let scope = RequestScope::default();
        let runs = AtomicUsize::new(0);
        let failing = scope.get_or_try_init(key(), || async {
            runs.fetch_add(1, Ordering::SeqCst);
            Err(DiError::Other("boom".into()))
        });
        assert!(failing.await.is_err());
        let ok = scope
            .get_or_try_init(key(), || async {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok(instance())
            })
            .await;
        assert!(ok.is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn different_keys_get_different_instances() {
        let scope = RequestScope::default();
        let a = scope.get_or_try_init((0, Token::named("A")), || async { Ok(instance()) }).await.unwrap();
        let b = scope.get_or_try_init((0, Token::named("B")), || async { Ok(instance()) }).await.unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn dropping_the_dispatch_future_removes_the_slot() {
        let req = Request::empty();
        let task = tokio::spawn({
            let req = req.clone();
            async move {
                let _guard = RequestScopeGuard::install(&req);
                std::future::pending::<()>().await;
            }
        });
        for _ in 0..200 {
            if RequestCtx::of(&req).is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(RequestCtx::of(&req).is_some());
        task.abort();
        let _ = task.await;
        assert!(RequestCtx::of(&req).is_none(), "a cancelled dispatch must release the store");
    }

    #[test]
    fn a_panic_removes_the_slot() {
        let req = Request::empty();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = RequestScopeGuard::install(&req);
            panic!("handler panicked");
        }));
        assert!(result.is_err());
        assert!(RequestCtx::of(&req).is_none());
    }

    #[test]
    fn the_guard_installs_and_removes_the_slot_and_a_nested_guard_keeps_the_outer_one() {
        let req = Request::empty();
        assert!(RequestCtx::of(&req).is_none());
        let outer = RequestScopeGuard::install(&req);
        let scope = RequestCtx::of(&req).unwrap().scope;
        {
            let _inner = RequestScopeGuard::install(&req);
            assert!(Arc::ptr_eq(&RequestCtx::of(&req).unwrap().scope, &scope));
        }
        assert!(RequestCtx::of(&req).is_some(), "only the installer removes the slot");
        drop(outer);
        assert!(RequestCtx::of(&req).is_none());
    }
}
