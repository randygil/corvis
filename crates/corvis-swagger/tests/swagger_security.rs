//! Security regressions of `corvis-swagger`: confinement of `custom_swagger_ui_path` and the
//! content of the document inside the Scalar page and `swagger-ui-init.js`.

use std::path::PathBuf;

use corvis::prelude::*;
use corvis::testing::request;
use corvis_swagger::*;

#[module()]
pub struct AppModule;

/// `<tmp>/<unique>/{secret.env, assets/{swagger-ui.css, .env, sub/x.js}}`.
fn fixture() -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("corvis-swagger-sec-{}-{}", std::process::id(), line!()));
    let assets = base.join("assets");
    std::fs::create_dir_all(assets.join("sub")).unwrap();
    std::fs::write(base.join("secret.env"), "SECRET=outside").unwrap();
    std::fs::write(assets.join("swagger-ui.css"), "body{}").unwrap();
    std::fs::write(assets.join(".env"), "SECRET=dotfile").unwrap();
    std::fs::write(assets.join("sub").join("x.js"), "x()").unwrap();
    (base, assets)
}

/// Before: `GET /docs/C:/Windows/win.ini`, `/docs//etc/passwd` or the absolute path of any file
/// returned its content (`dir.join(rel)` with an absolute `rel` replaces `dir`).
#[corvis::test]
async fn custom_swagger_ui_path_is_confined_to_the_folder() {
    let (base, assets) = fixture();
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.silence_route_logs();
    let doc = DocumentBuilder::new().set_title("API").build();
    let opts = SwaggerCustomOptions { custom_swagger_ui_path: Some(assets.clone()), ..Default::default() };
    SwaggerModule::setup_with("docs", &mut app, doc, opts);
    app.init().await.unwrap();
    let c = request(&app);

    let r = c.get("/docs/swagger-ui.css").send().await;
    assert_eq!((r.status, r.text()), (200, "body{}".to_string()));
    let r = c.get("/docs/sub/x.js").send().await;
    assert_eq!((r.status, r.text()), (200, "x()".to_string()));

    let secret = base.join("secret.env").to_string_lossy().replace('\\', "/");
    let no_drive = secret.split_once(':').map(|(_, rest)| rest.to_string()).unwrap_or(secret.clone());
    let mut attacks = vec![
        format!("/docs/{secret}"),
        format!("/docs/{}", secret.trim_start_matches('/')),
        format!("/docs/{no_drive}"),
        format!("/docs//{}", no_drive.trim_start_matches('/')),
        "/docs/%2e%2e/secret.env".to_string(),
        "/docs/%2E%2E%2Fsecret.env".to_string(),
        "/docs/sub/%2e%2e/%2e%2e/secret.env".to_string(),
        "/docs/.env".to_string(),
        "/docs/%2eenv".to_string(),
        "/docs/sub%5C..%5C..%5Csecret.env".to_string(),
        "/docs/swagger-ui.css:stream".to_string(),
        "/docs/NUL".to_string(),
        "/docs/con.js".to_string(),
    ];
    if cfg!(windows) {
        attacks.push("/docs/C:/Windows/win.ini".into());
        attacks.push("/docs//Windows/win.ini".into());
        attacks.push("/docs/C:%5CWindows%5Cwin.ini".into());
    } else {
        attacks.push("/docs//etc/passwd".into());
    }
    for path in attacks {
        let r = c.get(&path).send().await;
        let body = r.text();
        assert_ne!(r.status, 200, "{path} -> {body}");
        assert!(!body.contains("SECRET") && !body.contains("[fonts]") && !body.contains("root:"), "{path} -> {body}");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// Before: `</script>` in any string of the document closed the Scalar `<script>` element.
#[test]
fn scalar_inline_document_cannot_close_the_script() {
    let doc = DocumentBuilder::new()
        .set_title("API")
        .set_description("</script><script>alert('scalar-xss')</script><!-- & \u{2028}\u{2029}")
        .build();
    let html = scalar::render_api_reference(&ApiReferenceOptions::new().document(&doc), "");
    assert!(!html.contains("alert('scalar-xss')</script>"), "{html}");
    assert!(!html.contains("<!-- &"));
    assert!(html.contains(r"\u003c/script\u003e\u003cscript\u003ealert('scalar-xss')\u003c/script\u003e\u003c!-- \u0026 \u2028\u2029"));
    // Still the same document once parsed as JSON / JavaScript.
    let start = html.find("\"content\": ").unwrap() + "\"content\": ".len();
    let mut de = serde_json::Deserializer::from_str(&html[start..]).into_iter::<Value>();
    let content = de.next().unwrap().unwrap();
    assert_eq!(content["info"]["description"], json!("</script><script>alert('scalar-xss')</script><!-- & \u{2028}\u{2029}"));
}

/// Configuration functions (`{"$js": ...}`) of the developer still work in Scalar.
#[test]
fn scalar_configuration_functions_are_kept() {
    let html = scalar::render_api_reference(&ApiReferenceOptions::new().set("onLoaded", json!({ "$js": "() => console.log('<ok>')" })), "");
    assert!(html.contains("\"onLoaded\": () => console.log('<ok>')"), "{html}");
}

/// Before: `{"$js": "..."}` anywhere in the document became JavaScript in `swagger-ui-init.js`.
#[test]
fn swagger_init_js_only_runs_functions_from_swagger_options() {
    let doc = json!({
        "openapi": "3.0.0",
        "info": { "title": "API", "x-example": { "$js": "alert('swagger-init-js-injection')" }, "x-fake": "corvis-swagger-fn-0" }
    });
    let opts = json!({ "onComplete": { "$js": "function () { window.ok = 1 }" } });
    let js = swagger_ui::build_swagger_init_js(&doc, None, &opts);
    assert!(js.contains(r#""x-example": {"#) && js.contains(r#""$js": "alert('swagger-init-js-injection')""#), "{js}");
    assert!(!js.contains(": alert('swagger-init-js-injection')"), "{js}");
    assert!(js.contains(r#""x-fake": "corvis-swagger-fn-0""#), "{js}");
    assert!(js.contains(r#""onComplete": function () { window.ok = 1 }"#), "{js}");
}
