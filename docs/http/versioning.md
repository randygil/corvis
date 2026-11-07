---
status: documentado
---

# Versionado

El versionado permite publicar varias versiones de una misma ruta a la vez, por ejemplo `/v1/cats/1`
y `/v2/cats/1`, para cambiar un contrato sin romper a los clientes existentes. Se activa una vez en
la aplicación y luego cada controlador o handler declara su versión.

## Activarlo

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.set_global_prefix("api");
app.enable_versioning(VersioningOptions::uri().default_version("1"));
```

Hay tres estrategias:

| Estrategia | Constructor | La versión viaja en |
|---|---|---|
| URI | `VersioningOptions::uri()` | La ruta: `/api/v1/cats` (prefijo `v` por defecto, configurable con `.prefix("version-")`) |
| Cabecera | `VersioningOptions::header("Custom-Header")` | Una cabecera: `Custom-Header: 2` |
| Media type | `VersioningOptions::media_type("v=")` | El `Accept`: `Accept: application/json;v=2` |

`.default_version(...)` asigna una versión a los controladores y rutas que no declaran ninguna.

No existe una estrategia personalizada (una función que extrae la versión de la request).

## Declarar versiones

En el controlador, con la opción `version` de `#[controller]` o con `#[version]` debajo de él:

```rust
#[controller(path = "cats", version = "1")]
pub struct CatsController { /* ... */ }
```

En un handler, con `#[version]`. Un handler sin versión hereda la del controlador y, si tampoco la
tiene, la versión por defecto:

```rust
#[routes]
impl CatsController {
    #[get(":id")]
    fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
        self.cats_service.find_one(id)
    }

    #[get(":id")]
    #[version("2")]
    fn find_one_v2(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Value> {
        let cat = self.cats_service.find_one(id)?;
        Ok(json!({ "cat": cat, "apiVersion": 2 }))
    }
}
```

Con `VersioningOptions::uri().default_version("1")`, el ejemplo anterior publica
`GET /api/v1/cats/:id` y `GET /api/v2/cats/:id`.

Una ruta puede responder a varias versiones: `#[version("1", "2")]` o `#[version(["1", "2"])]`.

## Rutas sin versión

`VERSION_NEUTRAL` marca una ruta que no depende de la versión. Con la estrategia URI se publica sin
el segmento de versión; con cabecera o media type responde a cualquier versión pedida y a las
peticiones que no indican ninguna.

```rust
#[get("health")]
#[version(VERSION_NEUTRAL)]
async fn health(&self) -> HttpResult<Value> {
    // GET /api/health
}
```

## Cómo se elige la ruta con cabecera o media type

Con estas estrategias todas las versiones comparten la misma ruta, y Corvis elige el handler en cada
petición:

- Si la petición indica una versión, gana la ruta que la declara; si no hay ninguna, la ruta
  `VERSION_NEUTRAL`.
- Si no indica ninguna, solo responden las rutas sin versión o con `VERSION_NEUTRAL`. Ten en cuenta
  que `default_version` asigna versión a todas las rutas que no declaran otra, así que, con una
  versión por defecto, una petición sin cabecera recibe `404` salvo en las rutas neutrales.
- Si ninguna encaja, la respuesta es `404 Cannot GET /ruta`.

## Logs de arranque

Al registrar las rutas, el log indica la versión de cada una:

```text
LOG [RoutesResolver] CatsController {/api/v1/cats}: (version: 1)
LOG [RouterExplorer] Mapped {/api/v2/cats/:id, GET} route (version: 2)
```
