//! Swagger UI page and init script. The templates are the exact files shipped by the Node.js
//! generator, so the HTML and JavaScript are byte-identical.

use serde_json::Value;

use crate::js::{self, Obj};

pub(crate) const HTML_TEMPLATE: &str = include_str!("assets/swagger-ui-index.html.tpl");
pub(crate) const JS_TEMPLATE: &str = include_str!("assets/swagger-ui-init.js.tpl");
pub(crate) const FAVICON_HTML: &str = include_str!("assets/swagger-ui-favicon.html.tpl");

/// Version of `swagger-ui-dist` the templates and CDN assets correspond to.
pub const SWAGGER_UI_DIST_VERSION: &str = "5.32.8";

/// Files of `swagger-ui-dist` (served at `<path>/<file>`).
pub(crate) const SWAGGER_UI_DIST_FILES: &[&str] = &[
    "absolute-path.js",
    "favicon-16x16.png",
    "favicon-32x32.png",
    "index.css",
    "index.html",
    "index.js",
    "oauth2-redirect.html",
    "oauth2-redirect.js",
    "package.json",
    "README.md",
    "NOTICE",
    "swagger-initializer.js",
    "swagger-ui-bundle.js",
    "swagger-ui-bundle.js.map",
    "swagger-ui-bundle.js.LICENSE.txt",
    "swagger-ui-es-bundle-core.js",
    "swagger-ui-es-bundle-core.js.map",
    "swagger-ui-es-bundle-core.js.LICENSE.txt",
    "swagger-ui-es-bundle.js",
    "swagger-ui-es-bundle.js.map",
    "swagger-ui-es-bundle.js.LICENSE.txt",
    "swagger-ui-standalone-preset.js",
    "swagger-ui-standalone-preset.js.map",
    "swagger-ui-standalone-preset.js.LICENSE.txt",
    "swagger-ui.css",
    "swagger-ui.css.map",
    "swagger-ui.js",
    "swagger-ui.js.map",
];

/// The HTML options of `SwaggerCustomOptions`.
#[derive(Debug, Clone)]
pub struct SwaggerUiHtmlOptions<'a> {
    pub custom_css: &'a str,
    pub custom_js: &'a [String],
    pub custom_js_str: &'a [String],
    pub custom_favicon: Option<&'a str>,
    pub custom_site_title: &'a str,
    pub custom_css_url: &'a [String],
    pub explorer: bool,
}

fn to_tags(code: &[String], f: impl Fn(&str) -> String) -> String {
    code.iter().map(|c| f(c)).collect::<Vec<_>>().join("\n")
}

/// `buildSwaggerHTML(baseUrl, customOptions)`.
pub fn build_swagger_html(base_url: &str, o: &SwaggerUiHtmlOptions<'_>) -> String {
    let favicon = match o.custom_favicon {
        Some(f) if !f.is_empty() => format!("<link rel='icon' href='{f}' />"),
        _ => FAVICON_HTML.to_string(),
    };
    let explorer_css = if o.explorer { "" } else { ".swagger-ui .topbar .download-url-wrapper { display: none }" };
    HTML_TEMPLATE
        .replacen("<% customCss %>", o.custom_css, 1)
        .replacen("<% explorerCss %>", explorer_css, 1)
        .replacen("<% favIconString %>", &favicon, 1)
        .replace("<% baseUrl %>", base_url)
        .replacen("<% customJs %>", &to_tags(o.custom_js, |u| format!("<script src='{u}'></script>")), 1)
        .replacen("<% customJsStr %>", &to_tags(o.custom_js_str, |c| format!("<script>{c}</script>")), 1)
        .replacen("<% customCssUrl %>", &to_tags(o.custom_css_url, |u| format!("<link href='{u}' rel='stylesheet'>")), 1)
        .replacen("<% title %>", o.custom_site_title, 1)
}

/// Replaces `{"$js": "<code>"}` markers (JavaScript functions in `swaggerOptions`) with
/// placeholders; returns the placeholders and their code.
fn extract_functions(v: &mut Value, fns: &mut Vec<(String, String)>) {
    match v {
        Value::Object(o) => {
            if o.len() == 1 {
                if let Some(Value::String(code)) = o.get("$js") {
                    let placeholder = format!("corvis-swagger-fn-{}", fns.len());
                    fns.push((placeholder.clone(), code.clone()));
                    *v = Value::String(placeholder);
                    return;
                }
            }
            for (_, x) in o.iter_mut() {
                extract_functions(x, fns);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| extract_functions(x, fns)),
        _ => {}
    }
}

/// `buildSwaggerInitJS(swaggerDoc, customOptions)`.
pub fn build_swagger_init_js(doc: &Value, swagger_url: Option<&str>, swagger_options: &Value) -> String {
    let mut init = Obj::new();
    init.insert("swaggerDoc".into(), doc.clone());
    if let Some(u) = swagger_url {
        init.insert("swaggerUrl".into(), js::s(u));
    }
    init.insert("customOptions".into(), if swagger_options.is_null() { Value::Object(Obj::new()) } else { swagger_options.clone() });
    let mut init = Value::Object(init);
    let mut fns = Vec::new();
    extract_functions(&mut init, &mut fns);
    let mut json = js::stringify_pretty(&init, 2);
    for (placeholder, code) in fns {
        json = json.replacen(&format!("\"{placeholder}\""), &code, 1);
    }
    JS_TEMPLATE.replacen("<% swaggerOptions %>", &format!("let options = {json};"), 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opts() -> SwaggerUiHtmlOptions<'static> {
        SwaggerUiHtmlOptions {
            custom_css: "",
            custom_js: &[],
            custom_js_str: &[],
            custom_favicon: None,
            custom_site_title: "Swagger UI",
            custom_css_url: &[],
            explorer: false,
        }
    }

    #[test]
    fn html() {
        let h = build_swagger_html("./docs/", &opts());
        assert!(h.starts_with("\n<!-- HTML for static distribution bundle build -->"));
        assert!(h.contains("<title>Swagger UI</title>"));
        assert!(h.contains(r#"<link rel="stylesheet" type="text/css" href="./docs/swagger-ui.css" >"#));
        assert!(h.contains(r#"href="./docs/favicon-32x32.png""#));
        assert!(h.contains("<script src=\"./docs/swagger-ui-init.js\"> </script>"));
        assert!(h.contains(".swagger-ui .topbar .download-url-wrapper { display: none }"));
        assert!(!h.contains("<%"));
    }

    #[test]
    fn init_js() {
        let js = build_swagger_init_js(&json!({ "openapi": "3.0.0" }), None, &json!({ "persistAuthorization": true, "f": { "$js": "function () { return 1 }" } }));
        assert!(js.contains("let options = {\n  \"swaggerDoc\": {\n    \"openapi\": \"3.0.0\"\n  },\n  \"customOptions\": {\n    \"persistAuthorization\": true,\n    \"f\": function () { return 1 }\n  }\n};"));
        assert!(js.starts_with("\nwindow.onload = function() {"));
    }
}
