//! Read-only introspection of an application: modules, controllers, route metadata and the
//! paths the router mounts them on. Used by document generators such as `corvis-swagger`
//! (the module container and the application configuration, read-only).
//!
//! Nothing here requires `app.init()`: it can be used right after `CorvisFactory::create`.

use std::any::TypeId;

use super::CorvisApplication;
use crate::common::middleware::path_matches;
use crate::router::controller::{ControllerMetadata, RouteMeta, VersionValue};
use crate::router::join_paths;
use crate::router::versioning::{VersioningOptions, VersioningType};

/// A module of the application graph, in scan order (root first, then its imports depth-first).
#[derive(Clone)]
pub struct ModuleInfo {
    pub name: String,
    pub type_id: TypeId,
    pub global: bool,
    /// Framework-internal module (`InternalCoreModule`).
    pub internal: bool,
    /// Indexes (into the list returned by [`CorvisApplication::modules_info`]) of the imports.
    pub imports: Vec<usize>,
    pub controllers: Vec<ControllerInfo>,
}

/// A controller and the metadata of its routes.
#[derive(Clone)]
pub struct ControllerInfo {
    /// Class name (`CatsController`).
    pub name: String,
    pub type_id: TypeId,
    pub metadata: ControllerMetadata,
    /// Route handlers in declaration order.
    pub routes: Vec<RouteMeta>,
}

/// A full path a route is mounted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoutePath {
    /// Express-style path, e.g. `/api/v1/cats/:id`.
    pub path: String,
    /// URI version segment value (`"1"` for `/v1/...`) when URI versioning applies.
    pub version: Option<String>,
}

impl CorvisApplication {
    /// Every module with its controllers, in scan order.
    pub fn modules_info(&self) -> Vec<ModuleInfo> {
        self.container
            .modules
            .iter()
            .map(|m| ModuleInfo {
                name: m.name.clone(),
                type_id: m.type_id,
                global: m.global,
                internal: m.internal,
                imports: m.imports.clone(),
                controllers: m
                    .controllers
                    .iter()
                    .map(|c| ControllerInfo {
                        name: c.name().to_string(),
                        type_id: c.type_id(),
                        metadata: c.metadata(),
                        routes: c.route_metas(),
                    })
                    .collect(),
            })
            .collect()
    }

    /// `app.setGlobalPrefix(prefix)` value (empty when unset).
    pub fn global_prefix(&self) -> &str {
        &self.config.router.global_prefix
    }

    /// Paths excluded from the global prefix (`setGlobalPrefix(p, { exclude })`).
    pub fn global_prefix_exclude(&self) -> &[String] {
        &self.config.router.prefix_exclude
    }

    /// `app.enableVersioning(...)` options.
    pub fn versioning_options(&self) -> Option<&VersioningOptions> {
        self.config.router.versioning.as_ref()
    }

    /// The version that applies to a route: its own, the controller's, or the default one.
    pub fn effective_route_version(&self, controller: &ControllerMetadata, route: &RouteMeta) -> Option<VersionValue> {
        route
            .version
            .clone()
            .or_else(|| controller.version.clone())
            .or_else(|| self.versioning_options().and_then(|v| v.default_version.clone()))
    }

    /// The full paths a route is mounted on, computed exactly like the router: global prefix
    /// (and its exclusions), URI version segments and every controller/route path alias.
    pub fn resolve_route_paths(&self, controller: &ControllerMetadata, route: &RouteMeta) -> Vec<ResolvedRoutePath> {
        self.resolve_route_paths_with(controller, route, true)
    }

    /// Like [`resolve_route_paths`](Self::resolve_route_paths); with `global_prefix: false`
    /// the global prefix is left out (`ignore_global_prefix` of the OpenAPI document options).
    pub fn resolve_route_paths_with(
        &self,
        controller: &ControllerMetadata,
        route: &RouteMeta,
        global_prefix: bool,
    ) -> Vec<ResolvedRoutePath> {
        let cfg = &self.config.router;
        let version = self.effective_route_version(controller, route);
        let versions: Vec<Option<(String, String)>> = match (cfg.versioning.as_ref(), &version) {
            (Some(v), Some(VersionValue::Versions(list))) if v.kind == VersioningType::Uri => {
                list.iter().map(|x| Some((format!("{}{}", v.prefix, x), x.clone()))).collect()
            }
            _ => vec![None],
        };
        let c_paths = if controller.paths.is_empty() { vec![String::new()] } else { controller.paths.clone() };
        // Order: versions, controller paths, route paths.
        let mut out = Vec::new();
        for vp in &versions {
            for c_path in &c_paths {
                let excluded = cfg.prefix_exclude.iter().any(|e| path_matches(e, c_path));
                let prefix = if !global_prefix || excluded { "" } else { cfg.global_prefix.as_str() };
                for r_path in &route.paths {
                    let segment = vp.as_ref().map(|(s, _)| s.as_str()).unwrap_or("");
                    out.push(ResolvedRoutePath {
                        path: join_paths(&[prefix, segment, c_path, r_path]),
                        version: vp.as_ref().map(|(_, v)| v.clone()),
                    });
                }
            }
        }
        out
    }
}
