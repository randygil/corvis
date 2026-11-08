//! Controller and route metadata produced by `#[controller]` and `#[routes]`.

use std::any::TypeId;
use std::sync::Arc;

use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;

use crate::common::context::ExecutionContext;
pub use crate::common::decorator::CustomDecorator;
use crate::common::enhancer::{FilterRef, GuardRef, InterceptorRef, PipeRef};
use crate::common::pipe::{ArgumentMetadata, Metatype, ParamKind};
use crate::di::provider::downcast;
use crate::di::{Injectable, Instance, Provider};
use crate::http::{HandlerResult, RequestMethod};
use crate::openapi::ApiSchemaRef;

/// Version(s) of a controller or route (`@Version('1')`, `VERSION_NEUTRAL`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionValue {
    Neutral,
    Versions(Vec<String>),
}

impl VersionValue {
    pub fn single(v: impl Into<String>) -> Self {
        VersionValue::Versions(vec![v.into()])
    }
}

impl From<&str> for VersionValue {
    fn from(s: &str) -> Self {
        VersionValue::single(s)
    }
}
impl From<Vec<&str>> for VersionValue {
    fn from(v: Vec<&str>) -> Self {
        VersionValue::Versions(v.into_iter().map(String::from).collect())
    }
}
impl<const N: usize> From<[&str; N]> for VersionValue {
    fn from(v: [&str; N]) -> Self {
        VersionValue::Versions(v.into_iter().map(String::from).collect())
    }
}

/// `VERSION_NEUTRAL`.
pub const VERSION_NEUTRAL: VersionValue = VersionValue::Neutral;

/// Class-level metadata (`@Controller(...)` plus class decorators).
#[derive(Clone, Default)]
pub struct ControllerMetadata {
    pub paths: Vec<String>,
    pub host: Option<String>,
    pub version: Option<VersionValue>,
    pub guards: Vec<GuardRef>,
    pub interceptors: Vec<InterceptorRef>,
    pub pipes: Vec<PipeRef>,
    pub filters: Vec<FilterRef>,
    pub metadata: Vec<(String, Value)>,
}

impl ControllerMetadata {
    pub fn new(path: impl Into<String>) -> Self {
        ControllerMetadata { paths: vec![path.into()], ..Default::default() }
    }
    pub fn paths(mut self, paths: Vec<&str>) -> Self {
        self.paths = paths.into_iter().map(String::from).collect();
        self
    }
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }
    pub fn version(mut self, v: impl Into<VersionValue>) -> Self {
        self.version = Some(v.into());
        self
    }
    pub fn guard(mut self, g: GuardRef) -> Self {
        self.guards.push(g);
        self
    }
    pub fn interceptor(mut self, i: InterceptorRef) -> Self {
        self.interceptors.push(i);
        self
    }
    pub fn pipe(mut self, p: PipeRef) -> Self {
        self.pipes.push(p);
        self
    }
    pub fn filter(mut self, f: FilterRef) -> Self {
        self.filters.push(f);
        self
    }
    pub fn set_metadata(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        self.metadata.push((key.into(), serde_json::to_value(value).unwrap_or(Value::Null)));
        self
    }
    /// Applies a [`CustomDecorator`] (`#[apply_decorators(...)]`, `#[api_*(...)]` attributes).
    pub fn decorate(mut self, decorator: impl Into<CustomDecorator>) -> Self {
        self.metadata.extend(decorator.into().into_entries());
        self
    }
}

/// A handler parameter (`@Body()`, `@Param('id', ParseIntPipe)`, ...).
#[derive(Clone)]
pub struct ParamDef {
    pub kind: ParamKind,
    pub data: Option<&'static str>,
    pub pipes: Vec<PipeRef>,
    pub metatype: Metatype,
    /// Present when the parameter type implements [`ApiSchema`](crate::openapi::ApiSchema)
    /// (read by OpenAPI generators such as `corvis-swagger`).
    pub api_schema: Option<ApiSchemaRef>,
}

impl ParamDef {
    pub fn new(kind: ParamKind, data: Option<&'static str>, metatype: Metatype) -> Self {
        ParamDef { kind, data, pipes: Vec::new(), metatype, api_schema: None }
    }
    pub fn pipe(mut self, p: PipeRef) -> Self {
        self.pipes.push(p);
        self
    }
    /// Records the OpenAPI description of the parameter type.
    pub fn api_schema(mut self, schema: Option<ApiSchemaRef>) -> Self {
        self.api_schema = schema;
        self
    }
    pub fn argument_metadata(&self) -> ArgumentMetadata {
        ArgumentMetadata { kind: self.kind, data: self.data, metatype: self.metatype }
    }
}

/// Route metadata (method decorators).
#[derive(Clone)]
pub struct RouteMeta {
    pub method: RequestMethod,
    pub paths: Vec<String>,
    pub handler_name: &'static str,
    pub params: Vec<ParamDef>,
    pub guards: Vec<GuardRef>,
    pub interceptors: Vec<InterceptorRef>,
    pub pipes: Vec<PipeRef>,
    pub filters: Vec<FilterRef>,
    pub http_code: Option<u16>,
    pub headers: Vec<(String, String)>,
    pub redirect: Option<(String, u16)>,
    pub metadata: Vec<(String, Value)>,
    pub version: Option<VersionValue>,
    pub sse: bool,
    /// `@Res()` / `@Res({ passthrough: true })` on a parameter.
    pub response_mode: Option<crate::http::ResMode>,
}

pub type HandlerFn<C> = Arc<dyn Fn(Arc<C>, ExecutionContext, Vec<Value>) -> BoxFuture<'static, HandlerResult> + Send + Sync>;
pub(crate) type ErasedHandler = Arc<dyn Fn(ExecutionContext, Vec<Value>) -> BoxFuture<'static, HandlerResult> + Send + Sync>;

/// A route of controller `C`.
pub struct RouteDef<C> {
    pub meta: RouteMeta,
    pub handler: HandlerFn<C>,
}

impl<C> RouteDef<C> {
    pub fn new(method: RequestMethod, path: &str, handler_name: &'static str, handler: HandlerFn<C>) -> Self {
        RouteDef {
            meta: RouteMeta {
                method,
                paths: vec![path.to_string()],
                handler_name,
                params: Vec::new(),
                guards: Vec::new(),
                interceptors: Vec::new(),
                pipes: Vec::new(),
                filters: Vec::new(),
                http_code: None,
                headers: Vec::new(),
                redirect: None,
                metadata: Vec::new(),
                version: None,
                sse: false,
                response_mode: None,
            },
            handler,
        }
    }
    pub fn paths(mut self, paths: Vec<&str>) -> Self {
        self.meta.paths = paths.into_iter().map(String::from).collect();
        self
    }
    pub fn param(mut self, p: ParamDef) -> Self {
        self.meta.params.push(p);
        self
    }
    pub fn guard(mut self, g: GuardRef) -> Self {
        self.meta.guards.push(g);
        self
    }
    pub fn interceptor(mut self, i: InterceptorRef) -> Self {
        self.meta.interceptors.push(i);
        self
    }
    pub fn pipe(mut self, p: PipeRef) -> Self {
        self.meta.pipes.push(p);
        self
    }
    pub fn filter(mut self, f: FilterRef) -> Self {
        self.meta.filters.push(f);
        self
    }
    pub fn http_code(mut self, code: u16) -> Self {
        self.meta.http_code = Some(code);
        self
    }
    pub fn header(mut self, name: &str, value: impl ToString) -> Self {
        self.meta.headers.push((name.to_string(), value.to_string()));
        self
    }
    pub fn redirect(mut self, url: &str, status: u16) -> Self {
        self.meta.redirect = Some((url.to_string(), status));
        self
    }
    pub fn set_metadata(mut self, key: &str, value: impl Serialize) -> Self {
        self.meta.metadata.push((key.to_string(), serde_json::to_value(value).unwrap_or(Value::Null)));
        self
    }
    /// Applies a [`CustomDecorator`] (`#[apply_decorators(...)]`, `#[api_*(...)]` attributes).
    pub fn decorate(mut self, decorator: impl Into<CustomDecorator>) -> Self {
        self.meta.metadata.extend(decorator.into().into_entries());
        self
    }
    pub fn version(mut self, v: impl Into<VersionValue>) -> Self {
        self.meta.version = Some(v.into());
        self
    }
    pub fn sse(mut self) -> Self {
        self.meta.sse = true;
        self
    }
    /// The handler takes the response object (`#[res]` / `#[res(passthrough)]`).
    pub fn response_mode(mut self, mode: crate::http::ResMode) -> Self {
        self.meta.response_mode = Some(mode);
        self
    }
}

/// Implemented by `#[controller]`.
pub trait ControllerBase: Injectable {
    fn controller_metadata() -> ControllerMetadata;
}

/// Implemented by `#[routes]`.
pub trait Controller: ControllerBase {
    fn routes() -> Vec<RouteDef<Self>>;
}

/// Builds the routes of a controller instance.
type RoutesFn = Arc<dyn Fn(&Instance) -> Vec<ErasedRoute> + Send + Sync>;

pub(crate) struct ErasedRoute {
    pub meta: RouteMeta,
    pub handler: ErasedHandler,
}

/// A controller registered in `controllers: [...]`.
#[derive(Clone)]
pub struct ControllerDef {
    pub(crate) type_id: TypeId,
    pub(crate) name: String,
    pub(crate) provider: Provider,
    pub(crate) metadata: fn() -> ControllerMetadata,
    pub(crate) routes: RoutesFn,
    pub(crate) route_metas: fn() -> Vec<RouteMeta>,
}

fn route_metas_of<C: Controller>() -> Vec<RouteMeta> {
    C::routes().into_iter().map(|r| r.meta).collect()
}

impl ControllerDef {
    pub fn of<C: Controller>() -> Self {
        ControllerDef {
            type_id: TypeId::of::<C>(),
            name: crate::di::token::short_type_name(std::any::type_name::<C>()),
            provider: Provider::class::<C>(),
            metadata: C::controller_metadata,
            routes: Arc::new(|instance: &Instance| {
                let Some(this) = downcast::<C>(instance) else { return Vec::new() };
                C::routes()
                    .into_iter()
                    .map(|r| {
                        let this = this.clone();
                        let handler = r.handler;
                        ErasedRoute { meta: r.meta, handler: Arc::new(move |ctx, args| handler(this.clone(), ctx, args)) }
                    })
                    .collect()
            }),
            route_metas: route_metas_of::<C>,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// `TypeId` of the controller class.
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Class-level metadata (`#[controller(...)]` plus the class attributes).
    pub fn metadata(&self) -> ControllerMetadata {
        (self.metadata)()
    }

    /// Metadata of every route handler, in declaration order (no instance needed).
    pub fn route_metas(&self) -> Vec<RouteMeta> {
        (self.route_metas)()
    }
}
