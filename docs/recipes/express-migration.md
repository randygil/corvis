---
status: documented
---

# Migrating from Express

Corvis's HTTP layer deliberately reproduces Express 5 behavior: the default *body parser*, the router, the `res` object, cookies, `express.static` and `multer` respond the same way, with the same headers and the same error messages. That makes porting an Express API mostly a change of structure (from loose functions to modules, controllers and providers), not of behavior: existing clients keep working.

This page assumes you have already read [First steps](/first-steps) and [Controllers](/controllers).

## From routes to controllers

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
| `express.Router()` + `app.use('/cats', router)` | `#[controller("cats")]` registered in a module |
| `router.get(path, handler)` | `#[get(path)]` on a method of a `#[routes] impl` |
| `req.params.id`, `req.query.page`, `req.body` | `#[param("id")]`, `#[query("page")]`, `#[body]` |
| `req.headers.authorization`, `req.ip` | `#[headers("authorization")]`, `#[ip]` |
| Per-route authentication middleware | [Guard](/guards) with `#[use_guards(...)]` |
| Manual validation or with a package | [Pipes](/pipes): `ParseIntPipe`, `ValidationPipe`, `ZodValidationPipe` |
| `res.status(201).json(x)` | Return `x` (a `POST` responds 201 by default; otherwise, `#[http_code(...)]`) |
| `next(err)` and error middleware | Return `Err(HttpException)` and, if needed, an [exception filter](/exception-filters) |
| Global `app.use(prefix)` | `app.set_global_prefix("api")` |
| Shared instances in JS modules | [Providers](/providers) with dependency injection |

## Middleware

An Express middleware of the form `(req, res, next) => ...` translates into a function or a type that receives the `Request` and a `Next`:

```rust
app.use_fn(|req: Request, next: Next| async move {
    let started = std::time::Instant::now();
    let res = next.run(req).await;
    Logger::new("HTTP").log(format!("{}ms", started.elapsed().as_millis()));
    res
});
```

Unlike Express, there is no mutable `res` inside the middleware: the response is obtained as the result of `next.run(req)` and modified there. To apply it to only some routes, implement `CorvisMiddleware` and register it in the module's `configure` with `consumer.apply::<M>().for_routes("cats")`. See [Middleware](/middleware).

## The `res` object

If a handler needs to control the response as in Express, ask for `#[res] res: Res`. Its methods have the Express names in snake_case:

```rust
#[get("magic")]
async fn magic(&self, #[query("token")] token: Option<String>, #[res] res: Res) {
    if let Some(s) = self.auth.redeem(token).await {
        res.set_header("Set-Cookie", format!("sid={s}; Path=/; HttpOnly"));
    }
    res.redirect_with(302, "/");
}
```

With `#[res]` the return value is ignored and the response stays open until you call `send`, `json`, `end`, `redirect` or `send_status`. With `#[res(passthrough)]` you only adjust headers or cookies and Corvis serializes the return value as usual:

```rust
#[post("login")]
#[http_code(204)]
async fn login(&self, #[res(passthrough)] res: Res) -> HttpResult<()> {
    res.cookie("sid", &self.auth.create().await, CookieOptions::new().http_only(true).same_site("lax"));
    Ok(())
}
```

## What already behaves like Express

| Express piece | In Corvis |
|---|---|
| `express.json()` and `express.urlencoded({ extended: true })` | Enabled by default, with the 100 kB limit and the same `400`, `413` and `415` errors. `app.use_body_parser("json", bytes)` changes the limit and `app.disable_body_parser()` turns them off |
| `req.query` (Express 5 `simple` parser) | Same (`a[]=1` keeps the key `a[]`, at most 1000 pairs); `app.set_query_parser(corvis::http::body_parser::QueryParser::Extended)` uses `qs` like Express 4 |
| `cookie-parser` | `req.cookies()` and `req.cookie("sid")` |
| `res.cookie`, `res.clear_cookie` | Same defaults (`Path=/`) and same serialization |
| `express.static` | [`corvis::serve_static(root)`](/recipes/serve-static) |
| `multer` | `FileInterceptor`, `FilesInterceptor`... (see [File upload](/http/file-upload)) |
| `cors` | `app.enable_cors(CorsOptions::...)` (see [CORS](/security/cors)) |
| `app.set('trust proxy', ...)` | `app.set_trust_proxy(...)` |
| `req.hostname` | `req.hostname()`; keeps IPv6 literals (`Host: [::1]:3000` gives `[::1]`) |
| Weak `ETag` by default, `app.set('etag', ...)` | Same; `app.set_etag(EtagMode::Strong)` or `app.disable("etag")` |
| `X-Powered-By` | `X-Powered-By: corvis` by default; `app.set_x_powered_by("Express")` or `app.disable("x-powered-by")` |
| Case-insensitive routes and optional trailing slash | Same as the Express 5 router |

## Differences to keep in mind

- Compressed bodies (`Content-Encoding: gzip`, `deflate`, `br`) are rejected with `415`; Express decompresses them.
- JSON nested more than 256 levels deep is rejected with `400` (`app.set_body_parser_max_depth(n)` changes it).
- A *body parser* error is answered before the global middlewares registered with `app.use_fn`/`app.use_middleware`; in Express those middlewares run before the parser.
- `multer` files are kept in memory until each part finishes, so the multipart body has a total cap (`MulterOptions::body_limit`, 10 MiB by default).
- Handlers receive and return Rust types: pipes work on JSON (`serde_json::Value`) and the result is deserialized into the parameter's type.

## Suggested migration order

1. Create the project with `corvis new` and replicate the global configuration (`set_global_prefix`, CORS, `trust proxy`).
2. Port the services (data access, logic) as providers with `#[injectable]`.
3. Port each router as a controller, route by route, with a `corvis::testing` test that compares the response with the original app's.
4. Turn the authentication middleware into guards and the validation into DTOs with pipes.
