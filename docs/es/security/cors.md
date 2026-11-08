---
status: documented
---

# CORS

El intercambio de recursos entre orígenes (CORS) permite que un navegador llame a tu API desde una
página servida en otro origen. Corvis lo activa con `app.enable_cors(...)` y sigue la semántica del
paquete `cors` de Express: mismas opciones, mismas cabeceras y mismo tratamiento de las peticiones
*preflight*.

## Activarlo

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.enable_cors(CorsOptions::default());
```

`CorsOptions::default()` (igual que `CorsOptions::new()`) permite cualquier origen
(`Access-Control-Allow-Origin: *`) con los métodos `GET`, `HEAD`, `PUT`, `PATCH`, `POST` y `DELETE`.

También puedes pasarlo al crear la aplicación, en `CorvisApplicationOptions`:

```rust
use corvis::CorvisApplicationOptions;

let app = CorvisFactory::create_with_options::<AppModule>(CorvisApplicationOptions {
    cors: Some(CorsOptions::new().origin("https://app.example.com")),
    ..Default::default()
})
.await?;
```

## Configuración típica

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

| Método | Equivale a la opción de `cors` | Efecto |
|---|---|---|
| `.origin("https://a.com")` | `origin: 'https://a.com'` | Un origen fijo. Llamarlo dos veces arma una lista; `"*"` es cualquier origen |
| `.origins([...])` | `origin: [...]` | Lista de orígenes exactos |
| `.origin_regex(Regex::new(...)?)` | `origin: /re/` | Añade una expresión regular a la lista (usa tu propia dependencia del crate `regex`) |
| `.reflect_origin()` | `origin: true` | Refleja el origen de la petición (ver [credenciales y orígenes abiertos](#credentials-and-open-origins)) |
| `.origin_fn(\|origin\| ...)` | `origin: (origin, cb) => ...` | Decide por petición devolviendo un `CorsOrigin` |
| `.origin_value(CorsOrigin::Disabled)` | `origin: false` | No envía cabeceras CORS |
| `.methods([...])` | `methods` | `Access-Control-Allow-Methods` |
| `.allowed_headers([...])` | `allowedHeaders` | Sin valor se reflejan las cabeceras pedidas |
| `.exposed_headers([...])` | `exposedHeaders` | `Access-Control-Expose-Headers` |
| `.credentials(true)` | `credentials` | `Access-Control-Allow-Credentials: true` |
| `.max_age(segundos)` | `maxAge` | `Access-Control-Max-Age` en el preflight |
| `.preflight_continue(true)` | `preflightContinue` | Pasa el `OPTIONS` a la aplicación en lugar de responderlo |
| `.options_success_status(200)` | `optionsSuccessStatus` | Estado del preflight (204 por defecto) |

## Orígenes dinámicos

`origin_fn` recibe el origen de la petición (si lo hay) y devuelve un `CorsOrigin`:

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

Las variantes de `CorsOrigin` son `Any`, `Reflect`, `Disabled`, `Fixed(String)`, `List(...)` y
`Dynamic(...)`. `Disabled` omite las cabeceras CORS solo en esa petición.

## Comportamiento

- Cualquier `OPTIONS` se trata como preflight, aunque no traiga `Access-Control-Request-Method` y
  aunque la ruta no exista: se responde `204` con `Content-Length: 0` y las cabeceras
  `Access-Control-*` (incluida `Access-Control-Expose-Headers`). Esto ocurre antes de los middleware y de los guards. Las excepciones son
  `preflight_continue(true)` y un `origin_fn` que devuelve `CorsOrigin::Disabled` para esa petición:
  entonces el `OPTIONS` sigue a la aplicación.
- `Vary: Origin` se envía siempre que el origen configurado no sea `*`, también cuando el origen de
  la petición no está permitido (en ese caso falta `Access-Control-Allow-Origin` y el navegador
  bloquea la respuesta). Se combina con una cabecera `Vary` existente, como hace el paquete `vary`.
- Sin `allowed_headers`, se reflejan las cabeceras que pide el navegador y se agrega
  `Vary: Access-Control-Request-Headers`.
- Las cabeceras CORS se añaden también a las respuestas de error, incluidas las del body parser.
- Las cabeceras que fija el handler tienen prioridad sobre las de CORS.

## Credenciales y orígenes abiertos {#credentials-and-open-origins}

Con `credentials(true)` el navegador envía cookies. No lo combines con `reflect_origin()` u
`origin_fn` que acepten cualquier origen: cualquier sitio podría hacer peticiones autenticadas en
nombre del usuario y leer las respuestas. Usa una lista explícita con `origins([...])`.

La combinación de `reflect_origin()` y `credentials(true)` es lo que hace el paquete `cors` con
`origin: true, credentials: true`, así que Corvis la mantiene, pero registra una advertencia al
iniciar la aplicación (contexto `CorsOptions`):

```text
CORS reflects the request origin (reflect_origin()) together with credentials(true): any website can send requests with the user's cookies to this application and read the responses. Allow an explicit list of origins with origins([...]) instead. See https://corvis.randygil.com/security/cors#credentials-and-open-origins
```

Es solo una advertencia: la aplicación arranca y responde exactamente igual que antes. No hay opción
para silenciarla; para filtrarla usa un logger propio (`use_logger`) o `LoggerOption::Levels`. La
comprobación no cubre `origin_fn`, porque su resultado solo se conoce en cada petición: asegúrate de
que no devuelva `CorsOrigin::Reflect` para orígenes en los que no confías.

## WebSockets

`enable_cors` no afecta a los gateways. Para limitar los orígenes que pueden abrir un WebSocket usa
`GatewayOptions::allowed_origins` (responde `403` en el handshake). Ver
[Gateways](/es/websockets/gateways).
