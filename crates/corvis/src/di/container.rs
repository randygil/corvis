//! The IoC container: module graph (scanner), provider lookup with module
//! encapsulation rules (exports, global modules), instantiation and lifecycle hooks.

use std::any::TypeId;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::future::BoxFuture;

use super::error::DiError;
use super::injectable::{Injectable, ResolveCtx, unknown_dependency};
use super::module::{ConfigureFn, ExportItem, ModuleDefinition, ModuleImport};
use super::provider::{Built, DeclaredDeps, ErasedHooks, Instance, Provider, Scope, downcast, erase};
use super::request_scope::RequestCtx;
use super::token::Token;
use crate::common::lifecycle::HookKind;
use crate::router::controller::ControllerDef;

pub type ModuleId = usize;

/// The deepest resolution chain the container follows before it gives up. A chain this long that never repeats a
/// provider is almost surely a runaway, so it is reported as a circular dependency instead of exhausting the stack.
const MAX_RESOLUTION_DEPTH: usize = 256;

tokio::task_local! {
    /// The resolution path of the factory running on this task, read by `ModuleRef` so that a resolution started
    /// from inside a constructor continues the path instead of starting a new one.
    static RESOLUTION_PATH: Arc<Vec<(ModuleId, Token)>>;
}

/// The resolution path of the construction running on this task, empty outside any construction.
pub(crate) fn current_resolution_path() -> Arc<Vec<(ModuleId, Token)>> {
    RESOLUTION_PATH.try_with(Arc::clone).unwrap_or_default()
}

#[derive(Clone, Debug)]
pub(crate) enum ResolvedExport {
    Token(Token),
    Module(ModuleId),
}

pub(crate) struct ModuleNode {
    pub id: ModuleId,
    pub name: String,
    pub type_id: TypeId,
    pub providers: Vec<Provider>,
    pub provider_index: HashMap<Token, usize>,
    pub imports: Vec<ModuleId>,
    pub exports: Vec<ResolvedExport>,
    pub global: bool,
    pub controllers: Vec<ControllerDef>,
    pub configure: Option<ConfigureFn>,
    pub module_token: Option<Token>,
    pub distance: usize,
    pub internal: bool,
}

impl ModuleNode {
    pub fn provider(&self, token: &Token) -> Option<&Provider> {
        self.provider_index.get(token).map(|i| &self.providers[*i])
    }
}

/// The dependency injection container.
pub struct Container {
    pub(crate) modules: Vec<ModuleNode>,
    pub(crate) globals: Vec<ModuleId>,
    pub(crate) root: ModuleId,
    instances: Mutex<HashMap<(ModuleId, Token), Instance>>,
    /// Concrete instances behind `use_class_as` providers; hooks, handlers and gateways run on these.
    hook_targets: Mutex<HashMap<(ModuleId, Token), Instance>>,
    order: Mutex<Vec<(ModuleId, Token)>>,
    /// Some registered provider has the request scope: only then does `dispatch` install a per-request store.
    has_request_scoped: bool,
}

/// Overrides applied while scanning (used by the testing module).
#[derive(Default, Clone)]
pub(crate) struct Overrides {
    pub providers: HashMap<Token, Provider>,
    pub modules: HashMap<TypeId, fn() -> ModuleDefinition>,
    pub enhancers: Vec<Provider>,
}

struct Scanner {
    modules: Vec<ModuleNode>,
    static_index: HashMap<TypeId, ModuleId>,
    overrides: Overrides,
    /// The modules being filled right now, outermost first.
    chain: Vec<ChainEntry>,
}

/// A module on the scanner's current import chain.
struct ChainEntry {
    type_id: TypeId,
    name: String,
    /// The module was reached through a dynamic import (`for_root()`).
    dynamic: bool,
}

impl Scanner {
    fn new_node(&mut self, name: String, type_id: TypeId) -> ModuleId {
        let id = self.modules.len();
        self.modules.push(ModuleNode {
            id,
            name,
            type_id,
            providers: Vec::new(),
            provider_index: HashMap::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            global: false,
            controllers: Vec::new(),
            configure: None,
            module_token: None,
            distance: usize::MAX,
            internal: false,
        });
        id
    }

    fn add_provider(&mut self, id: ModuleId, provider: Provider) {
        let provider = match self.overrides.providers.get(&provider.token) {
            Some(o) => {
                let mut o = o.clone();
                o.token = provider.token.clone();
                // An overridden `Provider::app_*` stays a global enhancer of the same kind.
                o.enhancer = provider.enhancer.clone();
                // An overridden `Provider::strategy` stays in the strategy registry.
                o.strategy = provider.strategy.clone();
                o
            }
            None => provider,
        };
        let node = &mut self.modules[id];
        if let Some(idx) = node.provider_index.get(&provider.token) {
            node.providers[*idx] = provider;
        } else {
            node.provider_index.insert(provider.token.clone(), node.providers.len());
            node.providers.push(provider);
        }
    }

    fn add_import(&mut self, import: ModuleImport) -> Result<ModuleId, DiError> {
        match import {
            ModuleImport::Static { type_id, definition } => {
                if let Some(id) = self.static_index.get(&type_id) {
                    return Ok(*id);
                }
                let definition = self.overrides.modules.get(&type_id).copied().unwrap_or(definition);
                let def = definition();
                let id = self.new_node(def.name.clone(), type_id);
                self.static_index.insert(type_id, id);
                self.chain.push(ChainEntry { type_id, name: def.name.clone(), dynamic: false });
                let filled = self.fill(id, def, Vec::new(), Vec::new(), Vec::new(), Vec::new(), None);
                self.chain.pop();
                filled?;
                Ok(id)
            }
            ModuleImport::Dynamic(dm) => {
                let def = (dm.base)();
                self.check_dynamic_import_cycle(def.type_id)?;
                let id = self.new_node(def.name.clone(), def.type_id);
                // A dynamic module also satisfies later static imports of the same class.
                self.static_index.entry(def.type_id).or_insert(id);
                self.chain.push(ChainEntry { type_id: def.type_id, name: def.name.clone(), dynamic: true });
                let filled = self.fill(id, def, dm.imports, dm.providers, dm.controllers, dm.exports, dm.global);
                self.chain.pop();
                filled?;
                Ok(id)
            }
        }
    }

    /// Fails when the module on top of the chain is about to dynamically import a class that the same module class
    /// already imported dynamically earlier on the chain: the graph would repeat itself forever.
    fn check_dynamic_import_cycle(&self, imported: TypeId) -> Result<(), DiError> {
        let Some(top) = self.chain.last() else { return Ok(()) };
        let repeated = self.chain.windows(2).position(|w| w[0].type_id == top.type_id && w[1].type_id == imported && w[1].dynamic);
        match repeated {
            Some(start) => {
                let names: Vec<&str> = self.chain[start..].iter().map(|e| e.name.as_str()).collect();
                Err(DiError::CircularImport { chain: names.join(" -> ") })
            }
            None => Ok(()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fill(
        &mut self,
        id: ModuleId,
        def: ModuleDefinition,
        extra_imports: Vec<ModuleImport>,
        extra_providers: Vec<Provider>,
        extra_controllers: Vec<ControllerDef>,
        extra_exports: Vec<ExportItem>,
        global: Option<bool>,
    ) -> Result<(), DiError> {
        self.modules[id].global = global.unwrap_or(def.global);
        self.modules[id].configure = def.configure;
        if let Some(mp) = def.module_provider {
            self.modules[id].module_token = Some(mp.token.clone());
            self.add_provider(id, mp);
        }
        for p in def.providers.into_iter().chain(extra_providers) {
            self.add_provider(id, p);
        }
        self.modules[id].controllers.extend(def.controllers.into_iter().chain(extra_controllers));

        let mut import_ids = Vec::new();
        for imp in def.imports.into_iter().chain(extra_imports) {
            let iid = self.add_import(imp)?;
            if !import_ids.contains(&iid) {
                import_ids.push(iid);
            }
        }
        self.modules[id].imports = import_ids.clone();

        let mut exports = Vec::new();
        for e in def.exports.into_iter().chain(extra_exports) {
            match e {
                ExportItem::Token(t) => exports.push(ResolvedExport::Token(t)),
                ExportItem::Module(tid, name) => {
                    let found = import_ids.iter().copied().find(|i| self.modules[*i].type_id == tid);
                    match found {
                        Some(mid) => exports.push(ResolvedExport::Module(mid)),
                        None => {
                            return Err(DiError::InvalidExport {
                                module: self.modules[id].name.clone(),
                                token: crate::di::token::short_type_name(name),
                            });
                        }
                    }
                }
            }
        }
        self.modules[id].exports = exports;
        Ok(())
    }
}

impl Container {
    /// Scans the module graph starting at `root`.
    pub(crate) fn scan(root: ModuleImport, overrides: Overrides) -> Result<Arc<Container>, DiError> {
        let mut scanner = Scanner { modules: Vec::new(), static_index: HashMap::new(), overrides, chain: Vec::new() };

        // Internal core module (global): Reflector & friends.
        let core = scanner.new_node("InternalCoreModule".into(), TypeId::of::<InternalCoreModule>());
        scanner.modules[core].global = true;
        scanner.modules[core].internal = true;
        scanner.add_provider(core, Provider::value(crate::common::reflector::Reflector));
        scanner.modules[core].exports.push(ResolvedExport::Token(Token::of::<crate::common::reflector::Reflector>()));
        scanner.add_provider(core, Provider::value(crate::http_adapter::HttpAdapterHost::new()));
        scanner.modules[core].exports.push(ResolvedExport::Token(Token::of::<crate::http_adapter::HttpAdapterHost>()));
        scanner.add_provider(core, Provider::value(crate::auth::Authenticator::new()));
        scanner.modules[core].exports.push(ResolvedExport::Token(Token::of::<crate::auth::Authenticator>()));
        for p in std::mem::take(&mut scanner.overrides.enhancers) {
            let t = p.token.clone();
            scanner.add_provider(core, p);
            scanner.modules[core].exports.push(ResolvedExport::Token(t));
        }

        let root = scanner.add_import(root)?;

        let mut modules = scanner.modules;
        // Distances from the root module (BFS), used to order lifecycle hooks.
        let mut queue = VecDeque::new();
        modules[root].distance = 0;
        queue.push_back(root);
        while let Some(m) = queue.pop_front() {
            let d = modules[m].distance;
            for i in modules[m].imports.clone() {
                if modules[i].distance == usize::MAX {
                    modules[i].distance = d + 1;
                    queue.push_back(i);
                }
            }
        }
        for m in modules.iter_mut() {
            if m.distance == usize::MAX {
                m.distance = usize::MAX / 2;
            }
        }

        // Validate exports of providers.
        for m in modules.iter() {
            for e in &m.exports {
                if let ResolvedExport::Token(t) = e {
                    let own = m.provider_index.contains_key(t);
                    let via_import = m.imports.iter().any(|i| Self::find_exported_in(&modules, *i, t, &mut HashSet::new()).is_some());
                    if !own && !via_import {
                        return Err(DiError::InvalidExport { module: m.name.clone(), token: t.display_name() });
                    }
                }
            }
        }

        let globals = modules.iter().filter(|m| m.global).map(|m| m.id).collect();
        let mut container = Container {
            modules,
            globals,
            root,
            instances: Mutex::new(HashMap::new()),
            hook_targets: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
            has_request_scoped: false,
        };
        let has_request_scoped = container.all_providers().any(|(m, p)| container.effective_scope(m, p) == Scope::Request);
        container.has_request_scoped = has_request_scoped;
        container.reject_request_scoped_enhancers()?;
        container.validate_lazy_providers()?;
        container.check_request_scope_consumers()?;
        Ok(Arc::new(container))
    }

    /// Whether any registered provider is request-scoped.
    pub(crate) fn has_request_scoped(&self) -> bool {
        self.has_request_scoped
    }

    /// Global enhancers (`Provider::app_*`) are created once at startup, so they cannot be request-scoped.
    fn reject_request_scoped_enhancers(&self) -> Result<(), DiError> {
        for (module, provider) in self.all_providers() {
            let Some(enhancer) = &provider.enhancer else { continue };
            if self.effective_scope(module, provider) == Scope::Request {
                return Err(DiError::Other(format!(
                    "global {} {} cannot be request-scoped: global guards, interceptors, pipes and filters are created once. Resolve the request-scoped provider inside it with ModuleRef::resolve_for(&request) instead",
                    enhancer.kind(),
                    provider.name
                )));
            }
        }
        Ok(())
    }

    /// Singletons, controllers and gateways are created once, so none of them may depend on a request-scoped provider
    /// or on the current `Request`, directly, through an alias or through transient providers (which are created
    /// along with their consumer). Without this check they would keep the instance of the first request.
    fn check_request_scope_consumers(&self) -> Result<(), DiError> {
        for node in &self.modules {
            for provider in &node.providers {
                if self.effective_scope(node.id, provider) == Scope::Default {
                    self.check_consumer(node.id, provider)?;
                }
            }
            for controller in &node.controllers {
                let provider = &controller.provider;
                if self.effective_scope(node.id, provider) == Scope::Request {
                    return Err(DiError::Other(format!(
                        "controller {} cannot be request-scoped: controllers are created once and bound to their routes at startup",
                        provider.name
                    )));
                }
                self.check_consumer(node.id, provider)?;
            }
        }
        Ok(())
    }

    fn check_consumer(&self, owner: ModuleId, provider: &Provider) -> Result<(), DiError> {
        let consumer = provider.token.display_name();
        let mut chain = vec![consumer.clone()];
        self.scan_for_request_scope(&consumer, owner, provider, &mut chain, &mut HashSet::new())
    }

    fn scan_for_request_scope(
        &self,
        consumer: &str,
        owner: ModuleId,
        provider: &Provider,
        chain: &mut Vec<String>,
        seen: &mut HashSet<(ModuleId, Token)>,
    ) -> Result<(), DiError> {
        if !seen.insert((owner, provider.token.clone())) {
            return Ok(());
        }
        let deps: Vec<Token> = match (&provider.alias_of, &provider.deps) {
            (Some(target), _) => vec![target.clone()],
            (None, DeclaredDeps::Fields(declared)) => declared.items.iter().map(|d| d.token.clone()).collect(),
            (None, DeclaredDeps::Unknown) => Vec::new(),
        };
        for dep in deps {
            let dependency_error = |chain: &[String], name: String| DiError::RequestScopedDependency {
                consumer: consumer.to_string(),
                dependency: name,
                chain: chain.join(" -> "),
            };
            if is_request_token(&dep) {
                chain.push(dep.display_name());
                return Err(dependency_error(chain, dep.display_name()));
            }
            let Some((o, p)) = self.lookup(owner, &dep) else { continue };
            match self.effective_scope(o, p) {
                Scope::Request => {
                    chain.push(dep.display_name());
                    return Err(dependency_error(chain, dep.display_name()));
                }
                Scope::Transient => {
                    chain.push(dep.display_name());
                    self.scan_for_request_scope(consumer, o, p, chain, seen)?;
                    chain.pop();
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Checks the declared dependencies of every transient and request-scoped provider without creating anything, so a
    /// missing dependency or a cycle stops startup instead of the first resolution. Singletons are validated by being
    /// instantiated; factory providers declare nothing and are not checked.
    fn validate_lazy_providers(&self) -> Result<(), DiError> {
        let mut done = HashSet::new();
        for node in &self.modules {
            for provider in &node.providers {
                if self.is_lazy(node.id, provider) {
                    self.validate_lazy_provider(node.id, provider, &mut Vec::new(), &mut done)?;
                }
            }
        }
        Ok(())
    }

    /// Transient and request-scoped providers are built on demand instead of at startup.
    fn is_lazy(&self, owner: ModuleId, provider: &Provider) -> bool {
        self.effective_scope(owner, provider) != Scope::Default
    }

    fn validate_lazy_provider(
        &self,
        owner: ModuleId,
        provider: &Provider,
        path: &mut Vec<(ModuleId, Token)>,
        done: &mut HashSet<(ModuleId, Token)>,
    ) -> Result<(), DiError> {
        let key = (owner, provider.token.clone());
        if done.contains(&key) {
            return Ok(());
        }
        if path.contains(&key) {
            let mut chain: Vec<String> = path.iter().map(|(_, t)| t.display_name()).collect();
            chain.push(provider.token.display_name());
            return Err(DiError::Circular { chain: chain.join(" -> ") });
        }
        path.push(key.clone());
        if let Some(target) = &provider.alias_of {
            let (o, p) = self.lookup(owner, target).ok_or_else(|| DiError::NotFound { token: target.display_name() })?;
            if self.is_lazy(o, p) {
                self.validate_lazy_provider(o, p, path, done)?;
            }
        } else if let DeclaredDeps::Fields(declared) = &provider.deps {
            for dep in &declared.items {
                if is_request_token(&dep.token) {
                    continue;
                }
                match self.lookup(owner, &dep.token) {
                    Some((o, p)) => {
                        if self.is_lazy(o, p) {
                            self.validate_lazy_provider(o, p, path, done)?;
                        }
                    }
                    None if dep.optional => {}
                    None => {
                        return Err(unknown_dependency(declared.requester, declared.names, dep.index, &dep.token, self.module_name(owner)));
                    }
                }
            }
        }
        path.pop();
        done.insert(key);
        Ok(())
    }

    pub(crate) fn module_name(&self, id: ModuleId) -> String {
        self.modules.get(id).map(|m| m.name.clone()).unwrap_or_default()
    }

    fn find_exported_in<'a>(
        modules: &'a [ModuleNode],
        module: ModuleId,
        token: &Token,
        visited: &mut HashSet<ModuleId>,
    ) -> Option<(ModuleId, &'a Provider)> {
        if !visited.insert(module) {
            return None;
        }
        let node = &modules[module];
        for e in &node.exports {
            match e {
                ResolvedExport::Token(t) if t == token => {
                    if let Some(p) = node.provider(t) {
                        return Some((module, p));
                    }
                    for i in &node.imports {
                        if let Some(r) = Self::find_exported_in(modules, *i, token, visited) {
                            return Some(r);
                        }
                    }
                }
                ResolvedExport::Module(mid) => {
                    if let Some(r) = Self::find_exported_in(modules, *mid, token, visited) {
                        return Some(r);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Finds the provider for `token` visible from `module`, honoring encapsulation.
    pub(crate) fn lookup(&self, module: ModuleId, token: &Token) -> Option<(ModuleId, &Provider)> {
        let node = &self.modules[module];
        if let Some(p) = node.provider(token) {
            return Some((module, p));
        }
        let mut visited = HashSet::new();
        for i in &node.imports {
            if let Some(r) = Self::find_exported_in(&self.modules, *i, token, &mut visited) {
                return Some(r);
            }
        }
        for g in &self.globals {
            if *g == module {
                continue;
            }
            if let Some(r) = Self::find_exported_in(&self.modules, *g, token, &mut visited) {
                return Some(r);
            }
        }
        None
    }

    /// The scope a provider resolves with: its own when it is not the default, otherwise the one of the provider it is
    /// an alias (`use_existing`) of.
    pub(crate) fn effective_scope(&self, owner: ModuleId, provider: &Provider) -> Scope {
        let (mut owner, mut provider) = (owner, provider);
        let mut seen = HashSet::new();
        loop {
            if provider.scope != Scope::Default {
                return provider.scope;
            }
            let Some(target) = &provider.alias_of else { return Scope::Default };
            if !seen.insert((owner, provider.token.clone())) {
                return Scope::Default;
            }
            match self.lookup(owner, target) {
                Some((o, p)) => (owner, provider) = (o, p),
                None => return Scope::Default,
            }
        }
    }

    pub(crate) fn resolve(
        self: Arc<Self>,
        module: ModuleId,
        token: Token,
        path: Arc<Vec<(ModuleId, Token)>>,
        request: Option<RequestCtx>,
    ) -> BoxFuture<'static, Result<Instance, DiError>> {
        async move {
            let (owner, provider) = match self.lookup(module, &token) {
                Some((o, p)) => (o, p.clone()),
                None => return Err(DiError::NotFound { token: token.display_name() }),
            };
            self.resolve_provider(owner, provider, path, request).await
        }
        .boxed()
    }

    /// Resolves a provider by its scope: singletons come from (or go to) the container and are built without a
    /// request, transient providers are built for each call with the caller's request, and request-scoped providers
    /// come from the store of `request` (an error without one).
    pub(crate) fn resolve_provider(
        self: Arc<Self>,
        owner: ModuleId,
        provider: Provider,
        path: Arc<Vec<(ModuleId, Token)>>,
        request: Option<RequestCtx>,
    ) -> BoxFuture<'static, Result<Instance, DiError>> {
        async move {
            let key = (owner, provider.token.clone());
            let scope = self.effective_scope(owner, &provider);
            if scope == Scope::Default
                && let Some(i) = self.instances.lock().unwrap().get(&key)
            {
                return Ok(i.clone());
            }
            if path.iter().any(|k| k == &key) {
                let mut chain: Vec<String> = path.iter().map(|(_, t)| t.display_name()).collect();
                chain.push(provider.token.display_name());
                return Err(DiError::Circular { chain: chain.join(" -> ") });
            }
            if path.len() >= MAX_RESOLUTION_DEPTH {
                return Err(depth_error(&path, &provider.token));
            }
            if scope == Scope::Request && request.is_none() {
                let chain = if path.is_empty() { String::new() } else { chain_names(&path, Some(&provider.token)) };
                return Err(DiError::OutsideRequest { token: provider.token.display_name(), chain });
            }
            let mut new_path = (*path).clone();
            new_path.push(key.clone());
            let ctx = ResolveCtx {
                container: self.clone(),
                module: owner,
                requester: Arc::from(provider.name.as_str()),
                deps: &[],
                path: Arc::new(new_path),
                // A singleton must never capture a request.
                request: if scope == Scope::Default { None } else { request.clone() },
            };
            let scoped_path = ctx.path.clone();
            let build = RESOLUTION_PATH.scope(scoped_path, (provider.factory)(ctx));
            match scope {
                Scope::Request => {
                    let store = request.expect("checked above").scope;
                    store.get_or_try_init(key, || async move { build.await.map(|built| built.instance) }).await
                }
                Scope::Transient => Ok(build.await?.instance),
                _ => {
                    let Built { instance, hook_target } = build.await?;
                    let mut map = self.instances.lock().unwrap();
                    if let Some(existing) = map.get(&key) {
                        return Ok(existing.clone());
                    }
                    map.insert(key.clone(), instance.clone());
                    if let Some(target) = hook_target {
                        self.hook_targets.lock().unwrap().insert(key.clone(), target);
                    }
                    self.order.lock().unwrap().push(key);
                    Ok(instance)
                }
            }
        }
        .boxed()
    }

    /// Instantiates an injectable class that is not registered as a provider
    /// (guards, pipes, interceptors... referenced by type) and caches it per module.
    pub(crate) fn construct_injectable<T: Injectable>(self: Arc<Self>, module: ModuleId) -> BoxFuture<'static, Result<Arc<T>, DiError>> {
        async move {
            if T::scope() == Scope::Request {
                return Err(DiError::Other(format!(
                    "{} cannot be request-scoped: guards, interceptors, pipes, filters and middleware referenced by type are created once. Resolve the request-scoped provider inside it with ModuleRef::resolve_for(&request) instead",
                    crate::di::token::short_type_name(std::any::type_name::<T>())
                )));
            }
            let key = (module, Token::of::<T>());
            if let Some(i) = self.instances.lock().unwrap().get(&key)
                && let Some(t) = downcast::<T>(i)
            {
                return Ok(t);
            }
            // Continue the path of the construction this call runs in (empty outside any construction).
            let mut path = (*current_resolution_path()).clone();
            if path.contains(&key) {
                let mut chain: Vec<String> = path.iter().map(|(_, t)| t.display_name()).collect();
                chain.push(key.1.display_name());
                return Err(DiError::Circular { chain: chain.join(" -> ") });
            }
            if path.len() >= MAX_RESOLUTION_DEPTH {
                return Err(depth_error(&path, &key.1));
            }
            path.push(key.clone());
            let path = Arc::new(path);
            let ctx = ResolveCtx {
                container: self.clone(),
                module,
                requester: Arc::from(crate::di::token::short_type_name(std::any::type_name::<T>())),
                deps: &[],
                path: path.clone(),
                request: None,
            };
            let value = Arc::new(RESOLUTION_PATH.scope(path, T::construct(ctx)).await?);
            if T::scope() != Scope::Transient {
                self.instances.lock().unwrap().entry(key).or_insert_with(|| erase(value.clone()));
            }
            Ok(value)
        }
        .boxed()
    }

    /// Instantiates every provider and controller (the instance loader).
    pub(crate) async fn instantiate_all(self: &Arc<Self>, log: bool) -> Result<(), DiError> {
        let logger = crate::logger::Logger::new("InstanceLoader");
        for m in 0..self.modules.len() {
            let node = &self.modules[m];
            for p in node.providers.clone() {
                if self.is_lazy(m, &p) {
                    continue;
                }
                self.clone().resolve_provider(m, p, Arc::new(Vec::new()), None).await?;
            }
            for c in &node.controllers {
                self.clone().resolve_provider(m, c.provider.clone(), Arc::new(Vec::new()), None).await?;
            }
            if log && !node.internal {
                logger.log(format!("{} dependencies initialized", node.name));
            }
        }
        Ok(())
    }

    pub(crate) fn instance(&self, module: ModuleId, token: &Token) -> Option<Instance> {
        self.instances.lock().unwrap().get(&(module, token.clone())).cloned()
    }

    /// The instance hooks run on: the concrete class behind a `use_class_as` provider, otherwise the instance.
    pub(crate) fn hook_instance(&self, module: ModuleId, token: &Token) -> Option<Instance> {
        let key = (module, token.clone());
        if let Some(t) = self.hook_targets.lock().unwrap().get(&key) {
            return Some(t.clone());
        }
        self.instances.lock().unwrap().get(&key).cloned()
    }

    /// Finds an already created instance anywhere in the application (`strict: false`).
    pub(crate) fn find_instance(&self, token: &Token) -> Option<Instance> {
        let map = self.instances.lock().unwrap();
        // Prefer the root module, then any.
        if let Some(i) = map.get(&(self.root, token.clone())) {
            return Some(i.clone());
        }
        let order = self.order.lock().unwrap();
        for key in order.iter() {
            if &key.1 == token
                && let Some(i) = map.get(key)
            {
                return Some(i.clone());
            }
        }
        map.iter().find(|((_, t), _)| t == token).map(|(_, i)| i.clone())
    }

    /// Every instance whose concrete type is `T`.
    pub(crate) fn instances_of<T: Send + Sync + 'static>(&self) -> Vec<Arc<T>> {
        let map = self.instances.lock().unwrap();
        let order = self.order.lock().unwrap();
        order.iter().filter_map(|k| map.get(k)).filter_map(|i| downcast::<T>(i)).collect()
    }

    /// Modules ordered by distance from the root, deepest first (the lifecycle hook order).
    pub(crate) fn modules_for_hooks(&self) -> Vec<ModuleId> {
        let mut ids: Vec<ModuleId> = (0..self.modules.len()).collect();
        ids.sort_by(|a, b| self.modules[*b].distance.cmp(&self.modules[*a].distance));
        ids
    }

    pub(crate) async fn call_hook(self: &Arc<Self>, kind: HookKind, signal: Option<String>) -> crate::Result<()> {
        let mut ids = self.modules_for_hooks();
        if matches!(kind, HookKind::OnModuleDestroy | HookKind::BeforeApplicationShutdown | HookKind::OnApplicationShutdown) {
            // Shutdown hooks are triggered in the same order as init hooks.
            ids = self.modules_for_hooks();
        }
        for m in ids {
            let node = &self.modules[m];
            let mut targets: Vec<(Token, ErasedHooks)> = Vec::new();
            for c in &node.controllers {
                targets.push((c.provider.token.clone(), c.provider.hooks.clone()));
            }
            for p in &node.providers {
                if Some(&p.token) == node.module_token.as_ref() {
                    continue;
                }
                targets.push((p.token.clone(), p.hooks.clone()));
            }
            if let Some(mt) = &node.module_token
                && let Some(p) = node.provider(mt)
            {
                targets.push((mt.clone(), p.hooks.clone()));
            }
            for (token, hooks) in targets {
                let Some(inst) = self.hook_instance(m, &token) else { continue };
                match kind {
                    HookKind::OnModuleInit => {
                        if let Some(h) = &hooks.on_module_init {
                            h(inst).await?;
                        }
                    }
                    HookKind::OnApplicationBootstrap => {
                        if let Some(h) = &hooks.on_application_bootstrap {
                            h(inst).await?;
                        }
                    }
                    HookKind::OnModuleDestroy => {
                        if let Some(h) = &hooks.on_module_destroy {
                            h(inst).await?;
                        }
                    }
                    HookKind::BeforeApplicationShutdown => {
                        if let Some(h) = &hooks.before_application_shutdown {
                            h(inst, signal.clone()).await?;
                        }
                    }
                    HookKind::OnApplicationShutdown => {
                        if let Some(h) = &hooks.on_application_shutdown {
                            h(inst, signal.clone()).await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// All `(module, provider)` pairs, for enhancer discovery.
    pub(crate) fn all_providers(&self) -> impl Iterator<Item = (ModuleId, &Provider)> {
        self.modules.iter().flat_map(|m| m.providers.iter().map(move |p| (m.id, p)))
    }
}

struct InternalCoreModule;

/// The error for a chain longer than [`MAX_RESOLUTION_DEPTH`]: the last links of the chain and the provider that was
/// about to be added.
fn depth_error(path: &[(ModuleId, Token)], next: &Token) -> DiError {
    let tail = path.len().saturating_sub(8);
    let mut chain: Vec<String> = path[tail..].iter().map(|(_, t)| t.display_name()).collect();
    chain.push(next.display_name());
    DiError::Circular { chain: format!("... -> {}", chain.join(" -> ")) }
}

/// `Token::of::<Request>()`, the dependency a `Request` field declares: the current request, not a provider.
fn is_request_token(token: &Token) -> bool {
    *token == Token::of::<crate::http::Request>()
}

/// The names of a resolution path joined with ` -> `, followed by `next` when given.
pub(crate) fn chain_names(path: &[(ModuleId, Token)], next: Option<&Token>) -> String {
    let mut names: Vec<String> = path.iter().map(|(_, t)| t.display_name()).collect();
    names.extend(next.map(Token::display_name));
    names.join(" -> ")
}
