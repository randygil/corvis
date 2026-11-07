---
status: documentado
---

# Errores comunes

Esta página reúne los errores que más aparecen al empezar con Corvis, con su mensaje exacto y la
forma de resolverlos.

## "Corvis can't resolve dependencies of the ..."

```text
Corvis can't resolve dependencies of the CatsService (?, ConfigService). Please make sure that the
argument API_KEY at index [0] is available in the CatsModule context.
```

Aparece al arrancar, cuando un provider o controlador pide algo que su módulo no ve. El `?` marca el
argumento que falta y el mensaje indica el token (`API_KEY` o el nombre del tipo) y el módulo.

Causas habituales:

- El provider está en otro módulo que no lo **exporta**, o ese módulo no está en los `imports` del
  que lo pide. Un módulo solo ve sus propios providers, los `exports` de sus `imports` y los de los
  módulos globales. Ver [Módulos](/es/modules).
- Un módulo global (`#[module(global, ...)]`) no exporta el provider: también los globales tienen que
  exportar lo que comparten.
- Pediste `Arc<T>` de un tipo que no está registrado en `providers` de ningún módulo.
- El token de `#[inject("TOKEN")]` no coincide con el de `Provider::use_value("TOKEN", ...)`.

Si la dependencia es opcional, decláralo con `Option<Arc<T>>`: recibirás `None` en lugar del error.

## "Corvis could not find X element"

```text
Corvis could not find CatsService element (this provider does not exist in the current context)
```

Lo devuelve una búsqueda programática (`app.get::<T>()`, `ModuleRef`, `ctx.get::<T>()` dentro de una
fábrica) cuando el token no existe en el contexto consultado.

## Dependencias circulares

```text
A circular dependency has been detected: AService -> BService -> AService. ...
```

Dos providers se piden mutuamente (directa o indirectamente). El mensaje sugiere `forwardRef()`,
pero **Corvis no tiene `forwardRef`**: hay que romper el ciclo. Opciones:

- Extraer la lógica compartida a un tercer provider del que dependan ambos.
- Comunicar uno de los lados con eventos (`EventEmitter`) en lugar de una llamada directa.
- Resolver la dependencia de forma diferida con `ModuleRef` (inyectando `module_ref: ModuleRef` y
  llamando a `self.module_ref.get::<T>()` cuando haga falta, no en el constructor).

Ver [Dependencias circulares](/es/fundamentals/circular-dependency).

## "Corvis cannot export a provider/module that is not a part of the currently processed module"

Un módulo lista en `exports` algo que no está en sus `providers` ni lo exporta ninguno de sus
`imports`, o reexporta un módulo que no importa. Añádelo a `providers` o importa el módulo
correspondiente.

## "The provider X was found but it is not of the requested type Y"

El token existe, pero el valor registrado es de otro tipo. Pasa con tokens de texto: por ejemplo,
`Provider::use_value("PORT", 3000)` registra un `i32`, y un campo `#[inject("PORT")] port: Arc<u16>`
pide un `u16`. Usa el mismo tipo en los dos lados (`Provider::use_value("PORT", 3000u16)`).

## "Error while instantiating X: ..."

Una fábrica (`Provider::use_factory`, `register_async`, `for_root_async`) devolvió un error. El
mensaje incluye la causa. Un caso típico es una variable de entorno obligatoria que falta, como
`JWT_SECRET` en `examples/tertulia`.

## Errores de compilación de los atributos

| Mensaje | Causa |
|---|---|
| `#[get] must be used on a method inside a #[routes] impl block` | El `impl` del controlador no lleva `#[routes]` |
| `#[use_guards] must be placed below #[controller] or on a method inside #[routes]` | Los atributos de clase van **debajo** de `#[controller]` (Rust procesa los atributos en orden) |
| `request-scoped providers are not supported; use transient or default scope` | `#[injectable(scope = "request")]` no existe. Ver [Scopes de inyección](/es/fundamentals/injection-scopes) |
| `unknown #[module] key; expected imports, controllers, providers, exports or global` | Una clave mal escrita en `#[module(...)]` |
| `#[corvis::main] requires an async fn` | Falta `async` en `main` |

Un campo de un `#[injectable]` que no es `Arc<T>`, `Option<Arc<T>>` ni `ModuleRef` se inicializa con
`Default`. Si su tipo no implementa `Default`, el error del compilador lo menciona: inicialízalo con
`#[init(expr)]`.

## El puerto está ocupado

```text
ERROR [CorvisApplication] listen EADDRINUSE / bind error on 0.0.0.0:3000: ...
```

Otro proceso (a menudo una ejecución anterior de la misma aplicación) escucha en ese puerto. Detenlo
o usa otro puerto.

## `404 Cannot GET /ruta`

```json
{ "message": "Cannot GET /cats", "error": "Not Found", "statusCode": 404 }
```

- Revisa el prefijo global (`set_global_prefix("api")` mueve todo a `/api/...`) y el versionado por
  URI (`/api/v1/...`). Los logs `Mapped {...}` al arrancar muestran las rutas reales.
- Comprueba que el controlador esté en `controllers` de un módulo alcanzable desde el módulo raíz.
- Con versionado por cabecera o media type, una petición sin versión solo llega a rutas sin versión
  o `VERSION_NEUTRAL`. Ver [Versionado](/es/http/versioning).

## `413`, `415` y `400` del body parser

| Respuesta | Causa |
|---|---|
| `413 request entity too large` | El JSON o urlencoded supera 100 kb. Súbelo con `app.use_body_parser("json", bytes)` |
| `415 unsupported content encoding` | El cuerpo viene comprimido (`gzip`, `br`...): no se admite |
| `415 unsupported charset "..."` | Un `charset` distinto de UTF-8 (o latin1 en urlencoded) |
| `400` con un mensaje de `JSON.parse` | JSON mal formado, o con más de 256 niveles de anidamiento |

## El handler recibe `null` en `#[body]`

El cuerpo no se parseó porque el `Content-Type` no es `application/json` ni
`application/x-www-form-urlencoded`. Revisa la cabecera del cliente, o lee los bytes con
`req.raw_body()` (ver [Cuerpo sin procesar](/es/faq/raw-body)).
