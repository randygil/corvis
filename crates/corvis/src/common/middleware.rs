//! Middleware (`CorvisMiddleware`), functional middleware and the `MiddlewareConsumer`.

use std::any::TypeId;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use futures::future::BoxFuture;

use super::enhancer::{MiddlewareRef, middleware_instance, middleware_of};
use crate::di::Injectable;
use crate::http::{HandlerResult, Request, RequestMethod};

/// Middleware is called before the route handler and has access to the request.
///
/// ```ignore
/// #[injectable]
/// pub struct LoggerMiddleware;
///
/// #[async_trait]
/// impl CorvisMiddleware for LoggerMiddleware {
///     async fn use_(&self, req: Request, next: Next) -> HandlerResult {
///         println!("Request...");
///         next.run(req).await
///     }
/// }
/// ```
#[async_trait]
pub trait CorvisMiddleware: Send + Sync + 'static {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult;
}

pub(crate) type Endpoint = Arc<dyn Fn(Request) -> BoxFuture<'static, HandlerResult> + Send + Sync>;

/// The `next()` function: calls the next middleware or the route handler.
pub struct Next {
    chain: Arc<Vec<Arc<dyn CorvisMiddleware>>>,
    index: usize,
    endpoint: Endpoint,
}

impl Next {
    pub(crate) fn new(chain: Arc<Vec<Arc<dyn CorvisMiddleware>>>, endpoint: Endpoint) -> Self {
        Next { chain, index: 0, endpoint }
    }

    pub async fn run(self, req: Request) -> HandlerResult {
        if let Some(mw) = self.chain.get(self.index).cloned() {
            let next = Next { chain: self.chain.clone(), index: self.index + 1, endpoint: self.endpoint.clone() };
            mw.use_(req, next).await
        } else {
            (self.endpoint)(req).await
        }
    }
}

/// Functional middleware: `consumer.apply_fn(|req, next| async move { next.run(req).await })`.
pub struct FnMiddleware<F>(pub F);

#[async_trait]
impl<F, Fut> CorvisMiddleware for FnMiddleware<F>
where
    F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HandlerResult> + Send + 'static,
{
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        (self.0)(req, next).await
    }
}

/// A route matcher for `forRoutes()` / `exclude()`.
#[derive(Debug, Clone)]
pub enum RouteSpec {
    Path { path: String, method: RequestMethod },
    Controller(TypeId),
}

impl RouteSpec {
    /// `forRoutes(CatsController)`.
    pub fn controller<C: 'static>() -> Self {
        RouteSpec::Controller(TypeId::of::<C>())
    }
    pub fn path(path: impl Into<String>, method: RequestMethod) -> Self {
        RouteSpec::Path { path: path.into(), method }
    }
}

/// `{ path: 'cats', method: RequestMethod.GET }`.
#[derive(Debug, Clone)]
pub struct RouteInfo {
    pub path: &'static str,
    pub method: RequestMethod,
}

impl From<RouteInfo> for RouteSpec {
    fn from(r: RouteInfo) -> Self {
        RouteSpec::Path { path: r.path.to_string(), method: r.method }
    }
}
impl From<&str> for RouteSpec {
    fn from(s: &str) -> Self {
        RouteSpec::Path { path: s.to_string(), method: RequestMethod::All }
    }
}
impl From<String> for RouteSpec {
    fn from(s: String) -> Self {
        RouteSpec::Path { path: s, method: RequestMethod::All }
    }
}

/// Anything accepted by `for_routes()` / `exclude()`.
pub trait IntoRouteSpecs {
    fn into_route_specs(self) -> Vec<RouteSpec>;
}
impl IntoRouteSpecs for RouteSpec {
    fn into_route_specs(self) -> Vec<RouteSpec> {
        vec![self]
    }
}
impl IntoRouteSpecs for RouteInfo {
    fn into_route_specs(self) -> Vec<RouteSpec> {
        vec![self.into()]
    }
}
impl IntoRouteSpecs for &str {
    fn into_route_specs(self) -> Vec<RouteSpec> {
        vec![self.into()]
    }
}
impl IntoRouteSpecs for String {
    fn into_route_specs(self) -> Vec<RouteSpec> {
        vec![self.into()]
    }
}
impl<T: Into<RouteSpec>, const N: usize> IntoRouteSpecs for [T; N] {
    fn into_route_specs(self) -> Vec<RouteSpec> {
        self.into_iter().map(Into::into).collect()
    }
}
impl<T: Into<RouteSpec>> IntoRouteSpecs for Vec<T> {
    fn into_route_specs(self) -> Vec<RouteSpec> {
        self.into_iter().map(Into::into).collect()
    }
}

pub(crate) struct MiddlewareConfig {
    pub middleware: Vec<MiddlewareRef>,
    pub include: Vec<RouteSpec>,
    pub exclude: Vec<RouteSpec>,
}

/// Passed to `CorvisModule::configure`.
#[derive(Default)]
pub struct MiddlewareConsumer {
    pub(crate) configs: Vec<MiddlewareConfig>,
}

impl MiddlewareConsumer {
    pub fn new() -> Self {
        Self::default()
    }

    /// `consumer.apply(LoggerMiddleware)` – a middleware class with dependency injection.
    pub fn apply<M: Injectable + CorvisMiddleware>(&mut self) -> MiddlewareConfigProxy<'_> {
        MiddlewareConfigProxy { consumer: self, middleware: vec![middleware_of::<M>()], exclude: Vec::new() }
    }

    /// `consumer.apply(new SomeMiddleware(...))` – an instance.
    pub fn apply_instance<M: CorvisMiddleware>(&mut self, middleware: M) -> MiddlewareConfigProxy<'_> {
        MiddlewareConfigProxy { consumer: self, middleware: vec![middleware_instance(middleware)], exclude: Vec::new() }
    }

    /// `consumer.apply(logger)` – functional middleware.
    pub fn apply_fn<F, Fut>(&mut self, f: F) -> MiddlewareConfigProxy<'_>
    where
        F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        MiddlewareConfigProxy { consumer: self, middleware: vec![middleware_instance(FnMiddleware(f))], exclude: Vec::new() }
    }
}

pub struct MiddlewareConfigProxy<'a> {
    consumer: &'a mut MiddlewareConsumer,
    middleware: Vec<MiddlewareRef>,
    exclude: Vec<RouteSpec>,
}

impl<'a> MiddlewareConfigProxy<'a> {
    /// Adds another middleware class to the same `apply(...)` call.
    pub fn and<M: Injectable + CorvisMiddleware>(mut self) -> Self {
        self.middleware.push(middleware_of::<M>());
        self
    }
    pub fn and_instance<M: CorvisMiddleware>(mut self, m: M) -> Self {
        self.middleware.push(middleware_instance(m));
        self
    }
    pub fn and_fn<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.middleware.push(middleware_instance(FnMiddleware(f)));
        self
    }
    /// `.exclude({ path: 'cats', method: RequestMethod.GET }, 'cats/(.*)')`.
    pub fn exclude(mut self, routes: impl IntoRouteSpecs) -> Self {
        self.exclude.extend(routes.into_route_specs());
        self
    }
    /// `.forRoutes('cats')`, `.forRoutes(RouteSpec::controller::<CatsController>())`, `.forRoutes("*")`.
    pub fn for_routes(self, routes: impl IntoRouteSpecs) -> &'a mut MiddlewareConsumer {
        self.consumer.configs.push(MiddlewareConfig {
            middleware: self.middleware,
            include: routes.into_route_specs(),
            exclude: self.exclude,
        });
        self.consumer
    }
}

/// Express-like path pattern matching used by middleware routes.
/// Supports literals, `:param`, `*` and `(.*)` wildcards.
pub(crate) fn path_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_matches('/');
    let path = path.trim_matches('/');
    if pattern == "*" || pattern == "(.*)" || pattern == "{*path}" || pattern == "*path" {
        return true;
    }
    let pat: Vec<&str> = if pattern.is_empty() { vec![] } else { pattern.split('/').collect() };
    let segs: Vec<&str> = if path.is_empty() { vec![] } else { path.split('/').collect() };
    fn rec(pat: &[&str], segs: &[&str]) -> bool {
        match pat.first() {
            None => segs.is_empty(),
            Some(p) if *p == "*" || *p == "(.*)" || p.starts_with("{*") || (p.starts_with('*') && p.len() > 1) => {
                (0..=segs.len()).any(|i| rec(&pat[1..], &segs[i..]))
            }
            Some(p) => match segs.first() {
                None => false,
                Some(s) => {
                    let ok = if p.starts_with(':') || (p.starts_with('{') && p.ends_with('}')) {
                        true
                    } else if let Some(prefix) = p.strip_suffix('*') {
                        s.starts_with(prefix)
                    } else {
                        p.eq_ignore_ascii_case(s)
                    };
                    ok && rec(&pat[1..], &segs[1..])
                }
            },
        }
    }
    rec(&pat, &segs)
}

#[cfg(test)]
mod tests {
    use super::path_matches;

    #[test]
    fn matching() {
        assert!(path_matches("cats", "/cats"));
        assert!(path_matches("cats", "/cats/"));
        assert!(!path_matches("cats", "/cats/1"));
        assert!(path_matches("cats/*", "/cats/1"));
        assert!(path_matches("cats/(.*)", "/cats/1/2"));
        assert!(path_matches("cats/:id", "/cats/1"));
        assert!(path_matches("*", "/anything/here"));
        assert!(path_matches("api/*", "/api"));
    }
}
