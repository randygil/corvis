# Corvis

**Corvis** (por Corviknight) es un framework backend modular para Rust: módulos, inyección
de dependencias, controladores con decoradores, pipes, guards, interceptors, filtros de
excepción, middleware, lifecycle hooks, testing, CLI y los módulos más usados (config, JWT,
eventos, schedule, cache, throttler, health, serve-static, WebSockets).

Si vienes del ecosistema de Node.js con TypeScript, el modelo te resultará conocido: los
decoradores son atributos (`@Get(':id')` → `#[get(":id")]`), las clases son `struct`s y los
constructores se generan solos a partir de los campos `Arc<T>`.

```rust
use corvis::prelude::*;

#[injectable]
pub struct CatsService;

impl CatsService {
    pub fn find_all(&self) -> Vec<&'static str> {
        vec!["Tom", "Garfield"]
    }
}

#[controller("cats")]
pub struct CatsController {
    cats_service: Arc<CatsService>,
}

#[routes]
impl CatsController {
    #[get]
    fn find_all(&self) -> Vec<&'static str> {
        self.cats_service.find_all()
    }

    #[get(":id")]
    fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Value> {
        if id == 0 {
            return Err(NotFoundException::new(format!("Cat #{id} not found")));
        }
        Ok(json!({ "id": id }))
    }
}

#[module(controllers = [CatsController], providers = [CatsService])]
pub struct AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let app = CorvisFactory::create::<AppModule>().await?;
    app.listen("127.0.0.1:3000").await
}
```

```text
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [CorvisFactory] Starting Corvis application...
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [InstanceLoader] AppModule dependencies initialized +1ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [RoutesResolver] CatsController {/cats}: +0ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [RouterExplorer] Mapped {/cats, GET} route +0ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [RouterExplorer] Mapped {/cats/:id, GET} route +0ms
[Corvis] 12345  - 07/11/2026, 10:00:00     LOG [CorvisApplication] Corvis application successfully started +0ms
```

---

## Contenido

- [Estructura del repositorio](#estructura-del-repositorio)
- [Inicio rápido con la CLI](#inicio-rápido-con-la-cli)
- [Equivalencias para quien viene de Node.js](#equivalencias-para-quien-viene-de-nodejs)
- [Guía](#guía)
- [Schemas y validación estilo zod (`corvis::z`)](#schemas-y-validación-estilo-zod-corvisz)
- [ORM (`corvis-orm`)](#orm-corvis-orm)
- [OpenAPI y Swagger (`corvis-swagger`)](#openapi-y-swagger-corvis-swagger)
- [Rendimiento](#rendimiento)
- [Limitaciones conocidas](#limitaciones-conocidas)

## Estructura del repositorio

| Ruta | Contenido |
|---|---|
| `crates/corvis` | Runtime: DI, HTTP (hyper), router, pipeline, módulos oficiales, testing, `z` |
| `crates/corvis-macros` | Atributos `#[module]`, `#[injectable]`, `#[controller]`, `#[routes]`, `#[handlers]`, `#[websocket_gateway]`, `#[derive(Validate)]` |
| `crates/corvis-cli` | CLI `corvis` (`new`, `generate`, `start`, `build`, `info`) |
| `crates/corvis-orm`, `crates/corvis-orm-macros` | ORM sobre sqlx (modelos, consultas JSON, migraciones) + `OrmModule` |
| `crates/corvis-swagger`, `crates/corvis-swagger-macros` | OpenAPI: documento, Swagger UI, DTOs zod y la referencia de Scalar |
| `examples/cats-app` | App completa: auth JWT, roles, CRUD, versionado, SSE, cache, eventos, cron, WebSockets |
| `examples/blog-orm` | API REST con `corvis-orm` (SQLite) |
| `examples/tertulia` | Chat en tiempo real con WebSockets: salas, presencia, guards, zod, acks, SSE, `corvis::ws` y cliente web |
| `bench/` | Benchmark reproducible contra un framework equivalente de Node.js |
| `editors/vscode` | Extensión de VS Code: snippets, comandos de la CLI, explorador de rutas y resaltado ([README](editors/vscode/README.md)) |

```bash
cargo test --workspace                       # todos los tests
cargo run -p cats-app                        # http://127.0.0.1:3000/api/v1
```

## Inicio rápido con la CLI

```bash
cargo install --path crates/corvis-cli      # instala el binario `corvis`

corvis new my-app                            # proyecto nuevo (AppModule, controller, service, test)
cd my-app
corvis generate resource cats                # CRUD completo: module, controller, service, DTOs, entity, test
corvis g controller users --flat             # schematics sueltos
corvis g service users --no-spec
corvis start --watch                         # cargo run, reinicia al cambiar archivos
corvis build                                 # cargo build --release
corvis info
```

Schematics disponibles: `module`, `controller`, `service`, `resource`, `guard`,
`interceptor`, `pipe`, `filter`, `middleware`, `gateway`. La CLI registra lo generado en el
módulo correspondiente (`imports`, `controllers`, `providers`) y añade las declaraciones `mod`.

`corvis new` acepta `--corvis-path <ruta>` (dependencia local), `--corvis-version <ver>`
y `--skip-git`.

## Equivalencias para quien viene de Node.js

Las tablas muestran, a la izquierda, la forma habitual de cada pieza en un framework de Node.js
con TypeScript y decoradores y, a la derecha, su equivalente en Corvis.

### Estructura

| Node.js (TypeScript) | Corvis |
|---|---|
| `@Module({ imports, controllers, providers, exports })` | `#[module(imports = [...], controllers = [...], providers = [...], exports = [...])]` |
| `@Global()` | `#[global]` encima de `#[module]`, o `#[module(global, ...)]` |
| `static forRoot(): DynamicModule` | `fn for_root(...) -> DynamicModule` (`DynamicModule::new::<Self>()...`) |
| Módulo con `configure(consumer)` para middleware | `impl CorvisModule for AppModule { fn configure(&self, consumer: &mut MiddlewareConsumer) }` |
| `@Injectable()` | `#[injectable]` |
| `constructor(private catsService: CatsService)` | campo `cats_service: Arc<CatsService>` |
| `@Inject('TOKEN')` | `#[inject("TOKEN")] field: Arc<T>` |
| `@Optional()` | `field: Option<Arc<T>>` |
| `{ provide, useValue }` | `Provider::use_value("TOKEN", value)` |
| `{ provide, useClass }` | `Provider::use_class::<Impl>("TOKEN")`; para trait objects `Provider::use_class_as::<PgRepo, dyn Repo>("REPO", \|r\| r)` |
| `{ provide, useFactory, inject }` | `Provider::use_factory("TOKEN", \|ctx\| async move { ... })` |
| `{ provide, useExisting }` | `Provider::use_existing("ALIAS", Token::of::<CatsService>())` |
| `Scope.TRANSIENT` | `#[injectable(scope = "transient")]` |
| `ModuleRef` (`get`, `resolve`, `create`) | campo `module_ref: ModuleRef` |
| `APP_GUARD` / `APP_PIPE` / `APP_INTERCEPTOR` / `APP_FILTER` | `Provider::app_guard::<G>()`, `app_pipe`, `app_interceptor`, `app_filter` |

### Controladores

| Node.js (TypeScript) | Corvis |
|---|---|
| `@Controller('cats')` | `#[controller("cats")]` (`#[controller(path = "cats", host = "...", version = "1")]`) |
| `@Get()`, `@Post(':id')`, `@Put`, `@Patch`, `@Delete`, `@All`, `@Options`, `@Head` | `#[get]`, `#[post(":id")]`, `#[put]`, `#[patch]`, `#[delete]`, `#[all]`, `#[options]`, `#[head]` |
| `@Sse('stream')` | `#[sse("stream")]` devolviendo `Sse` |
| `@HttpCode(204)` | `#[http_code(HttpStatus::NO_CONTENT)]` |
| `@Header('Cache-Control', 'none')` | `#[header("Cache-Control", "none")]` |
| `@Redirect('url', 301)` | `#[redirect("url", 301)]` |
| `@Version('2')` | `#[version("2")]` |
| `@Body()`, `@Body('name')` | `#[body] dto: T`, `#[body("name")] name: String` |
| `@Param('id', ParseIntPipe)` | `#[param("id", ParseIntPipe)] id: u32` |
| `@Query()` | `#[query] q: T` (los strings se convierten a números/booleanos según el tipo) |
| `@Headers('authorization')` | `#[headers("authorization")] auth: Option<String>` |
| `@Ip()`, `@Req()` | `#[ip] ip: String`, `#[req] req: Request` |
| `@UploadedFile()`, `@UploadedFiles()` | `#[uploaded_file] file: Option<UploadedFile>`, `#[uploaded_files] files: Vec<UploadedFile>` (ver [Subida de archivos](#subida-de-archivos-multer)) |
| `createParamDecorator(...)` | `impl FromContext for CurrentUser` (parámetro sin atributo) |
| `@SetMetadata('roles', ['admin'])` | `#[set_metadata("roles", ["admin"])]` |
| `@UseGuards`, `@UseInterceptors`, `@UsePipes`, `@UseFilters` | `#[use_guards(...)]`, `#[use_interceptors(...)]`, `#[use_pipes(...)]`, `#[use_filters(...)]` (tipo o expresión) |
| `@Res()` / `@Res({ passthrough: true })` | `#[res] res: Res` / `#[res(passthrough)] res: Res` (ver [`@Res()`](#res-y-res-passthrough-true-)) |
| Respuestas especiales | devuelve `Response`, `Json`, `Html`, `Redirect`, `StreamableFile` |
| `throw new NotFoundException('msg')` | `Err(NotFoundException::new("msg"))?` / `HttpResult<T>` |
| `new HttpException({...}, 418)` | `HttpException::new(json!({...}), 418)` |

Los atributos de clase (`#[use_guards]`, `#[set_metadata]`, ...) van **debajo** de
`#[controller]`.

### Pipeline de la request

| Node.js (TypeScript) | Corvis |
|---|---|
| Middleware con `use(req, res, next)` | `CorvisMiddleware::use_(&self, req, next) -> HandlerResult` |
| `consumer.apply(M).exclude(...).forRoutes(...)` | `consumer.apply::<M>().exclude(...).for_routes("cats")` |
| `CanActivate.canActivate(ctx)` | `CanActivate::can_activate(&self, ctx) -> Result<bool, HttpException>` |
| Interceptor con `intercept(ctx, next)` + RxJS `map` | `intercept(&self, ctx, next: CallHandler)`; `next.handle().await?` y transformas la respuesta |
| `PipeTransform.transform(value, metadata)` | `PipeTransform::transform(&self, value: Value, meta) -> Result<Value, HttpException>` |
| `ExceptionFilter.catch(exception, host)` + `@Catch(T)` | `ExceptionFilter::{catches, catch}` |
| `Reflector.getAllAndOverride(key, [handler, class])` | `reflector.get_all_and_override(KEY, &[ctx.get_handler(), ctx.get_class()])` |
| `ValidationPipe({ whitelist, forbidNonWhitelisted, transform })` | `ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true)` |
| class-validator (`@IsString()`, `@IsEmail()`, `@Min(0)`, `@ValidateNested()`...) | `#[derive(Validate)]` con `#[is_string]`, `#[is_email]`, `#[min(0)]`, `#[validate_nested]`... |

Orden de ejecución: middleware → guards → interceptors (antes) → pipes → handler →
interceptors (después) → filtros de excepción. Los errores producen cuerpos JSON con el
formato habitual, por ejemplo
`{"message":"Cat #42 not found","error":"Not Found","statusCode":404}` o
`{"statusCode":500,"message":"Internal server error"}`.

### Aplicación

| Node.js (TypeScript) | Corvis |
|---|---|
| Crear la aplicación desde el módulo raíz | `CorvisFactory::create::<AppModule>().await?` |
| Contexto de aplicación sin HTTP (standalone) | `CorvisFactory::create_application_context::<M>()` |
| `app.setGlobalPrefix('api')` | `app.set_global_prefix("api")` |
| `app.useGlobalPipes/Guards/Interceptors/Filters` | `app.use_global_pipes(...)`, etc. |
| `app.use(fn)` | `app.use_fn(...)` / `app.use_middleware(...)` |
| `app.enableCors()` | `app.enable_cors(CorsOptions::default())` (semántica del paquete `cors`, ver más abajo) |
| `app.disable('x-powered-by')`, `app.set('etag', 'strong')` | `app.disable("x-powered-by")`, `app.set_etag(EtagMode::Strong)`, `app.set_x_powered_by("Express")` |
| `HttpAdapterHost`, `app.getHttpServer()`, `server.on('upgrade')` | `Arc<HttpAdapterHost>` inyectable, `app.get_http_server()`, `server.on_upgrade(...)` + `corvis::ws` |
| `app.enableVersioning({ type: URI })` | `app.enable_versioning(VersioningOptions::uri().default_version("1"))` (también header y media type) |
| `app.enableShutdownHooks()` | `app.enable_shutdown_hooks()` |
| `app.get(CatsService)` | `app.get::<CatsService>()?` |
| `app.listen(3000)` | `app.listen("127.0.0.1:3000").await` (acepta puerto, `&str` o `SocketAddr`) |
| `OnModuleInit`, `OnApplicationBootstrap`, `OnModuleDestroy`, `BeforeApplicationShutdown`, `OnApplicationShutdown` | mismos traits; se detectan automáticamente al implementarlos |
| `new Logger(CatsService.name)` | `Logger::new("CatsService")` (formato de consola `[Corvis] pid - fecha NIVEL [contexto] mensaje`) |

### Módulos incluidos

| Node.js (TypeScript) | Corvis |
|---|---|
| Configuración (`ConfigModule.forRoot`, `ConfigService.get`, `registerAs`) | `ConfigModule::for_root(ConfigModuleOptions {...})`, `ConfigService::get/get_or/get_or_throw`, `ConfigModule::register_as` (lee `.env`) |
| JWT (`JwtModule`, `JwtService`) | `JwtModule::register(...)` / `register_async`, `JwtService::{sign, verify, decode}` |
| Eventos (`@OnEvent`) | `EventEmitterModule::for_root(...)`, `EventEmitter::emit`, `#[on_event("cat.created")]` en un `#[handlers] impl` (con comodines) |
| Tareas programadas (`@Cron`, `@Interval`, `@Timeout`, `SchedulerRegistry`) y `cron` (`CronJob`, `CronTime`) | `ScheduleModule::for_root()` / `for_root_with` / `for_root_async`, `#[cron(expr, name = ..., time_zone = ...)]`, `#[interval("name", ms)]`, `#[timeout(ms)]`, `CronJob::from(CronJobParams)`, `set_interval`/`set_timeout` (ver [Tareas programadas](#tareas-programadas-corvisschedule)) |
| Cache (`CacheInterceptor`, `@CacheTTL`) | `CacheModule::register(...)`, `CacheManager`, `#[use_interceptors(CacheInterceptor)]`, `#[set_metadata(CACHE_TTL_METADATA, 5_000)]` |
| Límite de requests (`ThrottlerGuard`, `@SkipThrottle`) | `ThrottlerModule::for_root(...)`, `ThrottlerGuard`, `#[set_metadata(SKIP_THROTTLE, true)]` |
| Health checks | `TerminusModule`, `HealthCheckService`, indicadores HTTP y de memoria |
| Archivos estáticos (`ServeStaticModule`) | `ServeStaticModule::for_root(ServeStaticOptions {...})` |
| WebSockets (`@WebSocketGateway`, `@SubscribeMessage`, `@WebSocketServer`, rooms) | `#[websocket_gateway(path = "/chat")]`, `#[subscribe_message("msg")]`, `#[websocket_server] server: WsServer`, `client.join(room)`, `server.to(room).emit(...)` |
| Testing (`Test.createTestingModule`, `overrideProvider`, `overrideGuard`) + supertest | `Test::create_testing_module()`, `.override_provider::<T>().use_value(...)`, `.override_guard::<G>(mock)`, `request(&app).get("/").send().await.expect_status(200)` |

## Guía

### Módulos y providers

```rust
#[module(
    imports = [ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() })],
    controllers = [CatsController],
    providers = [
        CatsService,
        Provider::use_value("API_KEY", "secret".to_string()),
        Provider::use_factory("DB_URL", |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            Ok(config.get_or("DATABASE_URL", "sqlite::memory:".to_string()))
        }),
    ],
    exports = [CatsService],
)]
pub struct CatsModule;

#[injectable]
pub struct CatsService {
    #[inject("API_KEY")]
    api_key: Arc<String>,
    config: Arc<ConfigService>,
    #[init(Logger::new("CatsService"))]
    logger: Logger,
    #[default]
    cats: std::sync::RwLock<Vec<Cat>>,
}
```

Encapsulación: un módulo solo ve sus providers, los `exports` de sus `imports` (incluidos
los re-exports) y los módulos globales. Si falta una dependencia, el error indica cuál y en
qué módulo:
`Corvis can't resolve dependencies of the CatsService (?). Please make sure that the argument API_KEY at index [0] is available in the CatsModule context.`

También detecta dependencias circulares y ejecuta los lifecycle hooks empezando por los
módulos más profundos.

### Validación con DTOs (class-validator)

```rust
#[derive(Debug, Deserialize, Validate)]
pub struct CreateCatDto {
    #[is_string]
    #[is_not_empty]
    #[max_length(50)]
    pub name: String,
    #[is_int]
    #[min(0)]
    pub age: u32,
    #[is_optional]
    #[is_email]
    pub owner_email: Option<String>,
}

app.use_global_pipes(ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true));
```

Respuesta de error:
`{"message":["name should not be empty","age must not be less than 0"],"error":"Bad Request","statusCode":400}`.

### Guards, interceptors y decoradores personalizados

```rust
pub const ROLES_KEY: &str = "roles";

#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let roles: Option<Vec<String>> = self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(roles) = roles else { return Ok(true) };
        let user = ctx.switch_to_http().get_request().user_as::<JwtPayload>();
        Ok(user.is_some_and(|u| roles.iter().any(|r| u.roles.contains(r))))
    }
}

// En el controlador:
#[delete(":id")]
#[set_metadata(ROLES_KEY, ["admin"])]
#[use_guards(RolesGuard)]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> { ... }

// @CurrentUser()
pub struct CurrentUser(pub JwtPayload);

#[async_trait]
impl FromContext for CurrentUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ctx.request().user_as::<JwtPayload>().map(CurrentUser).ok_or_else(|| UnauthorizedException.into())
    }
}
```

### Eventos y tareas programadas

```rust
#[injectable]
pub struct TasksService {
    events: Arc<EventEmitter>,
}

#[handlers]
impl TasksService {
    #[cron(CronExpression::EVERY_30_SECONDS)]
    async fn report(&self) {
        self.events.emit("report.ready", json!({ "at": "now" }));
    }

    #[on_event("cat.*")]
    async fn on_cat(&self, payload: Value) { /* ... */ }
}
```

### Tareas programadas (`corvis::schedule`)

`corvis::schedule` replica el módulo de tareas programadas habitual en Node.js (con decoradores
`@Cron`, `@Interval` y `@Timeout`) y el paquete `cron` 4 que usa por debajo, con la misma sintaxis
de expresiones y los mismos mensajes de error:

- Expresiones de 5 o 6 campos (`[seg] min hora día-mes mes día-semana`), día de la semana `0-7`
  (`0` y `7` = domingo), alias `jan..dec` / `sun..sat`, presets (`@daily`, `@weekdays`, ...),
  rangos, listas y pasos. Si se restringen día del mes y día de la semana, basta con que coincida
  uno de los dos (como `cron`). `CronExpression` trae las constantes habituales (`EVERY_30_SECONDS`, ...).
- Zona horaria: `time_zone = "America/Caracas"` (nombre IANA, sin distinguir mayúsculas, o
  `UTC±H[:MM]`) o `utc_offset = -240` (minutos). Sin ninguna se usa la zona del proceso: la de
  `TZ` si existe (como Node, también en Windows) o la del sistema. Los saltos y repeticiones por
  horario de verano se resuelven igual que `cron` + luxon.
- `#[cron(expr, ...)]` acepta `name`, `time_zone`, `utc_offset`, `disabled`, `wait_for_completion`,
  `threshold`, `initial_delay` y `unref_timeout`, o una expresión `CronOptions` como segundo
  argumento (`#[cron(EXPR, CronOptions::new().name("x"))]`). `#[interval("nombre", ms)]` y
  `#[timeout("nombre", ms)]` aceptan el nombre opcional primero, como los decoradores.
- Ciclo de vida: los handlers se recolectan al inicializar (un nombre repetido hace
  fallar el arranque con `Interval with the given name (x) already exists. Ignored.`), se montan
  antes de `onApplicationBootstrap` (timeouts, intervalos y luego crons) y se limpian antes de
  `beforeApplicationShutdown`. Los que no tienen nombre reciben un UUID v4. Un error o un `panic`
  de un handler se registra con el logger `Scheduler` y el job sigue corriendo.
- Las ejecuciones pueden solaparse: `#[interval]` es `setInterval` y un cron no espera a la
  ejecución anterior salvo con `wait_for_completion = true` (entonces se salta el tick).
- `SchedulerRegistry`: `get_cron_job`, `get_interval`, `get_timeout` (fallan con
  `No Cron Job was found with the given name (x)...`), `add_cron_job(name, CronJob)`,
  `add_interval(name, set_interval(ms, f))`, `add_timeout(name, set_timeout(ms, f))`,
  `get_cron_jobs()` (un `IndexMap` en orden de inserción), `get_intervals`, `get_timeouts`,
  `delete_cron_job`, `delete_interval`, `delete_timeout` y `does_exist("cron" | "interval" | "timeout", name)`.
- `CronJob` (`CronJob::new(expr, f)` o `CronJob::from(CronJobParams::new(expr, f).time_zone(..).start(true))`):
  `start`, `stop`, `stop_and_wait`, `is_active`, `is_callback_running`, `next_date`, `next_dates(n)`,
  `last_date`, `fire_on_tick`, `add_callback`, `set_time(CronTime)`, `run_on_init`, `on_complete`,
  `error_handler`, `threshold` (aviso `[Cron] Missed execution deadline ...`) y jobs de una sola vez
  a partir de una fecha (`CronJobParams::new(chrono::Utc::now() + delta, f)`).
- `CronTime::new(expr, time_zone, utc_offset)`: `send_at`, `send_at_n`, `get_timeout`,
  `get_next_date_from`, `to_string()` / `to_json()`; y las funciones `send_at`, `timeout` y
  `validate_cron_expression` del paquete.

```rust
#[injectable]
pub struct ReportsService {
    registry: Arc<SchedulerRegistry>,
}

#[handlers]
impl ReportsService {
    #[cron("0 30 9,17 * * *", name = "fx", time_zone = "America/Caracas")]
    async fn refresh(&self) -> corvis::Result<()> { /* ... */ Ok(()) }

    #[interval("sync", 120_000)]
    async fn sync(&self) {}
}

impl ReportsService {
    fn pause(&self) -> corvis::Result<()> {
        self.registry.get_cron_job("fx")?.stop();
        Ok(())
    }
}
```

Las pruebas están en `crates/corvis/tests/schedule.rs` (parseo y mensajes de error, zonas horarias
y horario de verano, temporización de `CronJob`, `wait_for_completion`, jobs de una vez, registro,
opciones de los atributos y ciclo de vida). Además, el cálculo de fechas se comparó contra el paquete
`cron` 4.4 en Node con 48 expresiones × 9 zonas (incluidas zonas con horario de verano de 1 hora y
de 30 minutos) × 7 instantes de partida, sin diferencias.

### Testing

```rust
#[corvis::test]
async fn find_all() {
    let module = Test::create_testing_module()
        .import::<CatsModule>()
        .override_provider::<CatsService>()
        .use_value(CatsService::mock())
        .override_guard::<AuthGuard>(AllowAll)
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();

    request(&app).get("/cats").send().await.expect_status(200).expect_json(json!([]));
}
```

El ejemplo completo con autenticación, roles, versionado, SSE, cache, throttling, eventos,
cron y WebSockets está en `examples/cats-app`.

### WebSockets: gateways

Un gateway es un provider con `#[websocket_gateway(path = "/chat")]` y sus handlers en un
bloque `#[handlers]`. Los frames son JSON `{ "event": "...", "data": ... }`, como el
adaptador `WsAdapter` sobre el paquete `ws` de Node.js.

```rust
#[websocket_gateway(path = "/chat")]
#[use_guards(AuthGuard)]                          // enhancers de clase, debajo del atributo
#[use_interceptors(WsLoggingInterceptor)]
#[use_pipes(ZodValidationPipe::auto())]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,                             // el servidor de este gateway (uno por app)
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("message")]
    #[use_guards(RoomMemberGuard)]                // enhancers del handler
    async fn message(&self, #[message_body] dto: SendMessageDto, #[connected_socket] client: WsClient) -> HttpResult<WsResponse> {
        self.server.to(&dto.room).except(client.id()).emit("message", &dto.text);
        Ok(WsResponse::new("sent", json!({ "text": dto.text })))
    }

    #[subscribe_message("kick")]
    #[set_metadata("roles", ["moderator"])]
    #[use_guards(RolesGuard)]
    fn kick(&self, #[message_body("id")] id: String) -> HttpResult<()> {
        let c = self.server.client(&id).ok_or_else(|| WsException::object(json!({ "code": "NOT_FOUND" })))?;
        c.disconnect_with(4000, "kicked");
        Ok(())
    }
}
```

| Node.js (TypeScript) | Corvis |
|---|---|
| `@WebSocketGateway({ path })` / `@WebSocketServer() server` | `#[websocket_gateway(path = "...")]` / `#[websocket_server] server: WsServer` |
| `@SubscribeMessage('e')`, `@MessageBody()`, `@MessageBody('id', Pipe)`, `@ConnectedSocket()` | `#[subscribe_message("e")]`, `#[message_body]`, `#[message_body("id", Pipe)]`, `#[connected_socket] client: WsClient` |
| `@UseGuards`, `@UseInterceptors`, `@UsePipes`, `@SetMetadata` en gateway o handler | `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]`, `#[set_metadata]` (debajo de `#[websocket_gateway]` o en el método) |
| `context.getType() === 'ws'`, `switchToWs().getClient()`, `.getData()` | `ctx.get_type() == ContextType::Ws`, `ctx.switch_to_ws().get_client::<WsClient>()`, `.get_data()` |
| `afterInit`, `handleConnection`, `handleDisconnect` | `OnGatewayInit`, `OnGatewayConnection`, `OnGatewayDisconnect` |
| `return { event, data }` / `return data` | `WsResponse::new(event, data)` / cualquier valor `Serialize` (se responde con el mismo evento) |
| `throw new WsException('msg')` / `new WsException({ ... })` | `Err(WsException::new("msg"))` / `Err(WsException::object(json!({ ... })))`; `WsException::get_error(&e)` |
| `server.emit`, `server.to(room).emit`, `client.join/leave` | `server.emit(..)`, `server.to(room).except(id).emit(..)`, `client.join(room)`, `client.rooms()`, `server.clients()` |
| `client.close()` / `client.close(code, reason)` | `client.disconnect()` / `client.disconnect_with(4401, "Unauthorized")` |

Semántica:

- **Pipeline**: guards → interceptors → pipes → handler, igual que en HTTP. Un guard que
  devuelve `false` responde `WsException("Forbidden resource")`. Los guards reciben un
  `ExecutionContext` de tipo `Ws`: `ctx.request()` es la request del handshake (headers,
  query, IP) y lo que guardes con `client.insert(valor)` lo lee cualquier guard o handler
  posterior con `client.get::<T>()`. Así un mismo guard puede servir para HTTP y para WS.
- Los pipes de clase, de método y de parámetro se aplican a cada `#[message_body]`
  (`#[message_body("prop")]` toma solo esa propiedad del payload). Con
  `ZodValidationPipe::auto()` se validan los DTOs declarados con `zod_dto!`.
- Los interceptors ven la respuesta del handler como el cuerpo JSON de un `Response`: el dato,
  `{ event, data }` si fue un `WsResponse` o `null` si no hay respuesta; `map_data` funciona igual.
- Los enhancers **globales** (`use_global_*`, `APP_GUARD`, ...) no se aplican a los gateways:
  decláralos en el gateway. Los filtros de excepción (`#[use_filters]`) no están soportados en
  gateways.
- **Errores**: el cliente recibe `{ "event": "exception", "data": ... }` con
  `{ status: "error", message }` para `WsException::new`, el objeto tal cual para
  `WsException::object`, `{ status: "error", message: "Internal server error" }` para errores
  desconocidos (que además se registran) y, para cualquier otro `HttpException`,
  `{ status: "error", message, ...resto de su cuerpo }` (por ejemplo `statusCode` y los
  `errors` de zod).
- **Acks**: si el frame trae `id` (`{ event, data, id }`), la respuesta y el frame `exception`
  devuelven el mismo `id`. Es una extensión de Corvis (el adaptador `WsAdapter` de Node.js no
  correlaciona respuestas). Los clientes que no envían `id` no notan diferencia.
- **Orden**: los handlers de una conexión empiezan en el orden en que llegaron sus frames y
  luego corren en concurrencia (como un `mergeMap` de RxJS): un `join` síncrono seguido de un
  `message` siempre ve la sala unida, pero si el handler hace `await` antes de `client.join`,
  el siguiente puede empezar antes de que termine. Un pánico en un handler se responde como
  error interno y no cierra la conexión.
- **Autenticación al conectar**: el handshake de un gateway siempre se acepta. Verifica el
  token en `handle_connection` (el navegador no puede mandar headers, usa `?token=`) y, si no
  es válido, `client.disconnect_with(4401, "Unauthorized")`. Los frames que lleguen después de
  `disconnect()` ya no se despachan. Para rechazar en el handshake con `401` usa un listener
  `upgrade` con `corvis::ws` (ver abajo).
- Cada aplicación tiene su propio `WsServer` por gateway, así que dos apps en el mismo proceso
  (por ejemplo en tests) no comparten clientes. `WsServer::for_path(path)` devuelve el del
  último gateway inicializado en esa ruta; lo recomendable es inyectar el gateway.
- Eventos desconocidos y frames que no son JSON se ignoran. En
  `handle_disconnect` el cliente ya no está en `server.clients()`, pero `client.rooms()`
  conserva sus salas para avisarles.

`examples/tertulia` usa todo lo anterior en un chat con salas, presencia, indicadores de
escritura, historial en SQLite, endpoints REST que leen y empujan estado por WebSocket, SSE y
un WebSocket plano para moderadores.

### `HttpAdapterHost` y WebSockets planos (paquete `ws`)

`HttpAdapterHost` se inyecta en cualquier provider (`adapter_host: Arc<HttpAdapterHost>`)
y da acceso al servidor HTTP de la aplicación. Con
`get_http_server().on_upgrade(...)` registras listeners del evento `upgrade` de Node, y con
`corvis::ws::WebSocketServer` (equivalente a `new WebSocketServer({ noServer: true })` del
paquete `ws`) aceptas la conexión como un WebSocket RFC 6455 plano: frames de texto o
binarios tal cual, sin el protocolo `{ event, data }` de los gateways.

```rust
use corvis::ws::{ReadyState, WebSocket, WebSocketServer, WebSocketServerOptions};

#[derive(Default)]
pub struct BridgeState {
    clients: Mutex<HashSet<WebSocket>>,
}

#[injectable]
pub struct FiscalPrinterBridgeService {
    adapter_host: Arc<HttpAdapterHost>,
    #[default]
    state: Arc<BridgeState>, // estado compartido con los listeners ('static)
}

#[async_trait]
impl OnApplicationBootstrap for FiscalPrinterBridgeService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        let server = self.adapter_host.http_adapter().get_http_server();
        let wss = WebSocketServer::new(WebSocketServerOptions::default());
        let state = self.state.clone();
        server.on_upgrade(move |req| {
            if req.pathname() != "/ws/fiscal-printer" {
                req.destroy(); // socket.destroy()
                return;
            }
            let state = state.clone();
            wss.handle_upgrade(&req, move |ws| {
                state.clients.lock().unwrap().insert(ws.clone());
                let (s, me) = (state.clone(), ws.clone());
                ws.on_close(move |_code, _reason| {
                    s.clients.lock().unwrap().remove(&me);
                });
                ws.on_error(|e| Logger::new("Bridge").warn(format!("socket error: {e}")));
                ws.on_message(|data| println!("ACK: {data}")); // String(data)
            });
        });
        Ok(())
    }
}

// Enviar un frame: el primer cliente abierto.
let client = state.clients.lock().unwrap().iter().find(|ws| ws.ready_state() == ReadyState::Open).cloned();
client.send(json!({ "kind": "EMIT", "requestId": id, "payload": payload }))?;     // encola
client.send_and_wait(r#"{"kind":"EMIT"}"#).await?; // como ws.send(data, cb): espera la escritura
```

| Node / `ws` | Corvis |
|---|---|
| `adapterHost.httpAdapter.getHttpServer()` / `app.getHttpServer()` | `adapter_host.http_adapter().get_http_server()` / `app.get_http_server()` |
| `server.on('upgrade', (req, socket, head) => ...)` | `server.on_upgrade(\|req: UpgradeRequest\| ...)` (síncrono, en orden de registro) |
| `req.url`, `new URL(req.url).pathname`, `req.headers`, `req.method` | `req.url()`, `req.pathname()`, `req.query_param("x")`, `req.headers()`, `req.method()`, `req.request()` |
| `socket.destroy()` | `req.destroy()` |
| `socket.end('HTTP/1.1 401 ...')` | `req.respond(Response::text("...").with_status(401))` (agrega `Connection: close`) |
| `wss.handleUpgrade(req, socket, head, cb)` | `wss.handle_upgrade(&req, \|ws\| ...)` (valida el handshake con los mismos 400/405 de `ws`) |
| `ws.send(data[, cb])` | `ws.send(data)` / `ws.send_and_wait(data).await` (`String`, `&str`, `Value`, `Vec<u8>`, `Bytes`) |
| `ws.on('message' \| 'close' \| 'error' \| 'ping' \| 'pong')` | `ws.on_message(\|m: WsMessage\|)`, `on_close(\|code, reason\|)`, `on_error`, `on_ping`, `on_pong`; o `ws.messages()` como `Stream` |
| `ws.readyState`, `ws.protocol` | `ws.ready_state()` (`ReadyState::{Connecting, Open, Closing, Closed}`), `ws.protocol()` |
| `ws.close(code, reason)`, `ws.terminate()`, `ws.ping()` | `ws.close(Some(1000), "bye")?`, `ws.terminate()`, `ws.ping(data)`; `ws.closed().await` devuelve `(code, reason)` |
| `wss.clients`, `wss.close()`, `{ maxPayload, clientTracking, handleProtocols }` | `wss.clients()`, `wss.close()`, `WebSocketServerOptions::default().max_payload(n)...` |

Semántica:

- Mientras haya al menos un listener `upgrade`, **toda** request HTTP/1.1 con
  `Connection: upgrade` + `Upgrade: ...` va a los listeners y no al router (como Node).
  Las rutas de los `#[websocket_gateway]` se atienden primero con su gateway, así que
  gateways y listeners conviven.
- El primer listener que acepta, responde o destruye la request gana; si todos la sueltan
  sin decidir, el socket se destruye.
- Sin listeners, si la app tiene gateways, un upgrade a una ruta sin gateway destruye el
  socket (igual que el adaptador `WsAdapter` de Node.js); si no hay gateways se atiende
  como una request normal.
- Registra los listeners del socket dentro del callback de `handle_upgrade`: los frames se
  empiezan a leer cuando el callback termina, así que no se pierde ninguno.
- `on_close` recibe `1005` si el cliente cerró sin código y `1006` si la conexión se cortó o
  se usó `terminate()`. Si el cliente no responde a `close()` en 30 s, se destruye el socket.
- Para protocolos que no son WebSocket, `req.switch_protocols(headers)` responde `101` y
  devuelve el stream crudo (`AsyncRead + AsyncWrite`).

### CORS, ETag y cabeceras por defecto de Express

`app.enable_cors(...)` sigue la semántica exacta del paquete `cors` de Express:

```rust
app.enable_cors(
    CorsOptions::new()
        .origins(["http://localhost:3000"])          // origin: [...]
        .methods(["GET", "HEAD", "POST", "PATCH", "PUT", "DELETE", "OPTIONS"])
        .allowed_headers(["Content-Type", "Authorization"])
        .exposed_headers(["Content-Disposition"])
        .credentials(true)
        .max_age(86_400),
);
```

| `cors` | Corvis |
|---|---|
| `origin: '*'` (por defecto) | `CorsOptions::default()` |
| `origin: 'http://a.com'` (fijo) | `.origin("http://a.com")` (llamarlo dos veces arma una lista) |
| `origin: ['http://a.com', /\.b\.com$/]` | `.origins([...])`, `.origin_regex(Regex::new(...)?)` |
| `origin: true` / `origin: false` | `.reflect_origin()` / `.origin_value(CorsOrigin::Disabled)` |
| `origin: (origin, cb) => ...` | `.origin_fn(\|origin\| CorsOrigin::Reflect)` (`Disabled` omite CORS en esa request) |
| `preflightContinue`, `optionsSuccessStatus` | `.preflight_continue(true)`, `.options_success_status(200)` |

- Todo `OPTIONS` es un preflight (aunque no traiga `Access-Control-Request-Method` y aunque
  la ruta no exista): `204`, `Content-Length: 0` y las cabeceras `Access-Control-*`,
  incluida `Access-Control-Expose-Headers`.
- `Vary: Origin` se envía siempre que el origen no sea `*`, también cuando el origen de la
  request no está permitido o falta (en ese caso no se envía `Access-Control-Allow-Origin`).
  Se combina con un `Vary` existente como el paquete `vary`.
- Sin `allowed_headers` se reflejan los headers pedidos (`Vary: Access-Control-Request-Headers`).
- Las cabeceras que fija el handler tienen prioridad sobre las de CORS.

Cada respuesta pasa además por los valores por defecto de Express:

- Los cuerpos enviados con `res.send()`/`res.json()` (JSON, texto, HTML) llevan `ETag` débil
  (`W/"<largo hex>-<sha1 base64>"`, idéntico al paquete `etag`) y `Content-Length`. Un
  `GET`/`HEAD` con `If-None-Match`/`If-Modified-Since` vigente responde `304` sin cuerpo.
  `app.set_etag(EtagMode::Strong)` o `app.disable("etag")` cambian esto.
- Un `Content-Type` explícito en un cuerpo de texto/JSON recibe `charset=utf-8` (como
  `res.send(string)`), y `#[header("Content-Type", "text/plain")]` agrega el charset por
  defecto (como `res.set()`): `text/plain; charset=utf-8`.
- Los cuerpos crudos (`Response::bytes`, `StreamableFile`, streams, cuerpo vacío) se
  escriben sin tocar, como `res.end(buffer)`: sin ETag y con el `Content-Type` exacto
  (`text/plain; charset=windows-1252` se mantiene).
- `HEAD` conserva `Content-Length` y `ETag` del `GET`, sin cuerpo.
- `X-Powered-By: corvis` por defecto. `app.set_x_powered_by("Express")` reproduce byte a byte
  lo que envía una app de Express; `app.disable("x-powered-by")` lo quita.

`StreamableFile` acepta las opciones `type`, `disposition` y `length`:

```rust
#[get(":id/pdf")]
async fn pdf(&self, #[param("id", ParseUUIDPipe)] id: String) -> StreamableFile {
    StreamableFile::with_options(bytes, StreamableFileOptions::new()
        .r#type("application/pdf")
        .disposition(format!("inline; filename=\"{id}.pdf\"")))
    // también: StreamableFile::new(buf).content_type(..).filename(..),
    // StreamableFile::from_stream(stream, StreamableFileOptions::new().length(n))
}
```

Como en el adaptador de Express, sus cabeceras solo se escriben si la respuesta no las
tiene ya (`#[header(...)]` gana), el `Content-Length` se completa solo para datos en
memoria y un stream sin `length` se envía chunked. Para escribir la respuesta como
`res.setHeader(...); res.end(buffer)` devuelve
`Response::bytes(buf, "application/pdf").with_header("Content-Disposition", "...")`.

`ParseUUIDPipe::new().version(4)` equivale a `new ParseUUIDPipe({ version: '4' })`
(mensaje `Validation failed (uuid v 4 is expected)`); un valor que no es string produce
`The value passed as UUID is not a string`.

### Body parser y router de Express

Los cuerpos se leen como con los parsers que se registran por defecto sobre Express
(`express.json()` y `express.urlencoded({ extended: true })` de `body-parser` 2), y las rutas se resuelven como el
router de Express 5:

- Solo se parsean `application/json` y `application/x-www-form-urlencoded` (con o sin
  parámetros como `charset`). Cualquier otro tipo, o una request sin cuerpo, deja el body en
  `undefined` (`Value::Null`); `text/plain` no se parsea.
- JSON: modo estricto (solo `{` o `[` arriba), cuerpo vacío = `{}`, BOM ignorado y los mismos
  mensajes de error de `JSON.parse` de V8 (`Expected property name or '}' in JSON at position 1
  (line 1 column 2)`, `Unexpected token '"', ""x"" is not valid JSON`, ...). Responde
  `400 { "message": ..., "error": "Bad Request", "statusCode": 400 }`.
- urlencoded: `qs.parse` con las opciones de body-parser (objetos y arrays anidados, profundidad
  32 → `400 The input exceeded the depth`, 1000 parámetros → `413 too many parameters`).
- Límite de 100kb por parser → `413 { "statusCode": 413, "message": "request entity too large" }`.
  `app.use_body_parser("json", 10 * 1024 * 1024)` equivale a `app.useBodyParser('json', { limit })`;
  `app.disable_body_parser()` desactiva el parseo de cuerpos (`bodyParser: false`).
- Charset no soportado → `415 unsupported charset "LATIN1"`.
- Los errores del parser pasan por CORS y por los filtros globales, como un `next(err)` de Express.
- `req.query` usa el parser `simple` de Express 5 (`node:querystring`: `a[]=1` conserva la clave
  `a[]`). `app.set_query_parser(QueryParser::Extended)` usa `qs` como Express 4.
- Rutas sin distinguir mayúsculas (`/CATS` = `/cats`) y con una sola barra final opcional
  (`/cats/` sí, `/cats//` no). Los parámetros se decodifican con `decodeURIComponent`; uno mal
  codificado responde `400 Failed to decode param '%E0%A4%A'`.
- Los parámetros de un handler se resuelven como un `Promise.all`: si fallan varios, gana
  el que falla en el pipe más temprano de su cadena (los globales primero) y, si empatan, el
  último declarado.
### `@Res()` y `@Res({ passthrough: true })`

`#[res] res: Res` es el objeto `response` de Express 5 tal como lo recibe un handler:

```rust
#[get("magic")]
async fn magic(&self, #[query("token")] token: Option<String>, #[res] res: Res) {
    if let Some(s) = self.auth.redeem(token).await {
        res.set_header("Set-Cookie", format!("sid={s}; Path=/; HttpOnly"));
    }
    res.redirect_with(302, "/");          // res.redirect(302, '/')
}

#[post("login")]
#[http_code(204)]
async fn login(&self, #[body] b: Value, #[res(passthrough)] res: Res) -> HttpResult<()> {
    res.cookie("sid", &self.auth.create().await, CookieOptions::new().http_only(true).same_site("lax"));
    Ok(())
}
```

- Con `#[res]` el handler es dueño de la respuesta: su valor de retorno se ignora y la
  request queda abierta hasta que llama a `send`, `json`, `end`, `redirect` o `send_status`
  (puede hacerlo después de retornar, desde otra tarea). Con `#[res(passthrough)]`
  (o `#[res(passthrough = true)]`) Corvis serializa el valor de retorno como siempre y conserva
  el status, las cabeceras y las cookies que el handler fijó en `res`.
- `res` ya trae el status de la ruta (`#[http_code]`, `201` en POST, `200` en
  el resto) y los `#[header(...)]` antes de los interceptors, y el handler puede cambiarlos.
- Si el handler lanza un error antes de responder, el filtro de excepciones arma la respuesta
  y se mantienen las cabeceras que se fijaron en `res` (las del filtro tienen prioridad).
- Métodos (nombres de Express en snake_case): `status`, `status_code`, `send_status`,
  `set`/`header` (agrega el charset por defecto a `Content-Type`), `set_header`, `append`,
  `get`/`get_header`, `get_header_values`, `get_headers`, `has_header`, `remove_header`,
  `type_`/`content_type`, `vary`, `links`, `attachment`, `location`, `cookie`, `cookie_json`,
  `clear_cookie`, `redirect`, `redirect_with(status, url)`, `send`, `json`, `end`, `end_empty`,
  `headers_sent`, `writable_ended`.
- `send(string)` usa `text/html` si no hay tipo, `send(Vec<u8>)` usa
  `application/octet-stream`, y ambos (igual que `json`) llevan `ETag`, `Content-Length`,
  charset y `304`. `end(bytes)` escribe los bytes sin tocar (sin ETag ni charset).
- `redirect` codifica la URL como el paquete `encodeurl`, agrega `Vary: Accept` y un cuerpo
  negociado con `Accept` (`Found. Redirecting to /` en `text/plain`, `<p>...</p>` en
  `text/html` o vacío).
- `cookie` serializa como el paquete `cookie` 0.7 con los valores por defecto de Express
  (`Path=/`, `max_age` en milisegundos que también fija `Expires`, valor con
  `encodeURIComponent`). `clear_cookie` envía `Expires` en 1970.
- Los interceptors y filtros pueden leerlo con `ctx.switch_to_http().get_response()`.

Las pruebas (`crates/corvis/tests/res.rs`) comparan 17 respuestas con las que captura de una
app real de Node.js con Express 5 el script `tests/fixtures/res_express.oracle.mjs`.

### Subida de archivos (multer)

`corvis::multer` replica `multer` 2 + `busboy` 1.6 tal como se usan sobre Express: los mismos
interceptores, opciones, campos del archivo, límites y errores. La paridad se verificó contra
una app real de Node.js con las mismas rutas (`crates/corvis/tests/multer.rs`).

```rust
use corvis::multer::{DiskStorageOptions, Limits, MulterOptions, disk_storage};
use corvis::{FileInterceptor, FileTypeValidator, MaxFileSizeValidator, ParseFilePipe, UploadedFile};

#[post(":id/evidence")]
#[use_interceptors(FileInterceptor::new("file").options(
    MulterOptions::new()
        .storage(disk_storage(
            DiskStorageOptions::new()
                .destination_fn(|_req, _file| {
                    std::fs::create_dir_all("./uploads").ok();
                    Ok("./uploads".into())
                })
                .filename(|_req, file| Ok(format!("{}-{}", chrono::Utc::now().timestamp_millis(), file.originalname))),
        ))
        .limits(Limits::new().file_size(5 * 1024 * 1024)),
))]
async fn upload(
    &self,
    #[param("id")] id: String,
    #[uploaded_file(ParseFilePipe::new()
        .validator(MaxFileSizeValidator::new(5_000_000))
        .validator(FileTypeValidator::new("image/(png|jpeg)")))]
    file: UploadedFile,
    #[body] body: Value,                 // los campos de texto (`req.body`)
) -> HttpResult<Value> { ... }
```

| Node.js / multer | Corvis |
|---|---|
| `FileInterceptor('file', opts)` | `FileInterceptor::new("file").options(opts)` |
| `FilesInterceptor('photos', 12, opts)` | `FilesInterceptor::new("photos").max_count(12)` |
| `FileFieldsInterceptor([{ name: 'avatar', maxCount: 1 }, { name: 'docs' }])` | `FileFieldsInterceptor::new([MulterField::new("avatar").max_count(1), MulterField::new("docs")])` |
| `AnyFilesInterceptor()`, `NoFilesInterceptor()` | `AnyFilesInterceptor::new()`, `NoFilesInterceptor::new()` |
| `@UploadedFile() file?: Express.Multer.File` | `#[uploaded_file] file: Option<UploadedFile>` |
| `@UploadedFiles() files` | `#[uploaded_files] files: Vec<UploadedFile>` o `HashMap<String, Vec<UploadedFile>>` (`FileFieldsInterceptor`) |
| `{ dest: 'uploads' }`, `memoryStorage()`, `diskStorage({ destination, filename })` | `MulterOptions::new().dest("uploads")`, `memory_storage()`, `disk_storage(DiskStorageOptions::new().destination(..).filename(..))` |
| `limits: { fileSize, files, fields, fieldSize, fieldNameSize, parts, fieldNestingDepth }` | `Limits::new().file_size(..).files(..).fields(..).field_size(..).field_name_size(..).parts(..).field_nesting_depth(..)` |
| `fileFilter: (req, file, cb) => cb(null, true)` | `.file_filter(\|req, file\| Ok(true))` (`Ok(false)` omite el archivo; `Err(e)` corta la subida) |
| `preservePath`, `defParamCharset` | `.preserve_path(true)`, `.def_param_charset("utf8")` |
| `MulterModule.register({...})` / `registerAsync` | `MulterModule::register(MulterOptions::new()...)` / `register_async` (opciones por defecto del módulo que lo importa) |
| `ParseFilePipe({ validators, fileIsRequired, errorHttpStatusCode, exceptionFactory })` | `ParseFilePipe::new().validator(..).file_is_required(false).error_http_status_code(422).exception_factory(..)` |
| `new ParseFilePipeBuilder().addMaxSizeValidator({...}).addFileTypeValidator({...}).build({...})` | `ParseFilePipeBuilder::new().add_max_size_validator(..).add_file_type_validator(..).build(ParseFilePipe::new()...)` |
| `MaxFileSizeValidator({ maxSize, message, errorMessage })` | `MaxFileSizeValidator::new(n).message(..).error_message(..)` (también `message_fn`, `error_message_fn`) |
| `FileTypeValidator({ fileType, skipMagicNumbersValidation, fallbackToMimetype })` | `FileTypeValidator::new("image/png")` o `::new(Regex)`, `.skip_magic_numbers_validation(true)`, `.fallback_to_mimetype(true)` |
| `class X implements FileValidator` | `impl FileValidator for X` (`is_valid` + `build_error_message`) |
| `StorageEngine` propio (`_handleFile`, `_removeFile`) | `impl StorageEngine for X` (`handle_file`, `remove_file`) |

Semántica (igual que Express):

- El interceptor solo actúa si la request trae cuerpo y `Content-Type: multipart/*`. En otro
  caso no hace nada y `#[body]` recibe el cuerpo de siempre.
- `UploadedFile` tiene los campos de `Express.Multer.File`: `fieldname`, `originalname`,
  `encoding`, `mimetype`, `size` y `buffer` (memoria) o `destination`, `filename` y `path`
  (disco). Si el handler lo devuelve, se serializa con el mismo orden de claves y `buffer`
  como `{ "type": "Buffer", "data": [...] }`.
- `#[body]` recibe los campos de texto con la semántica de `append-field`: `tags[]`,
  `user[name]`, `items[0]` y claves repetidas que se vuelven arrays.
- Errores con los mismos cuerpos que esa app de referencia: `413 File too large`, `400 Unexpected field - <campo>`,
  `400 Too many files`, `400 Field value too long - <campo>`, `400 Field name missing`,
  `400 Multipart: Boundary not found`, `400 Multipart: Unexpected end of form`,
  `400 Multipart: Malformed part header`. Los que esa app no traduce (`Unsupported content type`,
  `Field name nesting too deep`, fallos de escritura) son un 500 cuya causa es `MulterError`
  o `BusboyError`. Si la subida falla, los archivos que ya se escribieron en disco se borran.
- Igual que busboy, un archivo que **alcanza** `file_size` (no solo el que lo supera) da
  `File too large`, y `filename` se decodifica como latin1 salvo que llegue como `filename*=UTF-8''...`
  o se use `def_param_charset("utf8")`.
- Los cuerpos multipart no tienen el límite de tamaño del body-parser JSON, como en Express:
  solo aplican los `limits` de multer.
- `FileTypeValidator` detecta el tipo real por *magic numbers* (como el paquete `file-type`):
  imágenes, PDF, ZIP y Office, audio, video, fuentes y comprimidos, entre otros. Para tipos de
  texto sin firma usa `fallback_to_mimetype(true)` o `skip_magic_numbers_validation(true)`.
- Dentro de los pipes el archivo viaja como JSON; `corvis::multer::buffer_from_value(&v["buffer"])`
  recupera los bytes en un pipe o validador propio. Desde un guard o interceptor posterior,
  `req.file()` y `req.files_value()` (trait `MulterRequestExt`) dan `req.file` / `req.files`.
- En las pruebas, `request(&app).post("/x").field("a", "1").attach("file", bytes, "a.png")`
  arma el `multipart/form-data` como supertest.

### Archivos estáticos (`express.static`)

`corvis::serve_static(raíz)` replica `express.static` (`serve-static` 2 + `send` 1), el middleware
que las apps de Express montan con `app.use('/uploads', express.static(join(process.cwd(), 'uploads')))`:

```rust
app.use_middleware(corvis::serve_static(std::env::current_dir()?.join("uploads")).mount("/uploads"));
```

- `.mount(ruta)` equivale a `app.use(ruta, ...)`: atiende esa ruta y todo lo que cuelga de ella
  (sin distinguir mayúsculas y por segmentos completos) y le quita el prefijo.
- Solo `GET` y `HEAD`. Con `fallthrough` (por defecto) los demás métodos y los errores 4xx previos a
  encontrar un archivo (codificación inválida, `..`, *dotfiles*, archivo inexistente) pasan al
  siguiente handler, así que la app responde su 404 de siempre.
- Un directorio sin barra final redirige con `301`, `Location: /dir/` y el mismo HTML de Express;
  con barra final sirve `index.html` o sigue de largo.
- Los archivos llevan `Accept-Ranges: bytes`, `Cache-Control: public, max-age=N`, `Last-Modified`,
  un `ETag` débil (`W/"<tamaño hex>-<mtime hex>"`) y el `Content-Type` de `mime-types`. También
  responde GET condicionales (`304`, `412`) y rangos simples (`206`, `416`).
- Opciones: `fallthrough`, `redirect`, `max_age` (ms), `immutable`, `index`, `dotfiles`
  (`Dotfiles::{Allow, Deny, Ignore}`), `etag`, `last_modified`, `accept_ranges`, `cache_control` y
  `extensions`.

Dos piezas relacionadas con el ciclo de la respuesta:

- `ctx.switch_to_http().status_code()` es el `res.statusCode` que ven guards e interceptors:
  el estado de la ruta (`#[http_code]`, 201 en POST, 200 en el resto), fijado antes de correr la
  ruta. Sirve, por ejemplo, para un interceptor que guarda `{ status, body }`.
- `HttpException::http_error(status, mensaje)` representa un error de `http-errors` que no es un
  `HttpException` (como el 413 o el 415 de `body-parser`): `BaseExceptionFilter` responde
  `{ statusCode, message }` con ese estado, pero `is_internal()` es `true`, así que un filtro
  `@Catch()` propio lo trata como un error desconocido.
- Los enhancers globales registrados como providers (`Provider::app_interceptor`, `app_guard`,
  `app_pipe`) se aplican antes que los de `use_global_*`: envuelven a los que se
  registran en el `main`.

## Schemas y validación estilo zod (`corvis::z`)

Además de los DTOs con `#[derive(Validate)]`, Corvis incluye `corvis::z`, un port de
[zod](https://zod.dev) **4.4.3** y de su integración habitual con DTOs, pipes y serialización
en frameworks de Node.js (versión **5.5.0**). Produce los mismos códigos de error, mensajes
(locale `en`), orden de las claves, orden de los checks y forma JSON que zod 4, y el
`ZodValidationPipe` responde los mismos cuerpos de error que esa integración. El comportamiento de zod 3 sigue disponible con
`z::set_version(z::ZodVersion::V3)` (global), `z::with_version(..)` (hilo actual) o
`ParseOptions::v3()` (una llamada).

```rust
use corvis::prelude::*;   // incluye `z` y el trait `ZodSchema`

let create_user = z::object! {
    name: z::string().trim().min(2),
    email: z::email().message("Correo inválido"),
    age: z::int().nonnegative().optional(),
    role: z::enum_(["admin", "user"]).default("user"),
    tags: z::array(z::string()).max(5).default(json!([])),
    website: z::url().nullish(),
};

let data = create_user.parse(&json!({ "name": "Ana", "email": "ana@corvis.dev" }))?;
// {"name":"Ana","email":"ana@corvis.dev","role":"user","tags":[]}

let dto: CreateUserDto = create_user.parse_as(&body)?;     // equivalente a z.infer + serde

let result = create_user.safe_parse(&json!({ "name": "A" }));
let error = result.error.unwrap();
error.to_json();
// [{"origin":"string","code":"too_small","minimum":2,"inclusive":true,"path":["name"],
//   "message":"Too small: expected string to have >=2 characters"},
//  {"expected":"string","code":"invalid_type","path":["email"],
//   "message":"Invalid input: expected string, received undefined"}]
z::flatten_error(&error);   // {"formErrors":[],"fieldErrors":{"name":[...],"email":[...]}}
z::treeify_error(&error);   // {"errors":[],"properties":{"name":{"errors":[...]},...}}
z::prettify_error(&error);  // "✖ Too small: ...\n  → at name\n✖ Invalid input: ...\n  → at email"
```

### Equivalencias

| zod 4 | `corvis::z` |
|---|---|
| `z.string().min(3, "msg")`, `.min(3, { error: (iss) => ... })` | `z::string().min(3).message("msg")`, `.min(3).error(z::error_fn(\|iss\| Some(..)))` |
| `z.string({ error: "msg" })` | `z::string().error("msg")` (sin checks, aplica al schema) |
| `.max() .length() .nonempty() .regex(/re/i) .startsWith() .endsWith() .includes(s, { position })` | `.max() .length() .nonempty() .regex(("re", "i")) .starts_with() .ends_with() .includes_at(s, pos)` |
| `.trim() .toLowerCase() .toUpperCase() .normalize() .slugify() .lowercase() .uppercase()` | `.trim() .to_lower_case() .to_upper_case() .normalize("NFC") .slugify() .lowercase() .uppercase()` |
| `z.email()`, `z.email({ pattern: z.regexes.html5Email })` | `z::email()`, `z::email_with(z::regexes::HTML5_EMAIL.clone())` |
| `z.uuid() z.uuidv4() z.guid() z.url({ hostname, protocol, normalize }) z.httpUrl()` | `z::uuid() z::uuidv4() z::guid() z::url_with(UrlOptions {..}) z::http_url()` |
| `z.cuid() z.cuid2() z.ulid() z.xid() z.ksuid() z.nanoid() z.emoji()` | iguales |
| `z.ipv4() z.ipv6() z.cidrv4() z.cidrv6() z.mac({ delimiter }) z.hostname()` | `z::ipv4() ... z::mac(Some("-")) z::hostname()` |
| `z.base64() z.base64url() z.hex() z.hash("sha256") z.jwt({ alg }) z.e164()` | `z::base64() ... z::hash("sha256", None) z::jwt(Some("HS256")) z::e164()` |
| `z.iso.datetime({ offset, local, precision }) z.iso.date() z.iso.time() z.iso.duration()` | `z::iso::datetime_with(IsoDatetime {..})`, `z::iso::date()`, `z::iso::time_with(p)`, ... (`z::time_precision::MILLISECOND`) |
| `z.stringFormat("cod", /re/ \| fn)` | `z::string_format("cod", "re")`, `z::string_format_fn("cod", \|s\| ..)` |
| `z.number().int().gt().gte().lt().lte().positive().multipleOf().step()` | iguales (`gt(0.0)`, ...); `z::int()`, `z::int32()`, `z::uint32()`, `z::float32()`, `z::float64()` |
| `z.bigint() z.int64() z.uint64()`, `z.boolean()`, `z.null()`, `z.undefined()`, `z.void()`, `z.nan()` | iguales |
| `z.any() z.unknown() z.never() z.date()` | iguales (`z::date()` recibe las fechas de `z::coerce::date()` o de un transform; con `ParseOptions::date_strings(true)` también acepta strings ISO) |
| `z.literal("a")`, `z.literal(["a", 1, null])` | `z::literal("a")`, `z::literal(json!(["a", 1, null]))` |
| `z.enum(["a", "b"])`, `z.enum({ A: "x" })`, `z.nativeEnum(E)`, `.extract() .exclude()` | `z::enum_(["a", "b"])`, `z::enum_entries([("A", json!("x"))])`, `z::native_enum::<E>()`, `.extract(&[..]) .exclude(&[..])` |
| `z.stringbool({ truthy, falsy, case })` | `z::stringbool()`, `z::stringbool_with(StringBoolOptions {..})` |
| `z.templateLiteral(["id-", z.number()])` | `z::template_literal([TemplatePart::from("id-"), z::number().into()])` |
| `z.object({...})`, `z.strictObject`, `z.looseObject` | `z::object! { campo: s, "con-guion": s }`, `z::strict_object! {..}`, `z::loose_object! {..}` |
| `.strict() .strip() .passthrough() .loose() .catchall(s)` | iguales |
| `.extend() .safeExtend() .merge() .pick({ a: true }) .omit() .partial() .required() .keyof() .shape` | `.extend(o) .safe_extend(o) .merge(o) .pick(&["a"]) .omit(&[..]) .partial() .partial_fields(&[..]) .required() .required_fields(&[..]) .keyof() .shape()` |
| `z.array(s).min().max().length().nonempty()`, `s.array()` | iguales |
| `z.tuple([a, b], rest)` | `z::tuple([a, b]).rest(rest)` |
| `z.record(k, v)`, `z.partialRecord(k, v)`, `z.looseRecord(k, v)` | `z::record_with(k, v)`, `z::partial_record(k, v)`, `z::loose_record(k, v)` (`z::record(v)` usa claves string) |
| `z.union([..])`, `z.xor([..])`, `z.discriminatedUnion("k", [..])`, `z.intersection(a, b)` | `z::union([..])`, `z::xor([..])`, `z::discriminated_union("k", [..])`, `z::intersection(a, b)`; `a.or(b)`, `a.and(b)` |
| `.optional() .exactOptional() .nullable() .nullish() .nonoptional() .readonly() .brand()` | iguales (`.exact_optional()`) |
| `.default(v) .default(() => v) .prefault(v) .catch(v) .catch((ctx) => v)` | `.default(v) .default_fn(\|\| v) .prefault(v) .catch(v) .catch_fn(\|ctx\| v)` |
| `.refine(fn, { message, path, abort, when, params })` | `.refine(fn, "msg")`, `.refine(fn, z::params().message(..).path([..]).abort().when(..).params(json!(..)))`, `.refine_at(fn, msg, [..])` |
| `.superRefine((v, ctx) => ctx.addIssue({...}))`, `.check((ctx) => ...)`, `.overwrite(fn)` | `.super_refine(\|v, ctx\| ctx.add(z::issue().code(..).message(..).path([..])))`, `.check_fn(\|payload\| ..)`, `.overwrite(\|v\| ..)` |
| `.transform(fn)`, `.transform((v, ctx) => { ctx.addIssue(..); return z.NEVER })` | `.transform(\|v\| ..)`, `.try_transform(\|v, ctx\| { ctx.add(..); None })` |
| `.pipe(s)`, `z.preprocess(fn, s)`, `z.codec(a, b, { decode, encode })`, `z.invertCodec(c)` | iguales (`z::invert_codec(&c)`); `schema.decode(&v)`, `schema.encode(&v)` |
| `z.lazy(() => s)`, getters recursivos, `z.custom(fn)`, `z.json()`, `z.success(s)` | `z::lazy(f)`, `z::custom(fn, msg)`, `z::json()`, `z::success(s)` |
| `z.coerce.number()/string()/boolean()/bigint()/date()` | `z::coerce::number()/...` (`boolean` es `Boolean(x)`: `"false"` da `true`) |
| `schema.parse / safeParse / parse(v, { reportInput, error })` | `parse`, `safe_parse`, `parse_with(&v, &ParseOptions::new().report_input(true).error(..))`, `parse_as::<T>`, `parse_str`, `is_valid`; con `undefined`: `parse_opt(None, ..)`, `safe_parse_opt(None, ..)` |
| `z.flattenError / treeifyError / prettifyError / formatError`, `error.issues`, `error.message` | `z::flatten_error(&e)` (o `e.flatten()`), `z::treeify_error`, `z::prettify_error`, `z::format_error`, `e.issues`, `e.message()` |
| `z.config({ customError })`, `z.config(z.locales.es())` | `z::configure(\|c\| c.custom_error = Some(..))`, `z::configure(\|c\| c.locale_error = Some(z::locales::es()))` |
| `.describe()`, `.meta({ id, title })`, `z.registry()`, `z.globalRegistry`, `.register(r, meta)` | iguales (`z::Registry::new()`, `z::global_registry()`) |
| `z.toJSONSchema(s, { target, io, unrepresentable, cycles, reused, override })` | `z::to_json_schema(&s, &JsonSchemaOptions::new().target("draft-07").input()...)` o `s.to_json_schema()` |

Diferencias propias de Rust:

- Los valores son `serde_json::Value`. `undefined` es la ausencia de valor (`parse_opt(None, ..)`
  o una clave que falta) y una salida `undefined` se omite en los objetos, igual que
  `JSON.stringify`. Los números siguen la semántica de JavaScript (`f64`, `Number(x)`,
  `String(x)`) y las claves de los objetos se ordenan como en V8 (los índices numéricos primero).
- `.refine()`, `.super_refine()`, `.overwrite()` y `.check_fn()` devuelven un `Schema`, así que
  los checks del tipo (`.min()`, ...) van antes que ellos. zod también acepta
  `z.string().refine(..).min(3)`; ese orden no se puede escribir en Rust.
- Lo que zod *lanza* como `Error` (una intersección que no se puede combinar, un `transform` al
  hacer `encode`) es un `panic!` en Rust. `ZodValidationPipe` y el serializer lo convierten en la
  misma respuesta que la integración original.
- Schemas recursivos: una función que devuelve el schema y `z::lazy(f)` donde se repite
  (`fn categoria() -> Schema { z::object! { hijos: z::array(z::lazy(categoria)) }.into() }`).
  Para que `to_json_schema` produzca `"$ref": "#"` como zod, use `z::lazy(categoria)` como raíz.
- `z.map`, `z.set`, `z.symbol`, `z.file`, `z.promise`, `z.function` y `z.instanceof` no
  existen porque describen valores que no viajan en JSON. De los locales solo están `en` (por
  defecto) y `es`; otros idiomas se agregan con `locale_error` y `z::error_fn`.

### DTOs, pipes y serialización con zod

| Node.js (integración zod 5) | Corvis |
|---|---|
| `class Dto extends createZodDto(schema)`, `{ codec: true }`, `Dto.create(input)` | `zod_dto!(Dto, schema)`, `zod_dto!(Dto, schema, codec = true)`, `Dto::create(&input)` |
| `new ZodValidationPipe(schemaOrDto)` | `ZodValidationPipe::new(schema)`, `ZodValidationPipe::dto::<Dto>()` |
| `{ provide: APP_PIPE, useClass: ZodValidationPipe }` | `app.use_global_pipes(ZodValidationPipe::auto())` |
| `createZodValidationPipe({ createValidationException, strictSchemaDeclaration })` | `z::create_zod_validation_pipe(ZodValidationPipeOptions::new().create_validation_exception(\|e\| ..).strict_schema_declaration(true))` |
| `ZodValidationException`, `getZodError()` | `z::ZodValidationException::new(e)`, `ZodValidationException::get_zod_error(&ex)` |
| `ZodSerializationException`, `ZodSchemaDeclarationException` | iguales (`z::...::new(..)`) |
| `@ZodSerializerDto(Dto)` / `([Dto])` + `ZodSerializerInterceptor` | `#[set_metadata(z::ZOD_SERIALIZER_DTO_OPTIONS, z::ZodSerializerDto::dto::<Dto>())]` (`dto_array`, `schema`, `schema_array`) + `app.use_global_interceptors(z::ZodSerializerInterceptor::new())` |
| `createZodSerializerInterceptor({ reportInput })` | `z::ZodSerializerInterceptor::create(Some(true))` |
| `@ZodResponse({ status, type })` | `#[apply_decorators(corvis_swagger::decorators::zod_response::<Dto>(Some(status), None))]`: código de estado + `ZodSerializerDto` + la respuesta OpenAPI (`Dto_Output`) |
| `ZodGuard`, `UseZodGuard('body', Dto)`, `createZodGuard({ createValidationException })` | `#[use_guards(z::ZodGuard::dto::<Dto>(z::ZodGuardSource::Body))]`, `.create_validation_exception(..)` |
| `validate(value, schemaOrDto, createValidationException?)` | `z::validate(&value, &schema, None)` |

```rust
#[derive(Deserialize)]
pub struct CreateCatDto { pub name: String, pub age: u32 }

zod_dto!(CreateCatDto, z::strict_object! {
    name: z::string().trim().min(1),
    age: z::int().nonnegative(),
});

app.use_global_pipes(ZodValidationPipe::auto());

// Schema explícito en un parámetro (query strings con coerce):
#[get]
fn list(&self, #[query(ZodValidationPipe::new(z::object! {
    page: z::coerce::number().int().min(1.0).default(1),
    active: z::stringbool().optional(),
}))] q: ListQuery) -> Vec<Cat> { ... }
```

La salida del schema (defaults aplicados, claves desconocidas eliminadas, transformaciones
ejecutadas) es la que se deserializa en el tipo del parámetro. Un body, query, parámetro o
header ausente se valida como `undefined`, igual que en Express. Cuando falla, responde:

```json
{
  "statusCode": 400,
  "message": "Validation failed",
  "errors": [
    { "origin": "string", "code": "too_small", "minimum": 1, "inclusive": true,
      "path": ["name"], "message": "Too small: expected string to have >=1 characters" },
    { "code": "unrecognized_keys", "keys": ["extra"], "path": [],
      "message": "Unrecognized key: \"extra\"" }
  ]
}
```

`ZodValidationPipe` y `ValidationPipe` pueden convivir: cada uno ignora los tipos que no
le corresponden.

La fidelidad se verifica contra los paquetes reales: `tests/fixtures/zod4/cases.mjs` ejecuta
zod 4.4.3 (y su build `zod/v3`) sobre casi 300 schemas con unas 600 entradas y casi 100 casos
de `toJSONSchema`, y
`tests/fixtures/zod4/pipe.mjs` ejecuta la integración original 5.5.0. `tests/zod4_fixtures.rs` y
`tests/zod_pipe.rs` exigen el mismo resultado, incluido el orden de las claves.

## ORM (`corvis-orm`)

`crates/corvis-orm` es un ORM sobre sqlx (PostgreSQL y SQLite) con consultas por argumentos
JSON (`where`, `include`, `orderBy`...) o helpers tipados, integrado con la DI de Corvis mediante
`OrmModule` y `OrmService`. El binario `corvis-orm` genera los modelos desde archivos
`schema.prisma` (formato compatible) y aplica las migraciones SQL. La documentación completa está
en [`crates/corvis-orm/README.md`](crates/corvis-orm/README.md).

```rust
use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
pub struct User {
    #[id] #[default(autoincrement)] pub id: i64,
    #[unique] pub email: String,
    pub name: Option<String>,
    #[relation(has_many)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<Post>>,
}

#[injectable]
pub struct UsersService {
    db: Arc<OrmService>,
}

impl UsersService {
    pub async fn search(&self, q: &str) -> Result<Vec<User>, OrmError> {
        // Argumentos JSON...
        self.db.user().find_many(json!({
            "where": { "OR": [{ "email": { "contains": q } }, { "name": { "contains": q, "mode": "insensitive" } }] },
            "include": { "posts": { "where": { "published": true } } },
            "orderBy": { "id": "desc" },
            "take": 20,
        })).await
        // ...o con helpers tipados: self.db.user().find_many(user::email::contains(q)).take(20).exec().await
    }
}

#[module(imports = [OrmModule::for_root_from_config(true)], providers = [UsersService])]
pub struct UsersModule;

app.use_global_filters(OrmExceptionFilter::new()); // P2002 → 409, P2025 → 404
```

`examples/blog-orm` es una API REST completa (usuarios y posts con paginación, búsqueda,
relaciones y escrituras anidadas) que combina `corvis-orm` con DTOs validados mediante
`corvis::z`:

```bash
cargo run -p blog-orm        # http://127.0.0.1:3000 (DATABASE_URL=file:./dev.db por defecto)
```

## OpenAPI y Swagger (`corvis-swagger`)

`crates/corvis-swagger` genera documentos OpenAPI 3 (`DocumentBuilder`,
`SwaggerModule::create_document` y `setup`, los atributos `#[api_*]`), incluye los DTOs zod en el
documento (`cleanup_open_api_doc`) y sirve Swagger UI y la referencia de Scalar. El documento que
genera es el mismo, byte a byte, que el de la app equivalente en Node.js: mismas claves, mismo
orden y los mismos `operationId`.

```rust
use corvis_swagger::*;

#[controller("cats")]
#[api_tags("cats")]
#[api_bearer_auth]
#[api_unauthorized_response(description = "Missing or invalid token")]
pub struct CatsController { /* ... */ }

#[routes]
impl CatsController {
    #[post]
    #[api_operation(summary = "Create a cat")]
    #[api_created_response(type = CatDto)]
    #[api_conflict_response(description = "Duplicated")]
    fn create(&self, #[body] dto: CreateCatDto) -> CatDto { /* ... */ }

    #[get]
    #[api_ok_response(type = [CatDto])]
    fn list(&self, #[query] q: ListCatsQuery) -> Vec<CatDto> { /* ... */ }
}

let config = DocumentBuilder::new().set_title("Cats").set_version("1.0").add_bearer_auth().build();
let document = cleanup_open_api_doc(SwaggerModule::create_document(&app, &config));
SwaggerModule::setup("docs", &mut app, document.clone());          // /docs, /docs-json, /docs-yaml
scalar::mount(&mut app, "/reference", ApiReferenceOptions::new().document(&document));
```

| Node.js (TypeScript) | `corvis-swagger` |
|---|---|
| `@ApiTags('x')`, `@ApiBearerAuth()`, `@ApiOperation({ summary })` | `#[api_tags("x")]`, `#[api_bearer_auth]`, `#[api_operation(summary = "...")]` |
| `@ApiOkResponse({ type: Dto })`, `@ApiCreatedResponse({ type: [Dto] })`, `@Api<Status>Response(...)`, `@ApiResponse({ status, ... })` | `#[api_ok_response(type = Dto)]`, `#[api_created_response(type = [Dto])]`, `#[api_not_found_response(description = "...")]`, `#[api_response(status = 418, ...)]` |
| `@ApiParam`, `@ApiQuery`, `@ApiHeader`, `@ApiBody`, `@ApiProduces`, `@ApiConsumes`, `@ApiExtraModels`, `@ApiExtension`, `@ApiExcludeEndpoint`, `@ApiSecurity`, ... | `#[api_param(name = "id", example = "...")]`, `#[api_query(...)]`, `#[api_produces("application/pdf")]`, ... |
| clase con `@ApiProperty()` | `#[derive(ApiSchema)]` con `#[api_property(...)]` |
| `createZodDto(schema)` en `@Body()`/`@Query()` y en `type` | `zod_dto!(Dto, schema)`: el body se referencia en `components.schemas` y un query DTO se expande en parámetros |
| `@ZodResponse({ status, description, type })` | `#[apply_decorators(corvis_swagger::decorators::zod_response::<Dto>(Some(201), None))]` (`zod_response_array` para `[Dto]`) |
| `SwaggerModule.createDocument(app, config, options)` | `SwaggerModule::create_document_with(&app, &config, &SwaggerDocumentOptions { .. })` |
| `SwaggerModule.setup(path, app, doc, { jsonDocumentUrl, swaggerOptions, customCss, ... })` | `SwaggerModule::setup_with(path, &mut app, doc, SwaggerCustomOptions { .. })`; `setup_lazy` para `() => document` |
| `cleanupOpenApiDoc(doc)` | `cleanup_open_api_doc(doc)` |
| `app.use('/reference', apiReference({ content }))` | `scalar::mount(&mut app, "/reference", ApiReferenceOptions::new().document(&doc))` |

- Los atributos `#[api_*]` van debajo de `#[controller]` / de `#[get]`, en el mismo orden que los
  decoradores de TypeScript (el orden decide el de las respuestas).
- `SwaggerModule::setup` registra rutas planas de Express (no salen en los logs `Mapped`, no pasan
  por guards): la página, `swagger-ui-init.js`, `-json` y `-yaml`. Los archivos de
  `swagger-ui-dist` se sirven desde `custom_swagger_ui_path` o, si no se indica, se redirigen al
  mismo archivo en jsDelivr.
- La página de Scalar es la misma que genera `@scalar/client-side-rendering` 0.3 (con su tema por
  defecto) y responde cualquier método bajo la ruta montada, como `app.use(path, ...)`.
  `ApiReferenceOptions::integration("...")` agrega el campo `_integration` cuando hace falta.
- Las pruebas están en `crates/corvis-swagger/tests/swagger_app.rs` y en los tests unitarios del
  crate. El port de facten (74 rutas) sirve el mismo `/reference` que la app original, byte a byte.

## Rendimiento

El benchmark reproducible está en `bench/`: dos apps con los mismos endpoints, una de Node.js
con un framework equivalente (sobre Express y sobre Fastify) y otra de Corvis en release,
medidas con [oha](https://github.com/hatoo/oha) en `127.0.0.1`. Para repetirlo: `bash bench/run.sh`. Los resultados detallados se escriben en
[`bench/results.md`](bench/results.md).

Resultados en un i9-14900KF (32 hilos) con Windows 11, Node 24 y rustc 1.99. Se usaron 100
conexiones keep-alive y 15 s por escenario, con cliente y servidor en la misma máquina:

| Escenario | Node.js + Express | Node.js + Fastify | Corvis (1 worker) | Corvis (todos los núcleos) |
|---|---:|---:|---:|---:|
| `GET /` texto | 713 mil req/min | 1.02 M req/min | 1.54 M req/min | **7.15 M req/min** |
| `GET /json` | 680 mil | 963 mil | 1.45 M | **5.26 M** |
| `GET /users/:id` + `ParseIntPipe` | 647 mil | 879 mil | 1.48 M | **7.59 M** |
| `POST /users` + `ValidationPipe` | 446 mil | 547 mil | 1.42 M | **3.42 M** |
| p50 `GET /` | 8.98 ms | 4.46 ms | 4.03 ms | 0.73 ms |
| RSS bajo carga | ~165–205 MB | ~163–344 MB | ~11 MB | ~9 MB |

Todas las mediciones terminaron con 0 errores y 0 respuestas non-2xx.

- **Con un solo worker** (comparación por núcleo contra un proceso Node sin cluster), Corvis
  procesa 2.1–2.3 veces más requests que la app de Node.js sobre Express en los GET y 3.2
  veces más en el POST con validación.
- **Con todos los núcleos**, la ventaja sube a 8–12 veces. Ese número lo limita sobre todo el
  entorno: oha y el loopback de Windows se saturan cerca de 110–120k req/s, igual que con un
  servidor hyper sin framework.
- **La fila del POST tuvo más varianza entre corridas**: 57k–100k req/s.
- **En memoria**, Corvis usa ~15–35 veces menos.

## Limitaciones conocidas

- **Scope REQUEST**: no existe. Para datos por request usa `req.insert`/`req.get` (las
  extensiones de la request) o `FromContext`. Sí hay `DEFAULT` y `TRANSIENT`.
- **Microservicios y GraphQL**: no están implementados.
- **Body parser**: los cuerpos comprimidos (`Content-Encoding: gzip`, `deflate`, `br`) se rechazan con
  `415 unsupported content encoding`; Express los descomprime. Un error del parser se responde antes
  de los middlewares globales de `app.use()`, que en Express corren antes del parser.
- **Subida de archivos**: el cuerpo `multipart/form-data` se lee completo en memoria antes de
  que corra el interceptor (no hay streaming hacia el disco); usa `limits` para acotarlo.
- **Pipes sobre JSON**: los pipes reciben y devuelven `serde_json::Value`. El valor final
  se deserializa en el tipo del parámetro.
- **Orden de los atributos**: los atributos de clase (`#[use_guards]`, `#[set_metadata]`,
  ...) deben ir debajo de `#[controller]`, porque Rust procesa los atributos en orden.
- **Interceptors sin RxJS**: `CallHandler::handle()` devuelve un `Future`, no un
  `Observable`. Puedes transformar, cronometrar, cachear y aplicar timeouts, pero no
  componer operadores.
- **Middleware estilo Express**: recibe la `Request` y un `Next`. No hay un objeto `res`
  mutable; la respuesta se modifica sobre el resultado de `next.run(req)` (en los handlers
  sí existe: `#[res]`).

## Licencia

Corvis se publica bajo la licencia MIT (ver [`LICENSE`](LICENSE)). Los avisos de copyright y las
licencias de los proyectos cuyo comportamiento, API o lógica replica están en
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
