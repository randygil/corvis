//! Static files: `ServeStaticModule.forRoot({ rootPath, serveRoot, exclude })`.
//!
//! The files are served with the behavior of `express.static`
//! ([`crate::http::ServeStatic`]): dotfiles ignored by default, `Range`, `ETag`,
//! `Last-Modified`, conditional GET, a `301` to `dir/` for directories and streamed bodies.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::common::middleware::path_matches;
use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, Token};
use crate::http::serve_static::Outcome;
use crate::http::{Dotfiles, Request, Response, ServeStatic};

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
    /// `serveStaticOptions.dotfiles` (default `Ignore`, like `express.static`).
    pub dotfiles: Dotfiles,
    /// See [`ServeStatic::follow_symlinks`] (default `true`).
    pub follow_symlinks: bool,
    /// The `express.static` built from these options on first use.
    server: OnceLock<ServeStatic>,
}

impl ServeStaticOptions {
    pub fn new(root_path: impl Into<String>) -> Self {
        ServeStaticOptions {
            root_path: root_path.into(),
            serve_root: "/".into(),
            exclude: Vec::new(),
            index: true,
            spa_fallback: false,
            dotfiles: Dotfiles::Ignore,
            follow_symlinks: true,
            server: OnceLock::new(),
        }
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
    /// `serveStaticOptions: { dotfiles }`.
    pub fn dotfiles(mut self, d: Dotfiles) -> Self {
        self.dotfiles = d;
        self
    }
    /// See [`ServeStatic::follow_symlinks`].
    pub fn follow_symlinks(mut self, v: bool) -> Self {
        self.follow_symlinks = v;
        self
    }

    fn server(&self) -> &ServeStatic {
        self.server.get_or_init(|| {
            let mut s = ServeStatic::new(&self.root_path)
                .mount(self.serve_root.trim_end_matches('/'))
                .dotfiles(self.dotfiles)
                .follow_symlinks(self.follow_symlinks);
            if !self.index {
                s = s.index(&[]);
            }
            s
        })
    }

    pub(crate) async fn try_serve(&self, req: &Request) -> Option<Response> {
        if req.method() != http::Method::GET && req.method() != http::Method::HEAD {
            return None;
        }
        let path = req.path();
        if self.exclude.iter().any(|e| path_matches(e, path)) {
            return None;
        }
        let server = self.server();
        let inner = server.inner_path(path)?;
        match server.serve(req, inner).await {
            Outcome::Respond(r) => Some(r),
            Outcome::Next if self.spa_fallback && Path::new(path).extension().is_none() => match server.serve(req, "/index.html").await {
                Outcome::Respond(r) => Some(r),
                Outcome::Next => None,
            },
            Outcome::Next => None,
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
