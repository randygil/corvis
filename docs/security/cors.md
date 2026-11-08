---
status: documented
---

# CORS

Cross-origin resource sharing (CORS) lets a browser call your API from a
page served on another origin. Corvis enables it with `app.enable_cors(...)` and follows the semantics of Express's
`cors` package: same options, same headers and same handling of *preflight*
requests.

## Enabling it

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.enable_cors(CorsOptions::default());
```

`CorsOptions::default()` (same as `CorsOptions::new()`) allows any origin
(`Access-Control-Allow-Origin: *`) with the methods `GET`, `HEAD`, `PUT`, `PATCH`, `POST` and `DELETE`.

You can also pass it when creating the application, in `CorvisApplicationOptions`:

```rust
use corvis::CorvisApplicationOptions;

let app = CorvisFactory::create_with_options::<AppModule>(CorvisApplicationOptions {
    cors: Some(CorsOptions::new().origin("https://app.example.com")),
    ..Default::default()
})
.await?;
```

## Typical configuration

```rust
app.enable_cors(
    CorsOptions::new()
        .origins(["https://app.example.com", "http://localhost:5173"])
        .methods(["GET", "HEAD", "POST", "PATCH", "PUT", "DELETE", "OPTIONS"])
        .allowed_headers(["Content-Type", "Authorization"])
        .exposed_headers(["Content-Disposition"])
        .credentials(true)
        .max_age(86_400),
);
```

| Method | Equivalent `cors` option | Effect |
|---|---|---|
| `.origin("https://a.com")` | `origin: 'https://a.com'` | A fixed origin. Calling it twice builds a list; `"*"` is any origin |
| `.origins([...])` | `origin: [...]` | List of exact origins |
| `.origin_regex(Regex::new(...)?)` | `origin: /re/` | Adds a regular expression to the list (use your own dependency on the `regex` crate) |
| `.reflect_origin()` | `origin: true` | Reflects the request's origin (see [credentials and open origins](#credentials-and-open-origins)) |
| `.origin_fn(\|origin\| ...)` | `origin: (origin, cb) => ...` | Decides per request by returning a `CorsOrigin` |
| `.origin_value(CorsOrigin::Disabled)` | `origin: false` | Does not send CORS headers |
| `.methods([...])` | `methods` | `Access-Control-Allow-Methods` |
| `.allowed_headers([...])` | `allowedHeaders` | Without a value, the requested headers are reflected |
| `.exposed_headers([...])` | `exposedHeaders` | `Access-Control-Expose-Headers` |
| `.credentials(true)` | `credentials` | `Access-Control-Allow-Credentials: true` |
| `.max_age(seconds)` | `maxAge` | `Access-Control-Max-Age` in the preflight |
| `.preflight_continue(true)` | `preflightContinue` | Passes the `OPTIONS` to the application instead of answering it |
| `.options_success_status(200)` | `optionsSuccessStatus` | Preflight status (204 by default) |

## Dynamic origins

`origin_fn` receives the request's origin (if any) and returns a `CorsOrigin`:

```rust
use corvis::CorsOrigin;

app.enable_cors(
    CorsOptions::new()
        .origin_fn(|origin| match origin {
            Some(o) if o.ends_with(".example.com") => CorsOrigin::Reflect,
            _ => CorsOrigin::Disabled,
        })
        .credentials(true),
);
```

The `CorsOrigin` variants are `Any`, `Reflect`, `Disabled`, `Fixed(String)`, `List(...)` and
`Dynamic(...)`. `Disabled` omits the CORS headers for that request only.

## Behavior

- Any `OPTIONS` is treated as a preflight, even if it carries no `Access-Control-Request-Method` and
  even if the route does not exist: `204` is returned with `Content-Length: 0` and the
  `Access-Control-*` headers (including `Access-Control-Expose-Headers`). This happens before middleware and guards. The exceptions are
  `preflight_continue(true)` and an `origin_fn` that returns `CorsOrigin::Disabled` for that request:
  in those cases the `OPTIONS` continues to the application.
- `Vary: Origin` is always sent when the configured origin is not `*`, also when the request's
  origin is not allowed (in that case `Access-Control-Allow-Origin` is missing and the browser
  blocks the response). It is merged with an existing `Vary` header, like the `vary` package does.
- Without `allowed_headers`, the headers requested by the browser are reflected and
  `Vary: Access-Control-Request-Headers` is added.
- CORS headers are also added to error responses, including those from the body parser.
- Headers set by the handler take priority over those from CORS.

## Credentials and open origins {#credentials-and-open-origins}

With `credentials(true)` the browser sends cookies. Do not combine it with `reflect_origin()` or an
`origin_fn` that accepts any origin: any site could make authenticated requests on
behalf of the user and read the responses. Use an explicit list with `origins([...])`.

The combination of `reflect_origin()` and `credentials(true)` is what the `cors` package does with
`origin: true, credentials: true`, so Corvis keeps it, but it logs one warning when the application
starts (context `CorsOptions`):

```text
CORS reflects the request origin (reflect_origin()) together with credentials(true): any website can send requests with the user's cookies to this application and read the responses. Allow an explicit list of origins with origins([...]) instead. See https://corvis.randygil.com/security/cors#credentials-and-open-origins
```

It is only a warning: the application starts and answers exactly as before. There is no option to
silence it; to filter it, use a custom logger (`use_logger`) or `LoggerOption::Levels`. The check does
not cover `origin_fn`, because its result is only known per request: make sure it does not return
`CorsOrigin::Reflect` for origins you do not trust.

## WebSockets

`enable_cors` does not affect gateways. To limit the origins that can open a WebSocket use
`GatewayOptions::allowed_origins` (responds `403` on the handshake). See
[Gateways](/websockets/gateways).
