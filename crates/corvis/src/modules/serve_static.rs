//! Static files: `ServeStaticModule.forRoot({ rootPath, serveRoot, exclude })`.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::common::middleware::path_matches;
use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, Token};
use crate::http::{Request, Response};

#[derive(Debug, Clone)]
pub struct ServeStaticOptions {
    /// Directory with the static files (`join(__dirname, '..', 'client')`).
    pub root_path: String,
    /// URL prefix where files are served (default `/`).
    pub serve_root: String,
    /// Route patterns that must never be served from disk (e.g. `/api/(.*)`).
    pub exclude: Vec<String>,
    /// Serve `index.html` for directories.
    pub index: bool,
    /// SPA fallback: serve `index.html` for unknown GET routes not excluded.
    pub spa_fallback: bool,
}

impl ServeStaticOptions {
    pub fn new(root_path: impl Into<String>) -> Self {
        ServeStaticOptions { root_path: root_path.into(), serve_root: "/".into(), exclude: Vec::new(), index: true, spa_fallback: false }
    }
    pub fn serve_root(mut self, r: impl Into<String>) -> Self {
        self.serve_root = r.into();
        self
    }
    pub fn exclude(mut self, patterns: &[&str]) -> Self {
        self.exclude = patterns.iter().map(|s| s.to_string()).collect();
        self
    }
    pub fn spa_fallback(mut self, v: bool) -> Self {
        self.spa_fallback = v;
        self
    }

    fn resolve(&self, url_path: &str) -> Option<PathBuf> {
        let root = self.serve_root.trim_end_matches('/');
        let rel = url_path.strip_prefix(root)?;
        if !rel.is_empty() && !rel.starts_with('/') && !root.is_empty() {
            return None;
        }
        let decoded = percent_encoding::percent_decode_str(rel).decode_utf8_lossy().to_string();
        let rel_path = Path::new(decoded.trim_start_matches('/'));
        if rel_path.components().any(|c| !matches!(c, Component::Normal(_))) {
            return None;
        }
        Some(Path::new(&self.root_path).join(rel_path))
    }

    pub(crate) async fn try_serve(&self, req: &Request) -> Option<Response> {
        if req.method() != http::Method::GET && req.method() != http::Method::HEAD {
            return None;
        }
        let path = req.path();
        if self.exclude.iter().any(|e| path_matches(e, path)) {
            return None;
        }
        let mut file = self.resolve(path)?;
        if file.is_dir() || path.ends_with('/') {
            if !self.index {
                return None;
            }
            file = file.join("index.html");
        }
        match tokio::fs::read(&file).await {
            Ok(data) => Some(Response::bytes(data, crate::common::mime::from_path(&file))),
            Err(_) if self.spa_fallback && Path::new(path).extension().is_none() => {
                let index = Path::new(&self.root_path).join("index.html");
                tokio::fs::read(&index).await.ok().map(|d| Response::bytes(d, "text/html; charset=utf-8"))
            }
            Err(_) => None,
        }
    }
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// `ServeStaticModule`.
#[derive(Default)]
pub struct ServeStaticModule;
crate::default_injectable!(ServeStaticModule);

impl Module for ServeStaticModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<ServeStaticModule>("ServeStaticModule").module_provider(Provider::class::<ServeStaticModule>())
    }
}

impl ServeStaticModule {
    /// `ServeStaticModule.forRoot({ rootPath: 'public' })`.
    pub fn for_root(options: Vec<ServeStaticOptions>) -> DynamicModule {
        let mut m = DynamicModule::new::<ServeStaticModule>();
        for o in options {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            m = m.provider(Provider::use_value(Token::named(format!("SERVE_STATIC_OPTIONS:{n}")), o));
        }
        m
    }
}
