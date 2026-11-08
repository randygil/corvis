---
status: documented
---

# Migrar desde Express

La capa HTTP de Corvis reproduce deliberadamente el comportamiento de Express 5: el *body parser* por defecto, el router, el objeto `res`, las cookies, `express.static` y `multer` responden lo mismo, con las mismas cabeceras y los mismos mensajes de error. Eso hace que portar una API de Express sea sobre todo un cambio de estructura (de funciones sueltas a módulos, controladores y providers), no de comportamiento: los clientes existentes siguen funcionando.

Esta página asume que ya leíste [Primeros pasos](/es/first-steps) y [Controladores](/es/controllers).

## De rutas a controladores

```js
// Express
const router = express.Router();
router.get('/cats/:id', auth, async (req, res) => {
  const cat = await cats.find(Number(req.params.id));
  if (!cat) return res.status(404).json({ message: 'Not found' });
  res.json(cat);
});
app.use('/api', router);
```

```rust
// Corvis
#[controller("cats")]
pub struct CatsController {
    cats: Arc<CatsService>,
}

#[routes]
impl CatsController {
    #[get(":id")]
    #[use_guards(AuthGuard)]
    async fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
        self.cats.find(id).await.ok_or_else(|| NotFoundException::new("Not found"))
    }
}

// main.rs
app.set_global_prefix("api");
```

| Express | Corvis |
|---|---|
| `express.Router()` + `app.use('/cats', router)` | `#[controller("cats")]` registrado en un módulo |
| `router.get(path, handler)` | `#[get(path)]` en un método de `#[routes] impl` |
| `req.params.id`, `req.query.page`, `req.body` | `#[param("id")]`, `#[query("page")]`, `#[body]` |
| `req.headers.authorization`, `req.ip` | `#[headers("authorization")]`, `#[ip]` |
| Middleware de autenticación por ruta | [Guard](/es/guards) con `#[use_guards(...)]` |
| Validación manual o con un paquete | [Pipes](/es/pipes): `ParseIntPipe`, `ValidationPipe`, `ZodValidationPipe` |
| `res.status(201).json(x)` | Devolver `x` (un `POST` responde 201 por defecto; si no, `#[http_code(...)]`) |
| `next(err)` y middleware de errores | Devolver `Err(HttpException)` y, si hace falta, un [filtro de excepción](/es/exception-filters) |
| `app.use(prefix)` global | `app.set_global_prefix("api")` |
| Instancias compartidas en módulos de JS | [Providers](/es/providers) con inyección de dependencias |

## Middleware

Un middleware de Express de la forma `(req, res, next) => ...` se traduce en una función o un tipo que recibe la `Request` y un `Next`:

```rust
app.use_fn(|req: Request, next: Next| async move {
    let started = std::time::Instant::now();
    let res = next.run(req).await;
    Logger::new("HTTP").log(format!("{}ms", started.elapsed().as_millis()));
    res
});
```

A diferencia de Express, no hay un `res` mutable dentro del middleware: la respuesta se obtiene como resultado de `next.run(req)` y se modifica ahí. Para aplicarlo solo a algunas rutas, implementa `CorvisMiddleware` y regístralo en el `configure` del módulo con `consumer.apply::<M>().for_routes("cats")`. Ver [Middleware](/es/middleware).

## El objeto `res`

Si un handler necesita controlar la respuesta como en Express, pide `#[res] res: Res`. Sus métodos tienen los nombres de Express en snake_case:

```rust
#[get("magic")]
async fn magic(&self, #[query("token")] token: Option<String>, #[res] res: Res) {
    if let Some(s) = self.auth.redeem(token).await {
        res.set_header("Set-Cookie", format!("sid={s}; Path=/; HttpOnly"));
    }
    res.redirect_with(302, "/");
}
```

Con `#[res]` el valor de retorno se ignora y la respuesta queda abierta hasta que llamas a `send`, `json`, `end`, `redirect` o `send_status`. Con `#[res(passthrough)]` solo ajustas cabeceras o cookies y Corvis serializa el valor de retorno como siempre:

```rust
#[post("login")]
#[http_code(204)]
async fn login(&self, #[res(passthrough)] res: Res) -> HttpResult<()> {
    res.cookie("sid", &self.auth.create().await, CookieOptions::new().http_only(true).same_site("lax"));
    Ok(())
}
```

## Lo que ya se comporta como Express

| Pieza de Express | En Corvis |
|---|---|
| `express.json()` y `express.urlencoded({ extended: true })` | Activos por defecto, con el límite de 100 kB y los mismos errores `400`, `413` y `415`. `app.use_body_parser("json", bytes)` cambia el límite y `app.disable_body_parser()` los desactiva |
| `req.query` (parser `simple` de Express 5) | Igual; `app.set_query_parser(corvis::http::body_parser::QueryParser::Extended)` usa `qs` como Express 4 |
| `cookie-parser` | `req.cookies()` y `req.cookie("sid")` |
| `res.cookie`, `res.clear_cookie` | Mismos valores por defecto (`Path=/`) y misma serialización |
| `express.static` | [`corvis::serve_static(raíz)`](/es/recipes/serve-static) |
| `multer` | `FileInterceptor`, `FilesInterceptor`... (ver [Subida de archivos](/es/http/file-upload)) |
| `cors` | `app.enable_cors(CorsOptions::...)` (ver [CORS](/es/security/cors)) |
| `app.set('trust proxy', ...)` | `app.set_trust_proxy(...)` |
| `ETag` débil por defecto, `app.set('etag', ...)` | Igual; `app.set_etag(EtagMode::Strong)` o `app.disable("etag")` |
| `X-Powered-By` | `X-Powered-By: corvis` por defecto; `app.set_x_powered_by("Express")` o `app.disable("x-powered-by")` |
| Rutas sin distinguir mayúsculas y barra final opcional | Igual que el router de Express 5 |

## Diferencias a tener en cuenta

- Los cuerpos comprimidos (`Content-Encoding: gzip`, `deflate`, `br`) se rechazan con `415`; Express los descomprime.
- Un JSON con más de 256 niveles de anidamiento se rechaza con `400` (`app.set_body_parser_max_depth(n)` lo cambia).
- Un error del *body parser* se responde antes de los middlewares globales registrados con `app.use_fn`/`app.use_middleware`; en Express esos middlewares corren antes del parser.
- Los archivos de `multer` se guardan en memoria hasta que termina cada parte, así que el cuerpo multipart tiene un tope total (`MulterOptions::body_limit`, 10 MiB por defecto).
- Los handlers reciben y devuelven tipos de Rust: los pipes trabajan sobre JSON (`serde_json::Value`) y el resultado se deserializa en el tipo del parámetro.

## Orden sugerido para migrar

1. Crea el proyecto con `corvis new` y replica la configuración global (`set_global_prefix`, CORS, `trust proxy`).
2. Porta los servicios (acceso a datos, lógica) como providers con `#[injectable]`.
3. Porta cada router como un controlador, ruta por ruta, con un test de `corvis::testing` que compare la respuesta con la de la app original.
4. Convierte los middleware de autenticación en guards y la validación en DTOs con pipes.
