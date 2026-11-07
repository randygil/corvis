//! Routing: route compilation (`RoutesResolver` / `RouterExplorer`), the request
//! pipeline (middleware -> guards -> interceptors -> pipes -> handler -> filters)
//! and the top-level dispatcher.

pub mod controller;
pub mod cors;
pub mod versioning;

use std::any::TypeId;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use http::{Method, StatusCode};
use serde_json::Value;

use crate::common::context::{ExecutionContext, Target};
use crate::common::filter::{BaseExceptionFilter, ExceptionFilter};
use crate::common::guard::CanActivate;
use crate::common::interceptor::{CallHandler, CorvisInterceptor};
use crate::common::middleware::{Endpoint, MiddlewareConsumer, CorvisMiddleware, Next, RouteSpec, path_matches};
use crate::common::pipe::{ParamKind, PipeTransform};
use crate::di::container::Container;
use crate::di::{DiError, ModuleRef, ResolveCtx};
use crate::http::{ForbiddenException, HandlerResult, HttpException, NotFoundException, Request, RequestMethod, Res, ResMode, Response};
use crate::logger::Logger;
use controller::{ErasedHandler, ParamDef, VersionValue};
use cors::CorsOptions;
use versioning::{VersioningOptions, VersioningType};

pub(crate) struct CompiledParam {
    pub def: ParamDef,
    pub pipes: Vec<Arc<dyn PipeTransform>>,
}

pub(crate) struct CompiledRoute {
    pub method: RequestMethod,
    pub path: String,
    pub controller_type: TypeId,
    pub class_target: Arc<Target>,
    pub handler_target: Arc<Target>,
    pub guards: Vec<Arc<dyn CanActivate>>,
    pub interceptors: Arc<Vec<Arc<dyn CorvisInterceptor>>>,
    pub pipes: Vec<Arc<dyn PipeTransform>>,
    pub filters: Vec<Arc<dyn ExceptionFilter>>,
    pub params: Vec<CompiledParam>,
    pub handler: ErasedHandler,
    pub default_status: StatusCode,
    pub headers: Vec<(String, String)>,
    pub redirect: Option<(String, u16)>,
    pub module_ref: ModuleRef,
    pub versions: Option<VersionValue>,
    pub host: Option<String>,
    pub response_mode: Option<crate::http::ResMode>,
}

#[derive(Default)]
pub(crate) struct RouterTable {
    by_method: HashMap<String, matchit::Router<Vec<usize>>>,
    any: matchit::Router<Vec<usize>>,
    pub routes: Vec<Arc<CompiledRoute>>,
}

/// Joins path fragments: `("api", "/v1", "cats/", ":id")` -> `/api/v1/cats/:id`.
pub(crate) fn join_paths(parts: &[&str]) -> String {
    let segs: Vec<&str> = parts.iter().flat_map(|p| p.split('/')).filter(|s| !s.is_empty()).collect();
    if segs.is_empty() { "/".to_string() } else { format!("/{}", segs.join("/")) }
}

/// Converts an Express-style path (`/cats/:id`, `/files/*`) into matchit syntax.
pub(crate) fn to_matchit(path: &str) -> String {
    let mut out = Vec::new();
    for seg in path.split('/') {
        if seg.is_empty() {
            continue;
        }
        if let Some(name) = seg.strip_prefix(':') {
            let name = name.split('(').next().unwrap_or(name).trim_end_matches('?');
            out.push(format!("{{{name}}}"));
        } else if seg == "*" || seg == "(.*)" || seg == "*path" || seg == "{*path}" {
            out.push("{*path}".to_string());
            break;
        } else if let Some(name) = seg.strip_prefix('*') {
            out.push(format!("{{*{name}}}"));
            break;
        } else if seg.starts_with("{*") {
            out.push(seg.to_string());
            break;
        } else {
            out.push(seg.replace('{', "{{").replace('}', "}}"));
        }
    }
    format!("/{}", out.join("/"))
}

/// Express (router 2 / path-to-regexp 8, `strict: false`) accepts exactly one optional
/// trailing slash: `/cats/` matches `/cats`, `/cats//` does not.
fn normalize_request_path(path: &str) -> String {
    let p = if path.len() > 1 { path.strip_suffix('/').unwrap_or(path) } else { path };
    if p.is_empty() { "/".to_string() } else { p.to_string() }
}

/// Lowercases the static parts of a matchit path (`/Cats/{id}` → `/cats/{id}`): routes match
/// case-insensitively, like Express' default (`case sensitive routing` disabled).
fn lowercase_static(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    let mut depth = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') && depth == 0 => {
                chars.next();
                out.push_str("{{");
            }
            '}' if chars.peek() == Some(&'}') && depth == 0 => {
                chars.next();
                out.push_str("}}");
            }
            '{' => {
                depth += 1;
                out.push(c);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                out.push(c);
            }
            c if depth == 0 => out.push(c.to_ascii_lowercase()),
            c => out.push(c),
        }
    }
    out
}

/// `decodeURIComponent` of a route parameter; `Err` carries router's message
/// (`Failed to decode param '%zz'`), answered with 400 like Express.
fn decode_param(v: &str) -> Result<String, String> {
    let fail = || format!("Failed to decode param '{v}'");
    let b = v.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let hex = |i: usize| b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let lead = hex(i).ok_or_else(fail)?;
        i += 3;
        let n = match lead {
            0x00..=0x7F => 1,
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => return Err(fail()),
        };
        let mut seq = vec![lead];
        for _ in 1..n {
            if b.get(i) != Some(&b'%') {
                return Err(fail());
            }
            seq.push(hex(i).ok_or_else(fail)?);
            i += 3;
        }
        std::str::from_utf8(&seq).map_err(|_| fail())?;
        out.extend(seq);
    }
    String::from_utf8(out).map_err(|_| fail())
}

/// Matched route candidates and their decoded parameters (or the decode error message).
type RouteMatch = (Vec<usize>, Result<HashMap<String, String>, String>);

impl RouterTable {
    fn insert(&mut self, groups: Vec<((Option<String>, String), Vec<usize>)>) {
        let logger = Logger::new("RouterExplorer");
        for ((method, path), idx) in groups {
            let router = match &method {
                Some(m) => self.by_method.entry(m.clone()).or_default(),
                None => &mut self.any,
            };
            if let Err(e) = router.insert(lowercase_static(&path), idx) {
                logger.error(format!("Unable to register route {path}: {e}"));
            }
        }
    }

    fn lookup(&self, method: &Method, path: &str) -> Option<RouteMatch> {
        let original = normalize_request_path(path);
        // ASCII lowercasing keeps byte offsets, so parameter values are cut from the original.
        let lowered = original.to_ascii_lowercase();
        let base = lowered.as_ptr() as usize;
        let decode = |m: matchit::Match<'_, '_, &Vec<usize>>| {
            let params = m
                .params
                .iter()
                .map(|(k, v)| {
                    let start = v.as_ptr() as usize - base;
                    decode_param(&original[start..start + v.len()]).map(|v| (k.to_string(), v))
                })
                .collect();
            (m.value.clone(), params)
        };
        if let Some(r) = self.by_method.get(method.as_str()) {
            if let Ok(m) = r.at(&lowered) {
                return Some(decode(m));
            }
        }
        if method == Method::HEAD {
            if let Some(r) = self.by_method.get("GET") {
                if let Ok(m) = r.at(&lowered) {
                    return Some(decode(m));
                }
            }
        }
        self.any.at(&lowered).ok().map(decode)
    }
}

pub(crate) struct CompiledMiddleware {
    pub middleware: Vec<Arc<dyn CorvisMiddleware>>,
    pub include: Vec<(String, RequestMethod)>,
    pub exclude: Vec<(String, RequestMethod)>,
}

impl CompiledMiddleware {
    fn applies(&self, method: &Method, path: &str) -> bool {
        let hit = |list: &[(String, RequestMethod)]| list.iter().any(|(p, m)| m.matches(method) && path_matches(p, path));
        hit(&self.include) && !hit(&self.exclude)
    }
}

/// Global application configuration used to build the router.
#[derive(Clone, Default)]
pub(crate) struct RouterConfig {
    pub global_prefix: String,
    pub prefix_exclude: Vec<String>,
    pub versioning: Option<VersioningOptions>,
    pub global_guards: Vec<Arc<dyn CanActivate>>,
    pub global_interceptors: Vec<Arc<dyn CorvisInterceptor>>,
    pub global_pipes: Vec<Arc<dyn PipeTransform>>,
    pub global_filters: Vec<Arc<dyn ExceptionFilter>>,
}

fn version_paths(versioning: Option<&VersioningOptions>, version: &Option<VersionValue>) -> Vec<Option<String>> {
    match (versioning, version) {
        (Some(v), Some(VersionValue::Versions(list))) if v.kind == VersioningType::Uri => {
            list.iter().map(|x| Some(format!("{}{}", v.prefix, x))).collect()
        }
        _ => vec![None],
    }
}

fn version_label(version: &Option<VersionValue>, versioning: Option<&VersioningOptions>) -> String {
    match (versioning, version) {
        (Some(_), Some(VersionValue::Versions(list))) => format!(" (version: {})", list.join(",")),
        (Some(_), Some(VersionValue::Neutral)) => " (version: Neutral)".to_string(),
        _ => String::new(),
    }
}

async fn resolve_all<T: ?Sized + Send + Sync + 'static>(
    refs: &[crate::common::enhancer::EnhancerRef<T>],
    ctx: &ResolveCtx,
) -> Result<Vec<Arc<T>>, DiError> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        out.push(r.resolve(ctx.clone()).await?);
    }
    Ok(out)
}

/// Builds the routing table from every controller of every module.
pub(crate) async fn build_router(container: &Arc<Container>, cfg: &RouterConfig, log: bool) -> Result<RouterTable, DiError> {
    let resolver_log = Logger::new("RoutesResolver");
    let explorer_log = Logger::new("RouterExplorer");
    let mut table = RouterTable::default();
    let mut groups: Vec<((Option<String>, String), Vec<usize>)> = Vec::new();

    for module in &container.modules {
        for def in &module.controllers {
            let Some(instance) = container.instance(module.id, &def.provider.token) else { continue };
            let cmeta = (def.metadata)();
            let ctx = ResolveCtx::for_module(container.clone(), module.id);
            let c_guards = resolve_all(&cmeta.guards, &ctx).await?;
            let c_interceptors = resolve_all(&cmeta.interceptors, &ctx).await?;
            let c_pipes = resolve_all(&cmeta.pipes, &ctx).await?;
            let c_filters = resolve_all(&cmeta.filters, &ctx).await?;
            let class_target = Arc::new(Target {
                name: def.name.clone(),
                metadata: cmeta.metadata.iter().cloned().collect(),
            });
            let routes = (def.routes)(&instance);
            let c_paths = if cmeta.paths.is_empty() { vec![String::new()] } else { cmeta.paths.clone() };
            let module_ref = ModuleRef { container: container.clone(), module: module.id };

            for c_path in &c_paths {
                let prefix = if cfg.prefix_exclude.iter().any(|e| path_matches(e, c_path)) { "" } else { cfg.global_prefix.as_str() };
                if log {
                    let base_version = cmeta.version.clone().or_else(|| cfg.versioning.as_ref().and_then(|v| v.default_version.clone()));
                    let shown = version_paths(cfg.versioning.as_ref(), &base_version);
                    let p = join_paths(&[prefix, shown[0].as_deref().unwrap_or(""), c_path]);
                    resolver_log.log(format!("{} {{{}}}:{}", def.name, p, version_label(&base_version, cfg.versioning.as_ref())));
                }
                for r in &routes {
                    let meta = &r.meta;
                    let mut guards = cfg.global_guards.clone();
                    guards.extend(c_guards.iter().cloned());
                    guards.extend(resolve_all(&meta.guards, &ctx).await?);
                    let mut interceptors = cfg.global_interceptors.clone();
                    interceptors.extend(c_interceptors.iter().cloned());
                    interceptors.extend(resolve_all(&meta.interceptors, &ctx).await?);
                    let mut pipes = cfg.global_pipes.clone();
                    pipes.extend(c_pipes.iter().cloned());
                    pipes.extend(resolve_all(&meta.pipes, &ctx).await?);
                    let mut filters = resolve_all(&meta.filters, &ctx).await?;
                    filters.extend(c_filters.iter().cloned());
                    filters.extend(cfg.global_filters.iter().cloned());
                    let mut params = Vec::with_capacity(meta.params.len());
                    for p in &meta.params {
                        params.push(CompiledParam { def: p.clone(), pipes: resolve_all(&p.pipes, &ctx).await? });
                    }
                    let handler_target = Arc::new(Target {
                        name: meta.handler_name.to_string(),
                        metadata: meta.metadata.iter().cloned().collect(),
                    });
                    let version = meta
                        .version
                        .clone()
                        .or_else(|| cmeta.version.clone())
                        .or_else(|| cfg.versioning.as_ref().and_then(|v| v.default_version.clone()));
                    // `#[http_code]`, or `__httpCode__` metadata set by a composed decorator
                    // (`applyDecorators(HttpCode(201), ...)`, e.g. `ZodResponse`).
                    let http_code = meta.http_code.or_else(|| {
                        meta.metadata.iter().rev().find(|(k, _)| k == crate::HTTP_CODE_METADATA).and_then(|(_, v)| v.as_u64()).map(|c| c as u16)
                    });
                    let default_status = match http_code {
                        Some(c) => StatusCode::from_u16(c).unwrap_or(StatusCode::OK),
                        None if meta.method == RequestMethod::Post => StatusCode::CREATED,
                        None => StatusCode::OK,
                    };
                    let interceptors = Arc::new(interceptors);
                    for r_path in &meta.paths {
                        for vp in version_paths(cfg.versioning.as_ref(), &version) {
                            let full = join_paths(&[prefix, vp.as_deref().unwrap_or(""), c_path, r_path]);
                            let idx = table.routes.len();
                            table.routes.push(Arc::new(CompiledRoute {
                                method: meta.method,
                                path: full.clone(),
                                controller_type: def.type_id,
                                class_target: class_target.clone(),
                                handler_target: handler_target.clone(),
                                guards: guards.clone(),
                                interceptors: interceptors.clone(),
                                pipes: pipes.clone(),
                                filters: filters.clone(),
                                params: params
                                    .iter()
                                    .map(|p| CompiledParam { def: p.def.clone(), pipes: p.pipes.clone() })
                                    .collect(),
                                handler: r.handler.clone(),
                                default_status,
                                headers: meta.headers.clone(),
                                redirect: meta.redirect.clone(),
                                module_ref: module_ref.clone(),
                                versions: version.clone(),
                                host: cmeta.host.clone(),
                                response_mode: meta.response_mode,
                            }));
                            let key = (meta.method.to_http().map(|m| m.to_string()), to_matchit(&full));
                            match groups.iter_mut().find(|(k, _)| *k == key) {
                                Some((_, v)) => v.push(idx),
                                None => groups.push((key, vec![idx])),
                            }
                            if log {
                                explorer_log.log(format!(
                                    "Mapped {{{}, {}}} route{}",
                                    full,
                                    meta.method.as_str(),
                                    version_label(&version, cfg.versioning.as_ref())
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    table.insert(groups);
    Ok(table)
}

/// Runs every module's `configure(consumer)` and compiles the middleware routes.
pub(crate) async fn build_middleware(
    container: &Arc<Container>,
    table: &RouterTable,
    cfg: &RouterConfig,
) -> Result<Vec<CompiledMiddleware>, DiError> {
    let mut out = Vec::new();
    for module in &container.modules {
        let Some(configure) = module.configure else { continue };
        let Some(token) = &module.module_token else { continue };
        let Some(instance) = container.instance(module.id, token) else { continue };
        let mut consumer = MiddlewareConsumer::new();
        configure(&instance, &mut consumer);
        let ctx = ResolveCtx::for_module(container.clone(), module.id);
        for config in consumer.configs {
            let middleware = resolve_all(&config.middleware, &ctx).await?;
            let compile = |specs: &[RouteSpec]| -> Vec<(String, RequestMethod)> {
                let mut v = Vec::new();
                for s in specs {
                    match s {
                        RouteSpec::Path { path, method } => {
                            let p = path.trim_matches('/');
                            if p == "*" || p == "(.*)" || p == "*path" || p == "{*path}" {
                                v.push(("*".to_string(), *method));
                            } else {
                                v.push((join_paths(&[cfg.global_prefix.as_str(), p]), *method));
                            }
                        }
                        RouteSpec::Controller(tid) => {
                            for r in table.routes.iter().filter(|r| r.controller_type == *tid) {
                                v.push((r.path.clone(), r.method));
                            }
                        }
                    }
                }
                v
            };
            out.push(CompiledMiddleware { middleware, include: compile(&config.include), exclude: compile(&config.exclude) });
        }
    }
    Ok(out)
}

/// Shared state used by the HTTP server.
pub(crate) struct AppShared {
    pub router: RouterTable,
    pub global_middleware: Vec<Arc<dyn CorvisMiddleware>>,
    pub module_middleware: Vec<CompiledMiddleware>,
    pub global_filters: Vec<Arc<dyn ExceptionFilter>>,
    pub cors: Option<CorsOptions>,
    pub versioning: Option<VersioningOptions>,
    pub body_limit: usize,
    pub static_dirs: Vec<Arc<crate::modules::serve_static::ServeStaticOptions>>,
    #[allow(dead_code)]
    pub container: Arc<Container>,
    #[cfg(feature = "websockets")]
    pub gateways: HashMap<String, Arc<crate::websockets::GatewayRuntime>>,
    /// The server exposed through `HttpAdapterHost` (raw `upgrade` listeners).
    pub http_server: crate::http_adapter::HttpServer,
    /// `X-Powered-By` / ETag settings (`app.set` / `app.disable`).
    pub http_settings: crate::http::HttpSettings,
}

/// Entry point for every request.
pub(crate) async fn dispatch(shared: Arc<AppShared>, req: Request) -> Response {
    if let Some(cors) = &shared.cors {
        if cors.is_preflight(&req) {
            return cors.preflight(&req);
        }
    }
    let req_for_cors = shared.cors.as_ref().map(|_| req.clone());

    // `next(err)` from the body parsers (registered after `cors`): the error is mapped
    // (`SyntaxError` → `BadRequestException`) and runs the global filters.
    if let Some(e) = req.body_error.clone() {
        let mut response = handle_exception(e, &empty_context(req), &shared.global_filters).await;
        if let (Some(cors), Some(req)) = (&shared.cors, &req_for_cors) {
            cors.apply(req, &mut response);
        }
        return response;
    }

    let mut chain: Vec<Arc<dyn CorvisMiddleware>> = shared.global_middleware.clone();
    for m in &shared.module_middleware {
        if m.applies(req.method(), req.path()) {
            chain.extend(m.middleware.iter().cloned());
        }
    }
    let mut response = if chain.is_empty() {
        // Fast path: no middleware, so the request does not need to be cloned up front
        // (route_request hands it back on the only errors it can raise itself).
        match AssertUnwindSafe(route_request_owned(shared.clone(), req)).catch_unwind().await {
            Ok(Ok(r)) => r,
            Ok(Err((e, req))) => handle_exception(e, &empty_context(req), &shared.global_filters).await,
            Err(panic) => {
                let ctx = empty_context(Request::empty());
                handle_exception(panic_exception(panic), &ctx, &shared.global_filters).await
            }
        }
    } else {
        let shared2 = shared.clone();
        let endpoint: Endpoint = Arc::new(move |req: Request| {
            let shared = shared2.clone();
            async move { route_request(shared, req).await }.boxed()
        });
        let req_for_errors = req.clone();
        let result = AssertUnwindSafe(Next::new(Arc::new(chain), endpoint).run(req)).catch_unwind().await;
        match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => handle_exception(e, &empty_context(req_for_errors), &shared.global_filters).await,
            Err(panic) => handle_exception(panic_exception(panic), &empty_context(req_for_errors), &shared.global_filters).await,
        }
    };
    if response.status.is_none() {
        response.status = Some(StatusCode::OK);
    }
    if let (Some(cors), Some(req)) = (&shared.cors, &req_for_cors) {
        cors.apply(req, &mut response);
    }
    response
}

fn empty_context(req: Request) -> ExecutionContext {
    ExecutionContext::new(req, Arc::new(Target::default()), Arc::new(Target::default()), None)
}

async fn route_request(shared: Arc<AppShared>, req: Request) -> HandlerResult {
    route_request_owned(shared, req).await.map_err(|(e, _)| e)
}

/// Like `route_request`, but returns the request together with the error so the
/// caller can build an error context without cloning it beforehand.
async fn route_request_owned(shared: Arc<AppShared>, req: Request) -> Result<Response, (HttpException, Request)> {
    for s in &shared.static_dirs {
        if let Some(r) = s.try_serve(&req).await {
            return Ok(r);
        }
    }
    let Some((candidates, params)) = shared.router.lookup(req.method(), req.path()) else {
        let e = NotFoundException::new(format!("Cannot {} {}", req.method(), req.url()));
        return Err((e, req));
    };
    let params = match params {
        Ok(p) => p,
        // URIError from router's decodeParam → 400 Bad Request.
        Err(msg) => return Err((crate::http::BadRequestException::new(msg), req)),
    };
    let Some(idx) = select_route(&shared, &candidates, &req) else {
        let e = NotFoundException::new(format!("Cannot {} {}", req.method(), req.url()));
        return Err((e, req));
    };
    let route = shared.router.routes[idx].clone();
    Ok(execute_route(route, req, params).await)
}

fn select_route(shared: &AppShared, candidates: &[usize], req: &Request) -> Option<usize> {
    let host_ok = |i: &usize| match &shared.router.routes[*i].host {
        Some(h) => req.hostname().map(|rh| rh.eq_ignore_ascii_case(h)).unwrap_or(false),
        None => true,
    };
    let candidates: Vec<usize> = candidates.iter().copied().filter(host_ok).collect();
    let versioning = shared.versioning.as_ref().filter(|v| v.kind != VersioningType::Uri);
    let Some(versioning) = versioning else { return candidates.first().copied() };
    let requested = versioning.requested_version(req);
    let routes = &shared.router.routes;
    match requested {
        Some(v) => candidates
            .iter()
            .copied()
            .find(|i| matches!(&routes[*i].versions, Some(VersionValue::Versions(list)) if list.contains(&v)))
            .or_else(|| candidates.iter().copied().find(|i| matches!(routes[*i].versions, Some(VersionValue::Neutral)))),
        None => candidates.iter().copied().find(|i| matches!(routes[*i].versions, None | Some(VersionValue::Neutral))),
    }
}

/// Executes a matched route: guards -> interceptors -> pipes -> handler, then filters.
pub(crate) async fn execute_route(route: Arc<CompiledRoute>, mut req: Request, params: HashMap<String, String>) -> Response {
    req.params = params;
    req.route_path = Some(route.path.clone());
    req.insert(crate::common::context::RouteStatus(route.default_status));
    let ctx = ExecutionContext::new(req, route.class_target.clone(), route.handler_target.clone(), Some(route.module_ref.clone()));
    let result = match AssertUnwindSafe(run_pipeline(route.clone(), ctx.clone())).catch_unwind().await {
        Ok(r) => r,
        Err(panic) => Err(panic_exception(panic)),
    };
    let res = route.response_mode.and_then(|_| ctx.request().get::<Res>());
    let Some(res) = res else {
        return match result {
            Ok(resp) => finalize(resp, &route),
            Err(e) => handle_exception(e, &ctx, &route.filters).await,
        };
    };
    // `@Res()`: a response ended by the handler is what the client gets (a later
    // exception can no longer be written, as in Express).
    if res.headers_sent() || (res.mode() == ResMode::Handled && result.is_ok()) {
        if result.is_err() {
            crate::logger::Logger::new("ExceptionsHandler")
                .error("Error [ERR_HTTP_HEADERS_SENT]: Cannot set headers after they are sent to the client");
        }
        res.wait_end().await;
        return res.take_response().unwrap_or_default();
    }
    match result {
        Ok(mut resp) => {
            // Passthrough: the status and headers on `res` already include `#[http_code]`
            // and `#[header]`, set before the handler ran.
            res.merge_into(&mut resp, true);
            finalize(resp, &route)
        }
        Err(e) => {
            let mut resp = handle_exception(e, &ctx, &route.filters).await;
            res.merge_into(&mut resp, true);
            resp
        }
    }
}

async fn run_pipeline(route: Arc<CompiledRoute>, ctx: ExecutionContext) -> HandlerResult {
    for g in &route.guards {
        if !g.can_activate(&ctx).await? {
            return Err(ForbiddenException::new("Forbidden resource"));
        }
    }
    if let Some(mode) = route.response_mode {
        // The route status and `@Header()`s are set on the response after the
        // guards and before the interceptors.
        let req = ctx.request();
        let res = Res::new(req.method().clone(), req.header("accept").map(str::to_string), route.default_status, mode);
        for (k, v) in &route.headers {
            res.set(k, v);
        }
        req.insert(res);
    }
    call_interceptors(route.interceptors.clone(), 0, ctx, route).await
}

fn call_interceptors(
    list: Arc<Vec<Arc<dyn CorvisInterceptor>>>,
    index: usize,
    ctx: ExecutionContext,
    route: Arc<CompiledRoute>,
) -> BoxFuture<'static, HandlerResult> {
    if let Some(interceptor) = list.get(index).cloned() {
        let ctx2 = ctx.clone();
        let next = CallHandler::new(move || call_interceptors(list, index + 1, ctx2, route));
        async move { interceptor.intercept(&ctx, next).await }.boxed()
    } else {
        invoke_handler(route, ctx).boxed()
    }
}

/// Raw value of a parameter before pipes.
pub(crate) fn extract_param(req: &Request, kind: ParamKind, data: Option<&str>) -> Value {
    match kind {
        ParamKind::Body => {
            // After a file interceptor, `req.body` holds the multipart text fields.
            let body = crate::multer::body_override(req);
            let body = body.as_ref().unwrap_or(req.body());
            match data {
                None => body.clone(),
                Some(k) => body.get(k).cloned().unwrap_or(Value::Null),
            }
        }
        ParamKind::Param => match data {
            None => Value::Object(req.params().iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect()),
            Some(k) => req.param(k).map(|v| Value::String(v.to_string())).unwrap_or(Value::Null),
        },
        ParamKind::Query => match data {
            None => Value::Object(req.query().clone()),
            Some(k) => req.query().get(k).cloned().unwrap_or(Value::Null),
        },
        ParamKind::Headers => match data {
            None => Value::Object(
                req.headers()
                    .iter()
                    .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().to_string(), Value::String(v.to_string()))))
                    .collect(),
            ),
            Some(k) => req.header(&k.to_ascii_lowercase()).map(|v| Value::String(v.to_string())).unwrap_or(Value::Null),
        },
        ParamKind::Ip => Value::String(req.ip()),
        ParamKind::Custom => Value::Null,
        ParamKind::File => crate::multer::uploaded_param(req, false, data),
        ParamKind::Files => crate::multer::uploaded_param(req, true, data),
    }
}

async fn invoke_handler(route: Arc<CompiledRoute>, ctx: ExecutionContext) -> HandlerResult {
    // Every parameter is resolved concurrently (`Promise.all`) and surfaces the first
    // rejection: the parameter whose failing pipe comes earliest in its chain wins, and on a
    // tie the one declared last (TypeScript applies parameter decorators right to left, so
    // later parameters start first). Every parameter's pipes run.
    let mut args = Vec::with_capacity(route.params.len());
    let mut failure: Option<(usize, HttpException)> = None;
    for p in &route.params {
        if p.def.kind == ParamKind::Custom {
            args.push(Value::Null);
            continue;
        }
        let mut value = extract_param(ctx.request(), p.def.kind, p.def.data);
        let meta = p.def.argument_metadata();
        for (step, pipe) in route.pipes.iter().chain(p.pipes.iter()).enumerate() {
            match pipe.transform(value, &meta).await {
                Ok(v) => value = v,
                Err(e) => {
                    if failure.as_ref().is_none_or(|(s, _)| step <= *s) {
                        failure = Some((step, e));
                    }
                    value = Value::Null;
                    break;
                }
            }
        }
        args.push(value);
    }
    if let Some((_, e)) = failure {
        return Err(e);
    }
    (route.handler)(ctx, args).await
}

fn finalize(mut resp: Response, route: &CompiledRoute) -> Response {
    if let Some((url, code)) = &route.redirect {
        // A handler may override the redirect by returning `{ url, statusCode }`.
        let (url, code) = match resp.json_body() {
            Some(Value::Object(o)) if o.contains_key("url") => (
                o.get("url").and_then(|u| u.as_str()).unwrap_or(url).to_string(),
                o.get("statusCode").and_then(|c| c.as_u64()).map(|c| c as u16).unwrap_or(*code),
            ),
            _ => (url.clone(), *code),
        };
        return Response::redirect(url, code);
    }
    if resp.status.is_none() {
        resp.status = Some(route.default_status);
    }
    for (k, v) in &route.headers {
        if !resp.headers().contains_key(k.as_str()) {
            // `@Header()` goes through express `res.set()`, which adds the default charset.
            if k.eq_ignore_ascii_case("content-type") {
                resp.set_header(k, crate::http::express::content_type(v));
            } else {
                resp.set_header(k, v);
            }
        }
    }
    resp
}

pub(crate) async fn handle_exception(
    exception: HttpException,
    ctx: &ExecutionContext,
    filters: &[Arc<dyn ExceptionFilter>],
) -> Response {
    for f in filters {
        if f.catches(&exception) {
            let status = exception.status();
            let mut r = f.catch(exception, ctx).await;
            if r.status.is_none() {
                r.status = Some(status);
            }
            return r;
        }
    }
    BaseExceptionFilter::handle(&exception)
}

#[derive(Debug)]
struct PanicError(String);
impl std::fmt::Display for PanicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for PanicError {}

fn panic_exception(panic: Box<dyn std::any::Any + Send>) -> HttpException {
    let msg = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "handler panicked".into());
    HttpException::internal(PanicError(msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(join_paths(&["api", "/v1", "cats/", ":id"]), "/api/v1/cats/:id");
        assert_eq!(join_paths(&["", "", ""]), "/");
        assert_eq!(to_matchit("/cats/:id"), "/cats/{id}");
        assert_eq!(to_matchit("/files/*"), "/files/{*path}");
        assert_eq!(to_matchit("/"), "/");
    }
}
