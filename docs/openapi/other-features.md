---
status: documented
---

# Other features

## Global prefix

If the application uses `app.set_global_prefix("api")`, the document's paths include it (`/api/cats`). To leave it out:

```rust
use corvis_swagger::SwaggerDocumentOptions;

let options = SwaggerDocumentOptions { ignore_global_prefix: true, ..Default::default() };
let document = SwaggerModule::create_document_with(&app, &config, &options);
```

The path where the UI is mounted does not carry the prefix, unless you ask for it with `use_global_prefix: true` in `SwaggerCustomOptions`.

Create the document **after** configuring the prefix and versioning, because `create_document` reads that configuration from the application.

## Versioning

With `app.enable_versioning(VersioningOptions::uri())`, each version of a route appears as a distinct path (`/v1/cats`, `/v2/cats`) and the `operationId` carries the version at the end. See [Versioning](/http/versioning).

## Multiple documents

`include` limits the document to the controllers of certain modules. You can mount one document per area:

```rust
use std::any::TypeId;

let public = SwaggerModule::create_document_with(&app, &public_config, &SwaggerDocumentOptions {
    include: vec![TypeId::of::<CatsModule>()],
    ..Default::default()
});
let admin = SwaggerModule::create_document_with(&app, &admin_config, &SwaggerDocumentOptions {
    include: vec![TypeId::of::<AdminModule>()],
    ..Default::default()
});
SwaggerModule::setup("docs", &mut app, cleanup_open_api_doc(public));
SwaggerModule::setup("admin/docs", &mut app, cleanup_open_api_doc(admin));
```

By default only the controllers of the included modules are walked; with `deep_scan_routes: true` those of the modules they import are walked too.

## Document options

| `SwaggerDocumentOptions` | Default | Effect |
|---|---|---|
| `include` | All modules | Modules whose controllers are documented |
| `deep_scan_routes` | `false` | Also include the modules imported by those in `include` |
| `extra_models` | Empty | Additional models in `components.schemas` |
| `ignore_global_prefix` | `false` | Remove the global prefix from the paths |
| `operation_id_factory` | `<Controller>_<method>[_<version>]` | `(controller, method, version)` function that returns the `operationId` |
| `method_key_factory` | snake_case → camelCase | How the Rust method name is converted |
| `auto_tag_controllers` | `true` | Automatic tag for controllers without `#[api_tags]` |
| `only_include_decorated_endpoints` | `false` | Document only the handlers with `#[api_include_endpoint]` |
| `example_max_depth` | No limit | Maximum depth when generating examples |

```rust
use std::sync::Arc;

let options = SwaggerDocumentOptions {
    operation_id_factory: Some(Arc::new(|_controller, method, _version| method.to_string())),
    ..Default::default()
};
```

## More data in `DocumentBuilder`

| Method | Effect |
|---|---|
| `set_title`, `set_description`, `set_version`, `set_terms_of_service` | `info` block |
| `set_contact(name, url, email)`, `set_license(name, url)` | Contact and license |
| `set_open_api_version("3.1.0")` | The document's OpenAPI version (`3.0.0` by default) |
| `add_server(url, description, variables)` | Servers |
| `add_tag(name, description)`, `add_tag_with(...)` | Tags with a description |
| `set_external_doc(description, url)` | External documentation |
| `add_extension("x-key", value, "root")` | Extension at the document root (or another key as the location); the key must start with `x-` |
| `add_global_response(...)`, `add_global_parameters(...)` | Responses and parameters common to all operations |
| `add_bearer_auth()`, `add_security(...)`... | See [Security](/openapi/security) |

## Swagger UI options

`SwaggerModule::setup_with` accepts `SwaggerCustomOptions`:

```rust
use corvis_swagger::SwaggerCustomOptions;

SwaggerModule::setup_with("docs", &mut app, document, SwaggerCustomOptions {
    custom_site_title: "Tertulia API".into(),
    json_document_url: Some("openapi.json".into()),
    swagger_options: json!({ "persistAuthorization": true }),
    ..Default::default()
});
```

| Field | Effect |
|---|---|
| `ui` | `false` to serve only the JSON and YAML |
| `raw` | `None` serves JSON and YAML; `Some(vec!["json".into()])` only JSON; `Some(vec![])` neither |
| `json_document_url`, `yaml_document_url` | Paths of the documents (`<path>-json` and `<path>-yaml` by default) |
| `use_global_prefix` | Prepend the global prefix to the UI path |
| `swagger_options` | Swagger UI options. A function is written as `{"$js": "function () { ... }"}` |
| `custom_css`, `custom_css_url`, `custom_js`, `custom_js_str`, `custom_favicon`, `custom_site_title` | Page customization |
| `explorer` | Show the bar for loading another document |
| `custom_swagger_ui_path` | Local folder with `swagger-ui-dist`; without it, resources are redirected to jsDelivr |
| `patch_document_on_request` | Function that receives the `Request` and the document and returns the document to serve |

`custom_swagger_ui_path` is the option to use if the server has no internet access or your security policy does not allow loading scripts from a CDN. Only files inside that folder are served, as `express.static` does: the path is decoded and every segment must be a plain file name (no empty segments, `.`/`..`, dotfiles, `\`, `:`, control characters or Windows device names such as `CON` or `NUL`), and the canonical path (symbolic links and 8.3 short names resolved) must still be inside the folder and be a file. Anything else falls through to the next handler (`404`).

The `{"$js": "..."}` marker is only turned into code inside `swagger_options`; inside the document it is plain data.

## Lazy document

`SwaggerModule::setup_lazy` takes a function instead of the document and builds it on the first request:

```rust
SwaggerModule::setup_lazy("docs", &mut app, move || build_document(), SwaggerCustomOptions::default());
```

## Exporting the document

`OpenApiDocument` can be written to disk, for example to generate clients in continuous integration:

```rust
std::fs::write("openapi.json", document.to_json_pretty())?;
std::fs::write("openapi.yaml", document.to_yaml())?;
```

`document.as_value()` and `document.as_value_mut()` give access to the JSON for manual tweaks.
