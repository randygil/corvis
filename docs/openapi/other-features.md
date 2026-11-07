---
status: documentado
---

# Otras características

## Prefijo global

Si la aplicación usa `app.set_global_prefix("api")`, las rutas del documento lo incluyen (`/api/cats`). Para dejarlo fuera:

```rust
use corvis_swagger::SwaggerDocumentOptions;

let options = SwaggerDocumentOptions { ignore_global_prefix: true, ..Default::default() };
let document = SwaggerModule::create_document_with(&app, &config, &options);
```

La ruta donde se monta la UI no lleva el prefijo, salvo que lo pidas con `use_global_prefix: true` en `SwaggerCustomOptions`.

Crea el documento **después** de configurar el prefijo y el versionado, porque `create_document` lee esa configuración de la aplicación.

## Versionado

Con `app.enable_versioning(VersioningOptions::uri())`, cada versión de una ruta aparece como una ruta distinta (`/v1/cats`, `/v2/cats`) y el `operationId` lleva la versión al final. Ver [Versionado](/http/versioning).

## Varios documentos

`include` limita el documento a los controladores de ciertos módulos. Puedes montar un documento por área:

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

Por defecto solo se recorren los controladores de los módulos incluidos; con `deep_scan_routes: true` también los de los módulos que importan.

## Opciones del documento

| `SwaggerDocumentOptions` | Por defecto | Efecto |
|---|---|---|
| `include` | Todos los módulos | Módulos cuyos controladores se documentan |
| `deep_scan_routes` | `false` | Incluir también los módulos importados por los de `include` |
| `extra_models` | Vacío | Modelos adicionales en `components.schemas` |
| `ignore_global_prefix` | `false` | Quitar el prefijo global de las rutas |
| `operation_id_factory` | `<Controlador>_<método>[_<versión>]` | Función `(controlador, método, versión)` que devuelve el `operationId` |
| `method_key_factory` | snake_case → camelCase | Cómo se convierte el nombre del método de Rust |
| `auto_tag_controllers` | `true` | Tag automático para controladores sin `#[api_tags]` |
| `only_include_decorated_endpoints` | `false` | Documentar solo los handlers con `#[api_include_endpoint]` |
| `example_max_depth` | Sin límite | Profundidad máxima al generar ejemplos |

```rust
use std::sync::Arc;

let options = SwaggerDocumentOptions {
    operation_id_factory: Some(Arc::new(|_controller, method, _version| method.to_string())),
    ..Default::default()
};
```

## Más datos en `DocumentBuilder`

| Método | Efecto |
|---|---|
| `set_title`, `set_description`, `set_version`, `set_terms_of_service` | Bloque `info` |
| `set_contact(nombre, url, email)`, `set_license(nombre, url)` | Contacto y licencia |
| `set_open_api_version("3.1.0")` | Versión de OpenAPI del documento (por defecto `3.0.0`) |
| `add_server(url, descripción, variables)` | Servidores |
| `add_tag(nombre, descripción)`, `add_tag_with(...)` | Tags con descripción |
| `set_external_doc(descripción, url)` | Documentación externa |
| `add_extension("x-clave", valor, "root")` | Extensión en la raíz del documento (u otra clave como ubicación); la clave debe empezar con `x-` |
| `add_global_response(...)`, `add_global_parameters(...)` | Respuestas y parámetros comunes a todas las operaciones |
| `add_bearer_auth()`, `add_security(...)`... | Ver [Seguridad](/openapi/security) |

## Opciones de Swagger UI

`SwaggerModule::setup_with` acepta `SwaggerCustomOptions`:

```rust
use corvis_swagger::SwaggerCustomOptions;

SwaggerModule::setup_with("docs", &mut app, document, SwaggerCustomOptions {
    custom_site_title: "Tertulia API".into(),
    json_document_url: Some("openapi.json".into()),
    swagger_options: json!({ "persistAuthorization": true }),
    ..Default::default()
});
```

| Campo | Efecto |
|---|---|
| `ui` | `false` para servir solo el JSON y el YAML |
| `raw` | `None` sirve JSON y YAML; `Some(vec!["json".into()])` solo JSON; `Some(vec![])` ninguno |
| `json_document_url`, `yaml_document_url` | Rutas de los documentos (por defecto `<ruta>-json` y `<ruta>-yaml`) |
| `use_global_prefix` | Anteponer el prefijo global a la ruta de la UI |
| `swagger_options` | Opciones de Swagger UI. Una función se escribe como `{"$js": "function () { ... }"}` |
| `custom_css`, `custom_css_url`, `custom_js`, `custom_js_str`, `custom_favicon`, `custom_site_title` | Personalización de la página |
| `explorer` | Mostrar la barra para cargar otro documento |
| `custom_swagger_ui_path` | Carpeta local con `swagger-ui-dist`; sin ella, los recursos se redirigen a jsDelivr |
| `patch_document_on_request` | Función que recibe la `Request` y el documento y devuelve el documento a servir |

`custom_swagger_ui_path` es la opción a usar si el servidor no tiene salida a internet o tu política de seguridad no permite cargar scripts de un CDN. Solo se sirven archivos que están dentro de esa carpeta.

## Documento perezoso

`SwaggerModule::setup_lazy` recibe una función en lugar del documento y lo construye en la primera petición:

```rust
SwaggerModule::setup_lazy("docs", &mut app, move || build_document(), SwaggerCustomOptions::default());
```

## Exportar el documento

`OpenApiDocument` se puede escribir a disco, por ejemplo para generar clientes en la integración continua:

```rust
std::fs::write("openapi.json", document.to_json_pretty())?;
std::fs::write("openapi.yaml", document.to_yaml())?;
```

`document.as_value()` y `document.as_value_mut()` dan acceso al JSON para retoques manuales.
