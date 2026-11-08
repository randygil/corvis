//! `SwaggerModule`: `create_document` and `setup`.
//!
//! `setup` registers plain Express-style routes (they are not controller routes, so they do
//! not appear in the `Mapped {...}` logs and skip guards, pipes and interceptors):
//!
//! - `GET <path>`, `<path>/`, `<path>/index.html`: the Swagger UI page;
//! - `GET <path>/swagger-ui-init.js` (and `<path>/<last segment>/swagger-ui-init.js`);
//! - `GET <path>/LICENSE`: `404`;
//! - `GET <path>-json` and `<path>-yaml` (`jsonDocumentUrl` / `yamlDocumentUrl`);
//! - the `swagger-ui-dist` assets under `<path>/`: served from
//!   [`SwaggerCustomOptions::custom_swagger_ui_path`] when it is set, otherwise redirected (302)
//!   to the same files of `swagger-ui-dist` on jsDelivr.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use corvis::prelude::*;
use serde_json::Value;

use crate::document::OpenApiDocument;
use crate::explorer::{self, SwaggerDocumentOptions};
use crate::swagger_ui::{self, SWAGGER_UI_DIST_FILES, SWAGGER_UI_DIST_VERSION, SwaggerUiHtmlOptions};

/// Returns the document to serve for a request (`patchDocumentOnRequest`).
pub type PatchDocumentFn = Arc<dyn Fn(&Request, &OpenApiDocument) -> OpenApiDocument + Send + Sync>;

/// `SwaggerCustomOptions`.
#[derive(Clone)]
pub struct SwaggerCustomOptions {
    /// Prepend the global prefix to the path (`useGlobalPrefix`).
    pub use_global_prefix: bool,
    /// Serve the Swagger UI (`ui` / `swaggerUiEnabled`, default `true`).
    pub ui: bool,
    /// Serve the raw definitions: `None` = both (`raw: true`), `Some(vec![])` = none,
    /// `Some(vec!["json"])` = only JSON...
    pub raw: Option<Vec<String>>,
    /// `jsonDocumentUrl` (default `<path>-json`).
    pub json_document_url: Option<String>,
    /// `yamlDocumentUrl` (default `<path>-yaml`).
    pub yaml_document_url: Option<String>,
    /// `explorer`.
    pub explorer: bool,
    /// `swaggerOptions` (functions as `{"$js": "function () {...}"}`).
    pub swagger_options: Value,
    /// `customCss`.
    pub custom_css: String,
    /// `customCssUrl`.
    pub custom_css_url: Vec<String>,
    /// `customJs`.
    pub custom_js: Vec<String>,
    /// `customJsStr`.
    pub custom_js_str: Vec<String>,
    /// `customfavIcon`.
    pub custom_favicon: Option<String>,
    /// `customSiteTitle` (default `Swagger UI`).
    pub custom_site_title: String,
    /// `customSwaggerUiPath`: a local copy of `swagger-ui-dist` to serve the assets from.
    pub custom_swagger_ui_path: Option<PathBuf>,
    /// `swaggerUrl`.
    pub swagger_url: Option<String>,
    /// `patchDocumentOnRequest`.
    pub patch_document_on_request: Option<PatchDocumentFn>,
}

impl Default for SwaggerCustomOptions {
    fn default() -> Self {
        SwaggerCustomOptions {
            use_global_prefix: false,
            ui: true,
            raw: None,
            json_document_url: None,
            yaml_document_url: None,
            explorer: false,
            swagger_options: Value::Null,
            custom_css: String::new(),
            custom_css_url: Vec::new(),
            custom_js: Vec::new(),
            custom_js_str: Vec::new(),
            custom_favicon: None,
            custom_site_title: "Swagger UI".into(),
            custom_swagger_ui_path: None,
            swagger_url: None,
            patch_document_on_request: None,
        }
    }
}

/// The `SwaggerModule`: builds the document and serves it.
pub struct SwaggerModule;

impl SwaggerModule {
    /// `SwaggerModule.createDocument(app, config)`.
    pub fn create_document(app: &CorvisApplication, config: &OpenApiDocument) -> OpenApiDocument {
        explorer::create_document(app, config, &SwaggerDocumentOptions::default())
    }

    /// `SwaggerModule.createDocument(app, config, options)`.
    pub fn create_document_with(app: &CorvisApplication, config: &OpenApiDocument, options: &SwaggerDocumentOptions) -> OpenApiDocument {
        explorer::create_document(app, config, options)
    }

    /// `SwaggerModule.setup(path, app, document)`.
    pub fn setup(path: &str, app: &mut CorvisApplication, document: OpenApiDocument) {
        Self::setup_with(path, app, document, SwaggerCustomOptions::default());
    }

    /// `SwaggerModule.setup(path, app, () => createDocument(...), options)`: the document is
    /// built on the first request.
    pub fn setup_lazy(
        path: &str,
        app: &mut CorvisApplication,
        factory: impl Fn() -> OpenApiDocument + Send + Sync + 'static,
        options: SwaggerCustomOptions,
    ) {
        Self::mount(path, app, DocumentSource::Lazy(Arc::new(factory), Arc::new(Mutex::new(None))), options);
    }

    /// `SwaggerModule.setup(path, app, document, options)`.
    pub fn setup_with(path: &str, app: &mut CorvisApplication, document: OpenApiDocument, options: SwaggerCustomOptions) {
        Self::mount(path, app, DocumentSource::Ready(Arc::new(document)), options);
    }

    fn mount(path: &str, app: &mut CorvisApplication, source: DocumentSource, options: SwaggerCustomOptions) {
        let global_prefix = app.global_prefix().to_string();
        let prefix_ok = options.use_global_prefix && valid_global_prefix(&global_prefix);
        let final_path = validate_path(&if prefix_ok { format!("{global_prefix}{}", validate_path(path)) } else { path.to_string() });
        let last = final_path.rsplit('/').next().unwrap_or("").to_string();
        let validated_prefix = if prefix_ok { validate_path(&global_prefix) } else { String::new() };
        let json_url = match &options.json_document_url {
            Some(u) => format!("{validated_prefix}{}", validate_path(u)),
            None => format!("{final_path}-json"),
        };
        let yaml_url = match &options.yaml_document_url {
            Some(u) => format!("{validated_prefix}{}", validate_path(u)),
            None => format!("{final_path}-yaml"),
        };
        let (serve_json, serve_yaml) = match &options.raw {
            None => (true, true),
            Some(list) => (list.iter().any(|r| r == "json"), list.iter().any(|r| r == "yaml")),
        };
        let state = Arc::new(Served {
            source,
            options,
            final_path: normalize_rel_path(&final_path),
            nested_init: normalize_rel_path(&format!("{final_path}/{last}/swagger-ui-init.js")),
            init_js: normalize_rel_path(&format!("{final_path}/swagger-ui-init.js")),
            base_url: normalize_rel_path(&format!("./{last}/")),
            json_url: serve_json.then(|| normalize_rel_path(&json_url)),
            yaml_url: serve_yaml.then(|| normalize_rel_path(&yaml_url)),
            cache: Mutex::new(Cache::default()),
        });
        app.use_fn(move |req: Request, next: Next| {
            let state = state.clone();
            async move {
                match state.handle(&req) {
                    Some(r) => r,
                    None => next.run(req).await,
                }
            }
        });
    }
}

#[derive(Clone)]
enum DocumentSource {
    Ready(Arc<OpenApiDocument>),
    Lazy(Arc<dyn Fn() -> OpenApiDocument + Send + Sync>, Arc<Mutex<Option<Arc<OpenApiDocument>>>>),
}

impl DocumentSource {
    fn get(&self) -> Arc<OpenApiDocument> {
        match self {
            DocumentSource::Ready(d) => d.clone(),
            DocumentSource::Lazy(f, cell) => {
                let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
                guard.get_or_insert_with(|| Arc::new(f())).clone()
            }
        }
    }
}

#[derive(Default)]
struct Cache {
    html: Option<String>,
    html_trailing: Option<String>,
    init_js: Option<String>,
}

struct Served {
    source: DocumentSource,
    options: SwaggerCustomOptions,
    final_path: String,
    init_js: String,
    nested_init: String,
    base_url: String,
    json_url: Option<String>,
    yaml_url: Option<String>,
    cache: Mutex<Cache>,
}

/// An Express route path match: case-insensitive, one optional trailing slash.
fn route_matches(route: &str, path: &str) -> bool {
    let p = if path.len() > 1 { path.strip_suffix('/').unwrap_or(path) } else { path };
    let r = if route.len() > 1 { route.strip_suffix('/').unwrap_or(route) } else { route };
    p.eq_ignore_ascii_case(r)
}

fn typed(body: String, content_type: &str) -> HandlerResult {
    Ok(Response::text(body).with_header("Content-Type", content_type))
}

impl Served {
    fn html_options(&self) -> SwaggerUiHtmlOptions<'_> {
        SwaggerUiHtmlOptions {
            custom_css: &self.options.custom_css,
            custom_js: &self.options.custom_js,
            custom_js_str: &self.options.custom_js_str,
            custom_favicon: self.options.custom_favicon.as_deref(),
            custom_site_title: &self.options.custom_site_title,
            custom_css_url: &self.options.custom_css_url,
            explorer: self.options.explorer,
        }
    }

    fn document_for(&self, req: &Request) -> (Arc<OpenApiDocument>, bool) {
        let doc = self.source.get();
        match &self.options.patch_document_on_request {
            Some(f) => (Arc::new(f(req, &doc)), true),
            None => (doc, false),
        }
    }

    fn handle(&self, req: &Request) -> Option<HandlerResult> {
        let method = req.method();
        if method != corvis::http::Method::GET && method != corvis::http::Method::HEAD {
            return None;
        }
        let path = req.path();
        if self.options.ui {
            if route_matches(&self.init_js, path) || route_matches(&self.nested_init, path) {
                let (doc, patched) = self.document_for(req);
                let build = || {
                    swagger_ui::build_swagger_init_js(doc.as_value(), self.options.swagger_url.as_deref(), &self.options.swagger_options)
                };
                let js = if patched {
                    build()
                } else {
                    let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                    cache.init_js.get_or_insert_with(build).clone()
                };
                return Some(typed(js, "application/javascript"));
            }
            let index = format!("{}/index.html", self.final_path);
            if route_matches(&self.final_path, path) || route_matches(&index, path) {
                let trailing = req.url().ends_with('/');
                let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                let html = if trailing {
                    cache.html_trailing.get_or_insert_with(|| swagger_ui::build_swagger_html("./", &self.html_options())).clone()
                } else {
                    cache.html.get_or_insert_with(|| swagger_ui::build_swagger_html(&self.base_url, &self.html_options())).clone()
                };
                return Some(typed(html, "text/html"));
            }
            if route_matches(&format!("{}/LICENSE", self.final_path), path) {
                return Some(Err(NotFoundException::empty()));
            }
        }
        if let Some(url) = &self.json_url
            && route_matches(url, path)
        {
            let (doc, _) = self.document_for(req);
            return Some(typed(doc.to_json(), "application/json"));
        }
        if let Some(url) = &self.yaml_url
            && route_matches(url, path)
        {
            let (doc, _) = self.document_for(req);
            return Some(typed(doc.to_yaml(), "text/yaml"));
        }
        if self.options.ui {
            return self.asset(path);
        }
        None
    }

    /// `app.useStaticAssets(swaggerAssetsPath, { prefix: finalPath })`.
    fn asset(&self, path: &str) -> Option<HandlerResult> {
        let lower = path.to_ascii_lowercase();
        let prefix = format!("{}/", self.final_path.to_ascii_lowercase());
        let rel = lower.strip_prefix(&prefix).map(|_| &path[prefix.len()..])?;
        if rel.is_empty() || rel.contains("..") {
            return None;
        }
        if let Some(dir) = &self.options.custom_swagger_ui_path {
            let file = confined_asset(dir, rel)?;
            let data = std::fs::read(&file).ok()?;
            return Some(Ok(Response::bytes(data, mime_for(rel))));
        }
        let name = SWAGGER_UI_DIST_FILES.iter().find(|f| **f == rel)?;
        let url = format!("https://cdn.jsdelivr.net/npm/swagger-ui-dist@{SWAGGER_UI_DIST_VERSION}/{name}");
        Some(Ok(Response::redirect(url, 302)))
    }
}

/// Resolves `<path>/<rel>` inside `custom_swagger_ui_path` the way `express.static` does: the
/// result must stay inside the folder. `rel` is percent-decoded and must be plain file names
/// separated by `/` (no empty, `.`/`..` or dot-file segments, no `\`, `:` or control characters,
/// no Windows device names); the canonical path (symlinks and NTFS short names resolved) must
/// still be inside the canonical folder and be a regular file. Anything else is not served.
pub(crate) fn confined_asset(dir: &std::path::Path, rel: &str) -> Option<PathBuf> {
    let decoded = percent_decode(rel)?;
    let mut file = dir.to_path_buf();
    for seg in decoded.split('/') {
        if !safe_segment(seg) {
            return None;
        }
        file.push(seg);
    }
    let root = dir.canonicalize().ok()?;
    let canonical = file.canonicalize().ok()?;
    let inside = canonical.strip_prefix(&root).ok()?;
    let visible = inside.components().all(|c| match c {
        std::path::Component::Normal(s) => s.to_str().is_some_and(safe_segment),
        _ => false,
    });
    (visible && canonical.is_file()).then_some(canonical)
}

fn safe_segment(seg: &str) -> bool {
    if seg.is_empty() || seg.starts_with('.') || seg.chars().any(|c| matches!(c, '\\' | ':' | '/') || c.is_control()) {
        return false;
    }
    // `CON`, `nul.txt`, `COM1.js`...: Windows opens the device instead of a file.
    let stem = seg.split('.').next().unwrap_or(seg).trim_end().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    !device
}

/// `decodeURIComponent` of a path (`None` when malformed or not UTF-8, like `send`'s 400).
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn mime_for(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or("") {
        "js" => "application/javascript; charset=UTF-8",
        "css" => "text/css; charset=UTF-8",
        "html" => "text/html; charset=UTF-8",
        "png" => "image/png",
        "json" | "map" => "application/json; charset=UTF-8",
        "md" => "text/markdown; charset=UTF-8",
        _ => "application/octet-stream",
    }
}

/// `validatePath`: prepends `/` when missing.
pub fn validate_path(p: &str) -> String {
    if p.starts_with('/') { p.to_string() } else { format!("/{p}") }
}

/// `normalizeRelPath`: collapses repeated slashes.
pub fn normalize_rel_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    out
}

fn valid_global_prefix(p: &str) -> bool {
    !p.is_empty() && p != "/"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(validate_path("docs"), "/docs");
        assert_eq!(normalize_rel_path("/docs//x///y"), "/docs/x/y");
        assert!(route_matches("/docs", "/DOCS/"));
        assert!(!route_matches("/docs", "/docs//"));
    }
}
