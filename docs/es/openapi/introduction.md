---
status: documented
---

# Introducción

[OpenAPI](https://swagger.io/specification/) es un formato estándar para describir APIs HTTP. El crate `corvis-swagger` genera ese documento a partir de tus controladores, lo sirve en JSON y YAML y monta dos interfaces para explorarlo: Swagger UI y la referencia de Scalar.

La información sale de tres fuentes:

- **las rutas**: métodos, rutas, versiones y parámetros (`#[param]`, `#[query]`, `#[body]`, `#[headers]`);
- **los tipos de los parámetros**: DTOs con schema de `corvis::z` (`zod_dto!`) o con `#[derive(ApiSchema)]`;
- **los atributos `#[api_*]`** que añades a controladores y handlers: tags, descripciones, respuestas, seguridad.

## Instalación

Agrega el crate a tu proyecto (todavía no está en crates.io, así que se usa una ruta local):

```toml
[dependencies]
corvis = { path = "../corvis/crates/corvis" }
corvis-swagger = { path = "../corvis/crates/corvis-swagger" }
```

Los atributos `#[api_*]` son macros de `corvis-swagger`; para que estén disponibles, importa el crate en cada archivo que los use:

```rust
use corvis_swagger::*;
```

## Bootstrap

Después de crear la aplicación, construye el documento y móntalo:

```rust
use corvis::prelude::*;
use corvis_swagger::*;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.set_global_prefix("api");

    let config = DocumentBuilder::new()
        .set_title("Cats")
        .set_description("La API de los gatos")
        .set_version("1.0")
        .add_tag("cats", "Operaciones sobre gatos")
        .add_bearer_auth()
        .build();
    let document = cleanup_open_api_doc(SwaggerModule::create_document(&app, &config));
    SwaggerModule::setup("docs", &mut app, document);

    app.listen(3000).await
}
```

Con la aplicación en marcha:

| Ruta | Contenido |
|---|---|
| `http://localhost:3000/docs` | Swagger UI |
| `http://localhost:3000/docs-json` | El documento en JSON |
| `http://localhost:3000/docs-yaml` | El documento en YAML |

`DocumentBuilder` define la parte fija del documento (título, versión, servidores, esquemas de seguridad). `SwaggerModule::create_document` recorre los controladores de la aplicación y completa `paths` y `components.schemas`. `cleanup_open_api_doc` convierte los schemas de los DTOs de `corvis::z` al formato de OpenAPI; si no usas esos DTOs no hace falta, pero tampoco molesta.

::: tip Rutas planas
Las rutas que monta `SwaggerModule::setup` (la página, `swagger-ui-init.js`, `-json` y `-yaml`, más los archivos de `swagger-ui-dist`) son rutas planas al estilo de Express, no rutas de controlador: no aparecen en los logs `Mapped {...}` ni pasan por guards, pipes o interceptors. Si la documentación no debe ser pública, protégela con un middleware o no la montes en producción.
:::

## Un controlador documentado

```rust
use corvis::prelude::*;
use corvis::zod_dto;
use corvis_swagger::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateCatDto {
    pub name: String,
    pub lives: i64,
}
zod_dto!(CreateCatDto, z::object! { name: z::string().min(1), lives: z::int().min(1).default(9) });

#[derive(Debug, Deserialize, Serialize)]
pub struct CatDto {
    pub id: String,
    pub name: String,
}
zod_dto!(CatDto, z::object! { id: z::uuid(), name: z::string() });

#[controller("cats")]
#[api_tags("cats")]
#[api_bearer_auth]
#[api_unauthorized_response(description = "Missing token")]
pub struct CatsController;

#[routes]
impl CatsController {
    #[post]
    #[api_operation(summary = "Create a cat")]
    #[api_created_response(type = CatDto)]
    #[api_conflict_response(description = "Duplicated")]
    fn create(&self, #[body] dto: CreateCatDto) -> Value {
        json!({ "id": "00000000-0000-4000-8000-000000000000", "name": dto.name })
    }

    #[get]
    #[api_ok_response(type = [CatDto])]
    fn list(&self) -> Value {
        json!([])
    }
}
```

Este controlador es el de `crates/corvis-swagger/tests/swagger_app.rs`, que comprueba el documento generado campo por campo. El `POST /api/cats` queda así:

```json
{
  "operationId": "CatsController_create",
  "parameters": [],
  "requestBody": {
    "required": true,
    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateCatDto" } } }
  },
  "responses": {
    "201": { "description": "", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CatDto" } } } },
    "401": { "description": "Missing token" },
    "409": { "description": "Duplicated" }
  },
  "security": [{ "bearer": [] }],
  "summary": "Create a cat",
  "tags": ["cats"]
}
```

Fíjate en que los atributos van **debajo** de `#[controller]` y de `#[get]`/`#[post]`, en el orden en que quieres que aparezcan las respuestas.

## Referencia de Scalar

Además de Swagger UI puedes montar la referencia de [Scalar](https://scalar.com):

```rust
scalar::mount(&mut app, "/reference", ApiReferenceOptions::new().document(&document));
```

`mount` responde la página en esa ruta y en todo lo que cuelga de ella, para cualquier método. `ApiReferenceOptions` permite cambiar el tema (`.theme("purple")`), el título (`.page_title(..)`), el CDN (`.cdn(..)`), un `nonce` para CSP o cualquier clave de la configuración de Scalar con `.set(clave, valor)`. En lugar del documento embebido puedes apuntar a una URL con `.url("/docs-json")`.

La página es la que genera `@scalar/client-side-rendering` 0.3, con su tema por defecto salvo que indiques `.theme(..)`. No envía el campo `_integration` de las integraciones de servidor de Scalar; si lo necesitas (por ejemplo, para servir una página idéntica a una existente), indícalo con `ApiReferenceOptions::integration("...")`. El JSON de configuración embebido en la página (que incluye el documento) escapa `<`, `>`, `&`, U+2028 y U+2029, así que ningún texto del documento puede cerrar el `<script>`. Los valores `{"$js": "..."}` indicados con `.set(..)` se emiten como código, porque los escribe el desarrollador.

## Siguientes pasos

- [Tipos y parámetros](/es/openapi/types-and-parameters): cómo se describen los DTOs y los parámetros.
- [Operaciones](/es/openapi/operations): respuestas, cuerpos, tipos de contenido.
- [Seguridad](/es/openapi/security): esquemas de autenticación.
- [Decoradores](/es/openapi/decorators): la lista completa de atributos.
- [Otras características](/es/openapi/other-features): prefijo global, varios documentos, opciones de Swagger UI.
