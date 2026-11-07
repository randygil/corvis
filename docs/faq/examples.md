---
status: documentado
---

# Ejemplos

El repositorio incluye tres aplicaciones completas en `examples/`. Forman parte del workspace de
Cargo, así que se ejecutan desde la raíz del repositorio con `cargo run -p <nombre>`.

## `cats-app`: el recorrido completo

```bash
cargo run -p cats-app        # http://127.0.0.1:3000/api/v1
```

Una API de gatos que usa casi todas las piezas del framework en un solo proyecto:

- Módulos con `imports`/`exports` (`AuthModule`, `UsersModule`, `CatsModule`) y módulos incluidos:
  `ConfigModule`, `EventEmitterModule`, `ScheduleModule`, `CacheModule`, `ThrottlerModule` y
  `TerminusModule`.
- Autenticación JWT con un `AuthGuard` global, rutas públicas con `#[set_metadata(IS_PUBLIC_KEY, true)]`
  y un `RolesGuard` global (ver [Autenticación](/security/authentication) y
  [Autorización](/security/authorization)).
- `ValidationPipe` global con DTOs `#[derive(Validate)]`, un filtro de excepciones propio,
  interceptors de tiempo y de transformación, y un middleware de logging aplicado con
  `MiddlewareConsumer`.
- Versionado por URI (`/api/v1/cats/:id` y `/api/v2/cats/:id`), un endpoint `VERSION_NEUTRAL`,
  caché con `CacheInterceptor`, SSE en `/api/v1/cats/stream`, eventos con `#[on_event]`, tareas con
  `#[cron]` y `#[timeout]`, health checks y un gateway WebSocket en `/chat`.

Las variables de entorno están en `examples/cats-app/.env.example` (`PORT`, `JWT_SECRET`,
`JWT_EXPIRES_IN`). Los usuarios de prueba están en `src/users/users_service.rs`: `john`
(administrador) y `maria`.

```bash
curl -X POST http://127.0.0.1:3000/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"john","password":"changeme"}'
```

::: warning
Si no defines `JWT_SECRET`, `cats-app` firma con el valor por defecto `change-me`. Es un ejemplo: no
lo despliegues así.
:::

## `blog-orm`: API REST con base de datos

```bash
cargo run -p blog-orm        # http://127.0.0.1:3000 (DATABASE_URL=file:./dev.db por defecto)
```

Usuarios y posts sobre SQLite con [corvis-orm](/data/corvis-orm): paginación, búsqueda, relaciones,
escrituras anidadas y estadísticas. Los DTOs se validan con `corvis::z` (`zod_dto!` +
`ZodValidationPipe::auto()`) y `OrmExceptionFilter` traduce los errores de la base de datos a `409` o
`404`. `OrmModule::for_root_async` lee la URL de `ConfigService` y sincroniza las tablas al arrancar.

```bash
curl -X POST http://127.0.0.1:3000/users -H 'content-type: application/json' \
  -d '{"email":"ana@corvis.dev","name":"Ana"}'
curl 'http://127.0.0.1:3000/posts?search=hola&page=1&limit=10'
```

Variables: `DATABASE_URL`, `PORT` y `LOG_QUERIES=true` para imprimir cada consulta SQL.

## `tertulia`: chat en tiempo real

```bash
JWT_SECRET="$(openssl rand -hex 32)" cargo run -p tertulia
# abre http://127.0.0.1:5150
```

Un chat con salas, presencia e indicadores de escritura, con cliente web incluido. Es el ejemplo de
referencia para:

- WebSockets: un `#[websocket_gateway(path = "/chat")]` con salas, acks, `WsException`, guards,
  pipes zod e interceptors por mensaje; y un WebSocket plano con `corvis::ws` en `/ws/firehose`.
- Autenticación JWT al conectar (token en `?token=`) y un mismo `AuthGuard` para HTTP y WebSocket.
- Inyección entre capas: el controlador REST inyecta el gateway para leer quién está conectado y
  enviar anuncios.
- SSE en `/api/stats/stream`, persistencia con corvis-orm y archivos estáticos con
  `corvis::serve_static`.

`JWT_SECRET` es obligatoria: sin ella la aplicación no arranca. El resto de variables y el protocolo
del chat están en `examples/tertulia/README.md`. El login no pide contraseña, así que no lo expongas
tal cual.

## Pruebas de los ejemplos

`tertulia` tiene pruebas de integración en `examples/tertulia/tests/` (arranque y chat por
WebSocket). Para ejecutar todas las pruebas del workspace:

```bash
cargo test --workspace
```
