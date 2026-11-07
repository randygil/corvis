//! The Scalar API reference page (`apiReference(config)` of the Scalar server integrations),
//! rendered with `@scalar/client-side-rendering` 0.3: an HTML page that loads
//! `@scalar/api-reference` from a CDN and receives the configuration (usually the OpenAPI
//! document in `content`).
//!
//! ```ignore
//! let document = cleanup_open_api_doc(SwaggerModule::create_document(&app, &config));
//! // app.use('/reference', apiReference({ content: document }))
//! scalar::mount(&mut app, "/reference", ApiReferenceOptions::new().content(document));
//! ```
//!
//! The page applies [`DEFAULT_THEME_CSS`] unless a `theme` is given. Scalar integrations
//! also put an `_integration` key first in the configuration; it is left out unless
//! [`ApiReferenceOptions::integration`] sets it, so an app that must serve a byte-identical
//! page to an existing one sets the same value it used there.

use std::sync::Arc;

use corvis::prelude::*;
use serde_json::{Map, Value};

use crate::document::OpenApiDocument;
use crate::js;

/// The default theme (`customThemeCSS`) of the page, used when no `theme` is set.
pub const DEFAULT_THEME_CSS: &str = include_str!("assets/scalar-theme.css");

/// `DEFAULT_CDN` of `@scalar/client-side-rendering`.
pub const DEFAULT_CDN: &str = "https://cdn.jsdelivr.net/npm/@scalar/api-reference";

/// The configuration of `apiReference({...})`.
#[derive(Debug, Clone, Default)]
pub struct ApiReferenceOptions {
    /// Every option except `cdn`, `pageTitle` and `nonce`, in insertion order (`content`, `url`,
    /// `theme`, `customCss`, ...). Functions are written as `{"$js": "(req) => ..."}`.
    pub config: Map<String, Value>,
    pub cdn: Option<String>,
    pub page_title: Option<String>,
    pub nonce: Option<String>,
    /// The `_integration` value sent to the Scalar page (first key of its configuration).
    /// `None` (the default) leaves the key out; see [`ApiReferenceOptions::integration`].
    pub integration: Option<String>,
}

impl ApiReferenceOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// `content: document`.
    pub fn content(self, document: impl Into<Value>) -> Self {
        self.set("content", document.into())
    }

    /// `content: document` (an [`OpenApiDocument`]).
    pub fn document(self, document: &OpenApiDocument) -> Self {
        self.set("content", document.as_value().clone())
    }

    /// `url: '/openapi.json'`.
    pub fn url(self, url: impl Into<String>) -> Self {
        self.set("url", Value::String(url.into()))
    }

    /// `theme: 'purple'` (disables [`DEFAULT_THEME_CSS`]).
    pub fn theme(self, theme: impl Into<String>) -> Self {
        self.set("theme", Value::String(theme.into()))
    }

    /// Any other option (`customCss`, `hideModels`, `metaData`, ...).
    pub fn set(mut self, key: &str, value: Value) -> Self {
        self.config.insert(key.to_string(), value);
        self
    }

    pub fn cdn(mut self, cdn: impl Into<String>) -> Self {
        self.cdn = Some(cdn.into());
        self
    }

    pub fn page_title(mut self, title: impl Into<String>) -> Self {
        self.page_title = Some(title.into());
        self
    }

    pub fn nonce(mut self, nonce: impl Into<String>) -> Self {
        self.nonce = Some(nonce.into());
        self
    }

    /// `_integration: name`, the identifier Scalar server integrations put first in the
    /// configuration (`"express"`, `"fastify"`, ...). Not sent unless set: an app that must
    /// reproduce a page served by another integration byte for byte sets that integration's value.
    pub fn integration(mut self, name: impl Into<String>) -> Self {
        self.integration = Some(name.into());
        self
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Makes JSON safe inside an inline `<script>`: `<`, `>` and `&` become `<`, `>` and
/// `&` (a string can never close the tag or open a comment) and U+2028/U+2029 are escaped.
/// They only occur inside JSON strings, where the escapes denote the same characters.
pub(crate) fn escape_inline_json(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

fn nonce_attribute(nonce: Option<&str>) -> String {
    match nonce {
        Some(n) if !n.is_empty() => format!(" nonce=\"{}\"", escape_html(n).replace('"', "&quot;")),
        _ => String::new(),
    }
}

fn add_indent(s: &str, spaces: usize) -> String {
    let indent = " ".repeat(spaces);
    s.split('\n').enumerate().map(|(i, l)| if i == 0 { l.to_string() } else { format!("{indent}{l}") }).collect::<Vec<_>>().join("\n")
}

fn is_function(v: &Value) -> Option<&str> {
    let o = v.as_object()?;
    if o.len() != 1 {
        return None;
    }
    o.get("$js")?.as_str()
}

/// `serializeConfigToJs(configuration)`. The JSON part goes through [`escape_inline_json`] (the
/// document is embedded in the page); `{"$js": ...}` values are configuration functions written
/// by the developer and are emitted as they are.
pub fn serialize_config_to_js(configuration: &Map<String, Value>) -> String {
    let mut rest = Map::new();
    let mut function_props = Vec::new();
    for (k, v) in configuration {
        if let Some(code) = is_function(v) {
            function_props.push(format!("\"{k}\": {code}"));
        } else if let Value::Array(items) = v {
            if items.iter().any(|i| is_function(i).is_some()) {
                let parts: Vec<String> = items.iter().map(|i| is_function(i).map(str::to_string).unwrap_or_else(|| escape_inline_json(&js::stringify(i)))).collect();
                function_props.push(format!("\"{k}\": [{}]", parts.join(", ")));
            } else {
                rest.insert(k.clone(), v.clone());
            }
        } else {
            rest.insert(k.clone(), v.clone());
        }
    }
    let json = escape_inline_json(&js::stringify_pretty(&Value::Object(rest), 2));
    let indented = json.split('\n').enumerate().map(|(i, l)| if i == 0 { l.to_string() } else { format!("      {l}") }).collect::<Vec<_>>().join("\n");
    if function_props.is_empty() {
        return indented;
    }
    if json == "{}" {
        return format!("{{\n        {}\n      }}", function_props.join(",\n        "));
    }
    let lines: Vec<&str> = indented.split('\n').collect();
    format!("{},\n        {}\n      }}", lines[..lines.len() - 1].join("\n"), function_props.join(",\n        "))
}

/// `renderApiReference({ config, pageTitle, cdn, nonce }, customTheme)`.
pub fn render_api_reference(options: &ApiReferenceOptions, custom_theme: &str) -> String {
    let title = escape_html(options.page_title.as_deref().unwrap_or("Scalar API Reference"));
    // { customCss, theme, ...rest } → { ...rest, theme?, customCss? }
    let mut configuration = Map::new();
    for (k, v) in &options.config {
        if k != "customCss" && k != "theme" {
            configuration.insert(k.clone(), v.clone());
        }
    }
    let theme = options.config.get("theme").filter(|t| js::truthy(Some(t))).cloned();
    if let Some(t) = &theme {
        configuration.insert("theme".into(), t.clone());
    }
    if let Some(css) = options.config.get("customCss") {
        configuration.insert("customCss".into(), css.clone());
    }
    // getConfiguration: content is dropped when url is also given.
    if js::truthy(configuration.get("content")) && js::truthy(configuration.get("url")) {
        configuration.remove("content");
    }
    let nonce = options.nonce.as_deref();
    let csp_meta = match nonce {
        Some(n) if !n.is_empty() => format!("\n    <meta property=\"csp-nonce\" content=\"{}\" />", escape_html(n).replace('"', "&quot;")),
        _ => String::new(),
    };
    let mut styles: Vec<String> = Vec::new();
    if let Some(css) = configuration.get("customCss").filter(|c| js::truthy(Some(c))) {
        styles.push("/* Custom CSS */".into());
        styles.push(css.as_str().map(str::to_string).unwrap_or_else(|| js::stringify(css)));
    }
    if theme.is_none() && !custom_theme.is_empty() {
        styles.push("/* Custom Theme */".into());
        styles.push(custom_theme.to_string());
    }
    let style_tag = if styles.is_empty() {
        String::new()
    } else {
        format!("\n    <style type=\"text/css\"{}>\n      {}\n    </style>", nonce_attribute(nonce), add_indent(&styles.join("\n\n"), 6))
    };
    let nonce_attr = nonce_attribute(nonce);
    let scripts = format!(
        "\n    <!-- Load the Script -->\n    <script src=\"{}\"{nonce_attr}></script>\n\n    <!-- Initialize the Scalar API Reference -->\n    <script type=\"text/javascript\"{nonce_attr}>\n      Scalar.createApiReference('#app', {})\n    </script>",
        options.cdn.as_deref().unwrap_or(DEFAULT_CDN),
        serialize_config_to_js(&configuration)
    );
    format!(
        "<!doctype html>\n<html>\n  <head>\n    <title>{title}</title>\n    <meta charset=\"utf-8\" />\n    <meta\n      name=\"viewport\"\n      content=\"width=device-width, initial-scale=1\" />{csp_meta}{style_tag}\n  </head>\n  <body>\n    <div id=\"app\"></div>{scripts}\n  </body>\n</html>"
    )
}

/// `apiReference(configuration)`: the HTML page (`{ _integration?, ...configuration }` with
/// [`DEFAULT_THEME_CSS`] unless a `theme` is given).
pub fn api_reference_html(options: &ApiReferenceOptions) -> String {
    let mut config = Map::new();
    if let Some(name) = &options.integration {
        config.insert("_integration".into(), Value::String(name.clone()));
    }
    for (k, v) in &options.config {
        config.insert(k.clone(), v.clone());
    }
    let full = ApiReferenceOptions { config, ..options.clone() };
    render_api_reference(&full, DEFAULT_THEME_CSS)
}

/// `app.use(path, apiReference(configuration))`: every method on `path` and below it
/// (Express `app.use` prefix matching) answers the page with `res.send(html)`.
pub fn mount(app: &mut CorvisApplication, path: &str, options: ApiReferenceOptions) {
    let html: Arc<str> = api_reference_html(&options).into();
    let mount_path = path.trim_end_matches('/').to_ascii_lowercase();
    app.use_fn(move |req: Request, next: Next| {
        let html = html.clone();
        let matched = use_path_matches(&mount_path, req.path());
        async move {
            if matched {
                Ok(Response::html(html.to_string()))
            } else {
                next.run(req).await
            }
        }
    });
}

/// Express `app.use(path)` matching: the path itself or anything below it, case-insensitive.
fn use_path_matches(mount: &str, path: &str) -> bool {
    if mount.is_empty() {
        return true;
    }
    let lower = path.to_ascii_lowercase();
    match lower.strip_prefix(mount) {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn page() {
        let html = api_reference_html(
            &ApiReferenceOptions::new().integration("express").content(json!({ "openapi": "3.0.0", "info": { "title": "T" } })),
        );
        assert!(html.starts_with("<!doctype html>\n<html>\n  <head>\n    <title>Scalar API Reference</title>"));
        assert!(html.contains("<style type=\"text/css\">\n      /* Custom Theme */\n      \n      \n      /* basic theme */\n"));
        assert!(html.contains(
            "Scalar.createApiReference('#app', {\n        \"_integration\": \"express\",\n        \"content\": {\n          \"openapi\": \"3.0.0\",\n          \"info\": {\n            \"title\": \"T\"\n          }\n        }\n      })\n    </script>\n  </body>\n</html>"
        ));
        assert!(use_path_matches("/reference", "/Reference/x"));
        assert!(!use_path_matches("/reference", "/references"));
    }

    #[test]
    fn functions_and_theme() {
        let o = ApiReferenceOptions::new().theme("purple").set("onLoaded", json!({ "$js": "() => 1" }));
        let html = render_api_reference(&o, DEFAULT_THEME_CSS);
        assert!(!html.contains("<style"));
        assert!(html.contains("{\n        \"theme\": \"purple\",\n        \"onLoaded\": () => 1\n      }"));
    }
}
