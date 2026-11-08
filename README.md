# Corvis

**Corvis** is a modular backend framework for Rust: modules, dependency injection, controllers declared with
attributes, pipes, guards, interceptors, exception filters, middleware, lifecycle hooks, testing utilities, a CLI and
the modules most applications need (configuration, JWT, events, task scheduling, cache, rate limiting, health checks,
static files, WebSockets).

If you come from Node.js and TypeScript, the model will feel familiar: decorators become attributes
(`@Get(':id')` → `#[get(":id")]`), classes become `struct`s, and constructors are generated from the `Arc<T>` fields.

**Documentation:** [corvis.randygil.com](https://corvis.randygil.com) ·
**Documentación en español:** [corvis.randygil.com/es/](https://corvis.randygil.com/es/)

```rust,no_run
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

## Contents

- [Quick start with the CLI](#quick-start-with-the-cli)
- [Repository layout](#repository-layout)
- [Coming from Node.js](#coming-from-nodejs)
- [Features at a glance](#features-at-a-glance)
- [ORM (`corvis-orm`)](#orm-corvis-orm)
- [OpenAPI and Swagger (`corvis-swagger`)](#openapi-and-swagger-corvis-swagger)
- [Performance](#performance)
- [Security](#security)
- [Known limitations](#known-limitations)
- [License](#license)

## Quick start with the CLI

```bash
cargo install --path crates/corvis-cli      # installs the `corvis` binary

corvis new my-app                            # new project (AppModule, controller, service, test)
cd my-app
corvis generate resource cats                # full CRUD: module, controller, service, DTOs, entity, test
corvis g controller users --flat             # single schematics
corvis g service users --no-spec
corvis start --watch                         # cargo run, restarts when files change
corvis build                                 # cargo build (debug); add --release or --prod for an optimized build
corvis info
```

Available schematics: `module`, `controller`, `service`, `resource`, `guard`, `interceptor`, `pipe`, `filter`,
`middleware`, `gateway`. The CLI registers what it generates in the right module (`imports`, `controllers`,
`providers`) and adds the `mod` declarations. `corvis new` depends on the published `corvis` version that matches the
CLI by default; `--corvis-path <path>` or the `CORVIS_PATH` variable select a local checkout, `--corvis-version
<version>` another version, and `--skip-git` skips `git init`.

Next: [First steps](https://corvis.randygil.com/first-steps) and the [CLI reference](https://corvis.randygil.com/cli/usages).

## Repository layout

| Path | Contents |
|---|---|
| `crates/corvis` | Runtime: DI, HTTP (hyper), router, request pipeline, built-in modules, testing, `z` schemas |
| `crates/corvis-macros` | The `#[module]`, `#[injectable]`, `#[controller]`, `#[routes]`, `#[handlers]`, `#[websocket_gateway]` and `#[derive(Validate)]` attributes |
| `crates/corvis-cli` | The `corvis` CLI (`new`, `generate`, `start`, `build`, `info`) |
| `crates/corvis-orm`, `crates/corvis-orm-macros` | ORM on top of sqlx (models, JSON queries, migrations) and `OrmModule` |
| `crates/corvis-swagger`, `crates/corvis-swagger-macros` | OpenAPI: document, Swagger UI, zod DTOs and the Scalar reference |
| `examples/cats-app` | Complete app: JWT auth, roles, CRUD, versioning, SSE, cache, events, cron, WebSockets |
| `examples/blog-orm` | REST API with `corvis-orm` (SQLite) |
| `examples/tertulia` | Real-time chat over WebSockets: rooms, presence, guards, zod, acks, SSE, `corvis::ws` and a web client |
| `bench/` | Reproducible benchmark against an equivalent Node.js framework |
| `editors/vscode` | VS Code extension: snippets, CLI commands, route explorer and highlighting ([README](editors/vscode/README.md)) |
| `docs/` | The documentation site (VitePress; English with a Spanish translation) |

```bash
cargo test --workspace                       # all tests
cargo run -p cats-app                        # http://127.0.0.1:3000/api/v1
```

## Coming from Node.js

Each table shows, on the left, the usual form of a piece in a decorator-based Node.js framework written in TypeScript
and, on the right, its Corvis equivalent. The [migration guide](https://corvis.randygil.com/migration-guide) goes
further.

### Structure

| Node.js (TypeScript) | Corvis |
|---|---|
| `@Module({ imports, controllers, providers, exports })` | `#[module(imports = [...], controllers = [...], providers = [...], exports = [...])]` |
| `@Global()` | `#[global]` on the line **below** `#[module(...)]`, or `#[module(global, ...)]` |
| `static forRoot(): DynamicModule` | `fn for_root(...) -> DynamicModule` (`DynamicModule::new::<Self>()...`) |
| Module with `configure(consumer)` for middleware | `impl CorvisModule for AppModule { fn configure(&self, consumer: &mut MiddlewareConsumer) }` |
| `@Injectable()` | `#[injectable]` |
| `constructor(private catsService: CatsService)` | field `cats_service: Arc<CatsService>` |
| `@Inject('TOKEN')` | `#[inject("TOKEN")] field: Arc<T>` |
| `@Optional()` | `field: Option<Arc<T>>` |
| `{ provide, useValue }` | `Provider::use_value("TOKEN", value)` |
| `{ provide, useClass }` | `Provider::use_class::<Impl>("TOKEN")`; for trait objects `Provider::use_class_as::<PgRepo, dyn Repo>("REPO", \|r\| r)` |
| `{ provide, useFactory, inject }` | `Provider::use_factory("TOKEN", \|ctx\| async move { ... })` |
| `{ provide, useExisting }` | `Provider::use_existing("ALIAS", Token::of::<CatsService>())` |
| `Scope.TRANSIENT` | `#[injectable(scope = "transient")]` |
| `ModuleRef` (`get`, `resolve`, `create`) | field `module_ref: ModuleRef` |
| `APP_GUARD` / `APP_PIPE` / `APP_INTERCEPTOR` / `APP_FILTER` | `Provider::app_guard::<G>()`, `app_pipe`, `app_interceptor`, `app_filter` |

### Controllers

| Node.js (TypeScript) | Corvis |
|---|---|
| `@Controller('cats')` | `#[controller("cats")]` (`#[controller(path = "cats", host = "...", version = "1")]`) |
| `@Get()`, `@Post(':id')`, `@Put`, `@Patch`, `@Delete`, `@All`, `@Options`, `@Head` | `#[get]`, `#[post(":id")]`, `#[put]`, `#[patch]`, `#[delete]`, `#[all]`, `#[options]`, `#[head]` |
| `@Sse('stream')` | `#[sse("stream")]` returning `Sse` |
| `@HttpCode(204)` | `#[http_code(HttpStatus::NO_CONTENT)]` |
| `@Header('Cache-Control', 'none')` | `#[header("Cache-Control", "none")]` |
| `@Redirect('url', 301)` | `#[redirect("url", 301)]` |
| `@Version('2')` | `#[version("2")]` |
| `@Body()`, `@Body('name')` | `#[body] dto: T`, `#[body("name")] name: String` |
| `@Param('id', ParseIntPipe)` | `#[param("id", ParseIntPipe)] id: u32` |
| `@Query()` | `#[query] q: T` (strings are converted to numbers or booleans according to the type) |
| `@Headers('authorization')` | `#[headers("authorization")] auth: Option<String>` |
| `@Ip()`, `@Req()` | `#[ip] ip: String`, `#[req] req: Request` |
| `@UploadedFile()`, `@UploadedFiles()` | `#[uploaded_file] file: Option<UploadedFile>`, `#[uploaded_files] files: Vec<UploadedFile>` (see [File upload](https://corvis.randygil.com/http/file-upload)) |
| `createParamDecorator(...)` | `impl FromContext for CurrentUser` (a parameter without an attribute) |
| `@SetMetadata('roles', ['admin'])` | `#[set_metadata("roles", ["admin"])]` |
| `@UseGuards`, `@UseInterceptors`, `@UsePipes`, `@UseFilters` | `#[use_guards(...)]`, `#[use_interceptors(...)]`, `#[use_pipes(...)]`, `#[use_filters(...)]` (a type or an expression) |
| `@Res()` / `@Res({ passthrough: true })` | `#[res] res: Res` / `#[res(passthrough)] res: Res` (see [Controllers](https://corvis.randygil.com/controllers)) |
| Special responses | return `Response`, `Json`, `Html`, `Redirect`, `StreamableFile` |
| `throw new NotFoundException('msg')` | `Err(NotFoundException::new("msg"))?` / `HttpResult<T>` |
| `new HttpException({...}, 418)` | `HttpException::new(json!({...}), 418)` |

Class-level attributes (`#[use_guards]`, `#[set_metadata]`, ...) go **below** `#[controller]`.

### Request pipeline

| Node.js (TypeScript) | Corvis |
|---|---|
| Middleware with `use(req, res, next)` | `CorvisMiddleware::use_(&self, req, next) -> HandlerResult` |
| `consumer.apply(M).exclude(...).forRoutes(...)` | `consumer.apply::<M>().exclude(...).for_routes("cats")` |
| `CanActivate.canActivate(ctx)` | `CanActivate::can_activate(&self, ctx) -> Result<bool, HttpException>` |
| Interceptor with `intercept(ctx, next)` + RxJS `map` | `intercept(&self, ctx, next: CallHandler)`; `next.handle().await?`, then transform the response |
| `PipeTransform.transform(value, metadata)` | `PipeTransform::transform(&self, value: Value, meta) -> Result<Value, HttpException>` |
| `ExceptionFilter.catch(exception, host)` + `@Catch(T)` | `ExceptionFilter::{catches, catch}` |
| `Reflector.getAllAndOverride(key, [handler, class])` | `reflector.get_all_and_override(KEY, &[ctx.get_handler(), ctx.get_class()])` |
| `ValidationPipe({ whitelist, forbidNonWhitelisted, transform })` | `ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true)` |
| class-validator (`@IsString()`, `@IsEmail()`, `@Min(0)`, `@ValidateNested()`...) | `#[derive(Validate)]` with `#[is_string]`, `#[is_email]`, `#[min(0)]`, `#[validate_nested]`... |

Execution order: middleware → guards → interceptors (before) → pipes → handler → interceptors (after) → exception
filters. Errors produce JSON bodies in the usual format, for example
`{"message":"Cat #42 not found","error":"Not Found","statusCode":404}` or
`{"statusCode":500,"message":"Internal server error"}`. See
[Request lifecycle](https://corvis.randygil.com/faq/request-lifecycle).

### Application

| Node.js (TypeScript) | Corvis |
|---|---|
| Create the application from the root module | `CorvisFactory::create::<AppModule>().await?` |
| Application context without HTTP (standalone) | `CorvisFactory::create_application_context::<M>()` |
| `app.setGlobalPrefix('api')` | `app.set_global_prefix("api")` |
| `app.useGlobalPipes/Guards/Interceptors/Filters` | `app.use_global_pipes(...)`, etc. |
| `app.use(fn)` | `app.use_fn(...)` / `app.use_middleware(...)` |
| `app.enableCors()` | `app.enable_cors(CorsOptions::default())` (semantics of the `cors` package, see [CORS](https://corvis.randygil.com/security/cors)) |
| `app.disable('x-powered-by')`, `app.set('etag', 'strong')` | `app.disable("x-powered-by")`, `app.set_etag(EtagMode::Strong)`, `app.set_x_powered_by("Express")` |
| `HttpAdapterHost`, `app.getHttpServer()`, `server.on('upgrade')` | injectable `Arc<HttpAdapterHost>`, `app.get_http_server()`, `server.on_upgrade(...)` + `corvis::ws` |
| `app.enableVersioning({ type: URI })` | `app.enable_versioning(VersioningOptions::uri().default_version("1"))` (also header and media type) |
| `app.enableShutdownHooks()` | `app.enable_shutdown_hooks()` |
| `server.headersTimeout`, `requestTimeout`, `keepAliveTimeout`, `maxConnections` | `app.set_server_options(ServerOptions::new().keep_alive_timeout(..)...)` (see [HTTP adapters](https://corvis.randygil.com/http/performance#timeouts-and-connections)) |
| `app.get(CatsService)` | `app.get::<CatsService>()?` |
| `app.listen(3000)` | `app.listen("127.0.0.1:3000").await` (accepts a port, a `&str` or a `SocketAddr`) |
| `OnModuleInit`, `OnApplicationBootstrap`, `OnModuleDestroy`, `BeforeApplicationShutdown`, `OnApplicationShutdown` | the same traits; they are detected automatically when implemented |
| `new Logger(CatsService.name)` | `Logger::new("CatsService")` (console format `[Corvis] pid - date LEVEL [context] message`) |

### Built-in modules

| Node.js (TypeScript) | Corvis |
|---|---|
| Configuration (`ConfigModule.forRoot`, `ConfigService.get`, `registerAs`) | `ConfigModule::for_root(ConfigModuleOptions {...})` (reads `.env`), `ConfigService::get/get_or/get_or_throw`, `corvis::config::register_as(ns, factory)` passed to `ConfigModule::for_feature(..)` or to `ConfigModuleOptions { load: vec![..], .. }` |
| JWT (`JwtModule`, `JwtService`) | `JwtModule::register(...)` / `register_async`, `JwtService::{sign, verify, decode}` |
| Events (`@OnEvent`) | `EventEmitterModule::for_root(...)`, `EventEmitter::emit`, `#[on_event("cat.created")]` in a `#[handlers] impl`; `*` and `**` patterns only with `EventEmitterOptions { wildcard: true, .. }` |
| Task scheduling (`@Cron`, `@Interval`, `@Timeout`, `SchedulerRegistry`) and `cron` (`CronJob`, `CronTime`) | `ScheduleModule::for_root()` / `for_root_with` / `for_root_async`, `#[cron(expr, name = ..., time_zone = ...)]`, `#[interval("name", ms)]`, `#[timeout(ms)]`, `CronJob::from(CronJobParams)`, `set_interval`/`set_timeout` |
| Cache (`CacheInterceptor`, `@CacheTTL`) | `CacheModule::register(...)`, `CacheManager`, `#[use_interceptors(CacheInterceptor)]`, `#[set_metadata(CACHE_TTL_METADATA, 5_000)]` |
| Rate limiting (`ThrottlerGuard`, `@SkipThrottle`) | `ThrottlerModule::for_root(...)`, `ThrottlerGuard`, `#[set_metadata(SKIP_THROTTLE, true)]` |
| Health checks | `TerminusModule`, `HealthCheckService`, HTTP and memory indicators |
| Static files (`ServeStaticModule`) | `ServeStaticModule::for_root(vec![ServeStaticOptions::new("public").serve_root("/static")])` |
| WebSockets (`@WebSocketGateway`, `@SubscribeMessage`, `@WebSocketServer`, rooms) | `#[websocket_gateway(path = "/chat")]`, `#[subscribe_message("msg")]`, `#[websocket_server] server: WsServer`, `client.join(room)`, `server.to(room).emit(...)` |
| Testing (`Test.createTestingModule`, `overrideProvider`, `overrideGuard`) + supertest | `Test::create_testing_module()`, `.override_provider::<T>().use_value(...)`, `.override_guard::<G>(mock)`, `request(&app).get("/").send().await.expect_status(200)` |

## Features at a glance

One short example per feature; each links to the page that covers it in full.

### Modules and providers

A module only sees its own providers, the `exports` of the modules it imports (including re-exports) and global
modules. A global module is declared once and its exports are visible everywhere. If a dependency is missing, the
error names it and the module: `Corvis can't resolve dependencies of the CatsService (?). Please make sure that the
argument API_KEY at index [0] is available in the CatsModule context.` Circular dependencies are detected, and
lifecycle hooks run from the deepest modules up.

```rust,no_run
use corvis::prelude::*;

#[injectable]
pub struct AuditService;

impl AuditService {
    pub fn record(&self, action: &str) {
        println!("audit: {action}");
    }
}

// A global module: `#[global]` goes on the line below `#[module(...)]`
// (`#[module(global, providers = [...], exports = [...])]` is equivalent).
#[module(providers = [AuditService], exports = [AuditService])]
#[global]
pub struct AuditModule;

#[injectable]
pub struct CatsService {
    audit: Arc<AuditService>, // resolved from the global module, without importing it
}

impl CatsService {
    pub fn create(&self, name: &str) {
        self.audit.record(&format!("cat {name} created"));
    }
}

#[module(providers = [CatsService], exports = [CatsService])]
pub struct CatsModule;

#[module(imports = [AuditModule, CatsModule])]
pub struct AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let app = CorvisFactory::create::<AppModule>().await?;
    app.listen("127.0.0.1:3000").await
}
```

Custom providers (`use_value`, `use_class`, `use_factory`, `use_existing`), injection tokens and field initializers:

```rust,ignore
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

More: [Modules](https://corvis.randygil.com/modules) (including
[global modules](https://corvis.randygil.com/modules#global-modules)),
[Providers](https://corvis.randygil.com/providers),
[Custom providers](https://corvis.randygil.com/fundamentals/custom-providers).

### Configuration

`ConfigModule` reads `.env` files and the process environment. Namespaced configuration is created with the free
function `corvis::config::register_as` and added with `ConfigModule::for_feature` (or listed in
`ConfigModuleOptions::load`):

```rust,no_run
use corvis::config::register_as;
use corvis::prelude::*;

#[injectable]
pub struct PaymentsService {
    config: Arc<ConfigService>,
}

impl PaymentsService {
    pub fn currency(&self) -> String {
        self.config.get_or_throw("payments.currency")
    }
}

#[module(
    imports = [ConfigModule::for_feature(register_as("payments", || json!({ "currency": "USD" })))],
    providers = [PaymentsService],
)]
pub struct PaymentsModule;

#[module(imports = [
    // `for_feature` resolves `ConfigService` from the module context, so the root module registers it as global.
    ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
    PaymentsModule,
])]
pub struct AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let app = CorvisFactory::create::<AppModule>().await?;
    app.listen("127.0.0.1:3000").await
}
```

`.env` values are taken verbatim (`JWT_SECRET=pa$$w0rd` stays `pa$$w0rd`); variable expansion is opt-in with
`expand_variables: true`. More: [Configuration](https://corvis.randygil.com/application/configuration).

### Validation with DTOs

```rust,ignore
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

Error response:
`{"message":["name should not be empty","age must not be less than 0"],"error":"Bad Request","statusCode":400}`.

Corvis also ships `corvis::z`, a port of [zod](https://zod.dev) 4 with the same error codes, messages and JSON shape,
plus `zod_dto!`, `ZodValidationPipe` and a serializer interceptor:

```rust,ignore
let create_user = z::object! {
    name: z::string().trim().min(2),
    email: z::email(),
    role: z::enum_(["admin", "user"]).default("user"),
};
let data = create_user.parse(&json!({ "name": "Ana", "email": "ana@corvis.dev" }))?;
// {"name":"Ana","email":"ana@corvis.dev","role":"user"}
```

More: [Validation](https://corvis.randygil.com/application/validation) (class-validator style and zod style),
[Pipes](https://corvis.randygil.com/pipes),
[Serialization](https://corvis.randygil.com/application/serialization).

### Guards and custom decorators

```rust,ignore
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

// In the controller:
#[delete(":id")]
#[set_metadata(ROLES_KEY, ["admin"])]
#[use_guards(RolesGuard)]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> { ... }

// The equivalent of a @CurrentUser() parameter decorator:
pub struct CurrentUser(pub JwtPayload);

#[async_trait]
impl FromContext for CurrentUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ctx.request().user_as::<JwtPayload>().map(CurrentUser).ok_or_else(|| UnauthorizedException.into())
    }
}
```

More: [Guards](https://corvis.randygil.com/guards),
[Custom decorators](https://corvis.randygil.com/custom-decorators),
[Authentication](https://corvis.randygil.com/security/authentication),
[Authorization](https://corvis.randygil.com/security/authorization).

### Events and task scheduling

```rust,no_run
use std::sync::Mutex;

use corvis::prelude::*;

#[derive(Clone, Debug)]
pub struct CatCreated {
    pub name: String,
}

#[injectable]
pub struct CatsService {
    events: Arc<EventEmitter>,
}

impl CatsService {
    pub fn create(&self, name: &str) {
        self.events.emit("cat.created", CatCreated { name: name.to_string() });
    }
}

#[injectable]
pub struct CatsListener {
    #[default]
    seen: Mutex<Vec<String>>,
}

#[handlers]
impl CatsListener {
    // Receives `cat.created`, `cat.deleted`, ... because the emitter below has `wildcard: true`.
    // Without it, "cat.*" only matches an event literally named `cat.*`.
    #[on_event("cat.*")]
    async fn on_cat(&self, event: CatCreated) {
        self.seen.lock().unwrap().push(event.name);
    }

    #[cron(CronExpression::EVERY_HOUR, name = "cats-report")]
    async fn report(&self) {
        println!("{} cats created", self.seen.lock().unwrap().len());
    }
}

#[module(
    imports = [
        EventEmitterModule::for_root(EventEmitterOptions { wildcard: true, ..Default::default() }),
        ScheduleModule::for_root(),
    ],
    providers = [CatsService, CatsListener],
)]
pub struct AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let app = CorvisFactory::create::<AppModule>().await?;
    app.listen("127.0.0.1:3000").await
}
```

`corvis::schedule` follows the `cron` 4 package and the usual `@Cron`/`@Interval`/`@Timeout` decorators: 5- or
6-field expressions, IANA time zones (`time_zone = "America/Caracas"`) or `utc_offset`, daylight-saving transitions,
`SchedulerRegistry`, `CronJob` and `CronTime`. Date computation was compared with `cron` 4.4 on Node.js over 48
expressions × 9 time zones × 7 start instants, with no differences.

More: [Events](https://corvis.randygil.com/application/events),
[Task scheduling](https://corvis.randygil.com/application/task-scheduling).

### Testing

```rust,ignore
use corvis::testing::{Test, request};

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

`examples/cats-app` has the complete example with authentication, roles, versioning, SSE, cache, rate limiting,
events, cron and WebSockets. More: [Testing](https://corvis.randygil.com/fundamentals/testing).

### WebSocket gateways

A gateway is a provider with `#[websocket_gateway(path = "/chat")]` and its handlers in a `#[handlers]` block. Frames
are JSON `{ "event": "...", "data": ... }`, like the `WsAdapter` adapter on top of the Node.js `ws` package. Guards,
interceptors and pipes run as in HTTP; global enhancers do not apply to gateways.

```rust,ignore
#[websocket_gateway(path = "/chat")]
#[use_guards(AuthGuard)]                          // class-level enhancers, below the attribute
#[use_pipes(ZodValidationPipe::auto())]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,                             // this gateway's server (one per app)
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("message")]
    #[use_guards(RoomMemberGuard)]                // handler-level enhancers
    async fn message(&self, #[message_body] dto: SendMessageDto, #[connected_socket] client: WsClient) -> HttpResult<WsResponse> {
        self.server.to(&dto.room).except(client.id()).emit("message", &dto.text);
        Ok(WsResponse::new("sent", json!({ "text": dto.text })))
    }
}
```

`HttpAdapterHost` and `corvis::ws` give access to the HTTP server's `upgrade` event and to plain RFC 6455 WebSockets,
like `new WebSocketServer({ noServer: true })` of the `ws` package. `examples/tertulia` uses all of it in a chat with
rooms, presence, typing indicators, SQLite history, SSE and a plain WebSocket for moderators.

More: [Gateways](https://corvis.randygil.com/websockets/gateways),
[Plain WebSockets and adapters](https://corvis.randygil.com/websockets/adapter),
[HTTP adapter](https://corvis.randygil.com/faq/http-adapter).

### Static files

```rust,no_run
use corvis::prelude::*;

#[module(imports = [
    // Serves ./public under /static: GET and HEAD, ETag, ranges, dotfiles ignored.
    ServeStaticModule::for_root(vec![ServeStaticOptions::new("public").serve_root("/static")]),
])]
pub struct AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    // The same files server as a middleware, like `app.use('/uploads', express.static('uploads'))`:
    app.use_middleware(corvis::serve_static("uploads").mount("/uploads"));
    app.listen("127.0.0.1:3000").await
}
```

More: [Static files](https://corvis.randygil.com/recipes/serve-static).

### HTTP behavior

Corvis reproduces the HTTP behavior of an Express 5 application, so a ported API answers byte for byte the same:

- **CORS** follows the `cors` package (`app.enable_cors(CorsOptions::new().origins([...]).credentials(true))`):
  [CORS](https://corvis.randygil.com/security/cors).
- **Default headers**: weak `ETag` and `304` responses like the `etag` package, `charset=utf-8`, `X-Powered-By`, and
  `StreamableFile` with `type`, `disposition` and `length`:
  [Streaming responses](https://corvis.randygil.com/http/streaming-responses).
- **Body parser and router**: `express.json()` and `express.urlencoded({ extended: true })` with the same limits
  (100 kb), error messages and status codes, and the Express 5 router (case-insensitive routes, optional trailing
  slash, `decodeURIComponent` on parameters): [Raw body](https://corvis.randygil.com/faq/raw-body),
  [Controllers](https://corvis.randygil.com/controllers).
- **`#[res]` and `#[res(passthrough)]`**: the Express 5 `response` object, with `cookie`, `redirect`, `send`, `json`
  and the rest: [Controllers](https://corvis.randygil.com/controllers).
- **File upload**: `corvis::multer` replicates `multer` 2 + `busboy` 1.6 (interceptors, options, limits, errors,
  `ParseFilePipe`): [File upload](https://corvis.randygil.com/http/file-upload).

## ORM (`corvis-orm`)

`crates/corvis-orm` is an ORM on top of sqlx (PostgreSQL and SQLite) with queries built from JSON arguments
(`where`, `include`, `orderBy`...) or typed helpers, integrated with Corvis DI through `OrmModule` and `OrmService`.
The `corvis-orm` binary generates the models from `schema.prisma` files (compatible format) and applies SQL
migrations.

```rust,ignore
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
        // JSON arguments...
        self.db.user().find_many(json!({
            "where": { "OR": [{ "email": { "contains": q } }, { "name": { "contains": q, "mode": "insensitive" } }] },
            "include": { "posts": { "where": { "published": true } } },
            "orderBy": { "id": "desc" },
            "take": 20,
        })).await
        // ...or typed helpers: self.db.user().find_many(user::email::contains(q)).take(20).exec().await
    }
}

#[module(imports = [OrmModule::for_root_from_config(true)], providers = [UsersService])]
pub struct UsersModule;

app.use_global_filters(OrmExceptionFilter::new()); // P2002 → 409, P2025 → 404
```

`examples/blog-orm` is a complete REST API (users and posts with pagination, search, relations and nested writes)
that combines `corvis-orm` with DTOs validated by `corvis::z`:

```bash
cargo run -p blog-orm        # http://127.0.0.1:3000 (DATABASE_URL=file:./dev.db by default)
```

More: [corvis-orm](https://corvis.randygil.com/data/corvis-orm) and
[`crates/corvis-orm/README.md`](crates/corvis-orm/README.md).

## OpenAPI and Swagger (`corvis-swagger`)

`crates/corvis-swagger` generates OpenAPI 3 documents (`DocumentBuilder`, `SwaggerModule::create_document` and
`setup`, the `#[api_*]` attributes), includes zod DTOs in the document (`cleanup_open_api_doc`) and serves Swagger UI
and the Scalar reference. The document it generates is byte for byte the one of the equivalent Node.js app: same
keys, same order and the same `operationId`s.

```rust,ignore
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

The `#[api_*]` attributes go below `#[controller]` or the route attribute, in the same order as the TypeScript
decorators (the order decides the order of the responses). More:
[OpenAPI introduction](https://corvis.randygil.com/openapi/introduction),
[Decorators](https://corvis.randygil.com/openapi/decorators).

## Performance

The reproducible benchmark lives in `bench/`: two apps with the same endpoints, one on an equivalent Node.js framework
(on Express and on Fastify) and one on Corvis in release mode, measured with [oha](https://github.com/hatoo/oha) on
`127.0.0.1`. Run it again with `bash bench/run.sh`; the detailed results are written to
[`bench/results.md`](bench/results.md).

Results on an i9-14900KF (32 threads) with Windows 11, Node 24 and rustc 1.99, with 100 keep-alive connections and
15 s per scenario, client and server on the same machine:

| Scenario | Node.js + Express | Node.js + Fastify | Corvis (1 worker) | Corvis (all cores) |
|---|---:|---:|---:|---:|
| `GET /` text | 713k req/min | 1.02M req/min | 1.54M req/min | **7.15M req/min** |
| `GET /json` | 680k | 963k | 1.45M | **5.26M** |
| `GET /users/:id` + `ParseIntPipe` | 647k | 879k | 1.48M | **7.59M** |
| `POST /users` + `ValidationPipe` | 446k | 547k | 1.42M | **3.42M** |
| p50 `GET /` | 8.98 ms | 4.46 ms | 4.03 ms | 0.73 ms |
| RSS under load | ~165–205 MB | ~163–344 MB | ~11 MB | ~9 MB |

Every run ended with 0 errors and 0 non-2xx responses.

- **With a single worker** (a per-core comparison against a Node.js process without cluster), Corvis serves 2.1–2.3
  times as many requests as the Node.js app on Express in the GET scenarios and 3.2 times as many in the POST with
  validation.
- **With all cores**, the ratio is 7.7–11.7. That number is mostly limited by the environment: oha and the Windows
  loopback saturate at about the same rate as a bare hyper server.
- **The POST row varied the most between runs**: 57k–100k req/s.
- **Memory**: Corvis uses about 15–40 times less under load.

More: [Corvis compared with Fastify, Express and Hono](https://corvis.randygil.com/faq/corvis-vs-fastify-express-hono#measured-performance) and [HTTP adapters](https://corvis.randygil.com/http/performance).

## Security

Corvis keeps the observable behavior of the Node.js stack it replaces, except where that behavior is a
vulnerability; there it diverges and documents the difference. The main points:

- **Bounded parsers**: JSON bodies are limited to 256 nesting levels (`400` instead of a stack overflow), recursive
  `z` schemas to 128 levels, ORM query arguments to 125 levels; regular expressions run in linear time unless they
  need lookarounds or backreferences, which are capped at 1,000,000 steps.
- **Server limits with Node.js defaults**: 60 s to receive headers, 300 s for the body (`408`), 5 s keep-alive idle
  time, 32 KiB request headers (`431`); configurable with `ServerOptions`.
- **Client IP**: `req.ip()`, `#[ip]` and the `ThrottlerGuard` ignore `X-Forwarded-For` unless `trust proxy` is set,
  with the semantics of `proxy-addr`.
- **File uploads**: multipart bodies are only read by a file interceptor, after the guards, streamed, with multer's
  limits applied while reading and a total `body_limit` (10 MiB by default); disk storage rejects file names that
  leave the destination or use Windows device names and NTFS streams.
- **Static files**: read in chunks, never whole; dotfile rules are checked against the real names on disk (including
  Windows 8.3 short names); NTFS alternate streams are never served; `follow_symlinks(false)` confines files to the
  root.
- **Responses**: SSE fields and log lines cannot be forged with CR/LF or control characters; outgoing cookies are
  validated like `cookie` 0.7; redirects encode `Location` (redirecting to a client-supplied URL is still an open
  redirect: validate it).
- **Authentication and configuration**: `JwtService` refuses to sign or verify without a key, validates `nbf` and
  `exp` like Node.js, and a non-empty `secret` takes priority over the asymmetric keys; `.env` values are never
  expanded unless `expand_variables: true`.
- **WebSockets**: 1 MiB maximum payload, 8 MiB send buffer per client, 32 handlers in flight per connection, a 30 s
  close timeout, `allowed_origins` against cross-site WebSocket hijacking and `verify_client` for the handshake.
- **Global guards** protect controller routes only: not gateways, `upgrade` listeners, static files or the
  Swagger/Scalar routes. Protect those separately.
- **Dependencies**: `cargo deny` and `cargo audit` run with the policies in `deny.toml` and `.cargo/audit.toml`, and
  the workspace libraries declare `#![forbid(unsafe_code)]`.

Full details: [HTTP adapters](https://corvis.randygil.com/http/performance),
[CORS](https://corvis.randygil.com/security/cors),
[Rate limiting](https://corvis.randygil.com/security/rate-limiting),
[Authentication](https://corvis.randygil.com/security/authentication),
[File upload](https://corvis.randygil.com/http/file-upload),
[Static files](https://corvis.randygil.com/recipes/serve-static#security),
[Gateways](https://corvis.randygil.com/websockets/gateways),
[WebSocket guards](https://corvis.randygil.com/websockets/guards),
[Validation](https://corvis.randygil.com/application/validation),
[OpenAPI](https://corvis.randygil.com/openapi/introduction),
[corvis-orm security](https://corvis.randygil.com/data/corvis-orm#security).

## Known limitations

- **REQUEST scope**: not available. For per-request data use `req.insert`/`req.get` (the request extensions) or
  `FromContext`. `DEFAULT` and `TRANSIENT` scopes exist.
- **Microservices and GraphQL**: not implemented.
- **Body parser**: compressed bodies (`Content-Encoding: gzip`, `deflate`, `br`) are rejected with
  `415 unsupported content encoding`, while Express decompresses them. A parser error is answered before the global
  `app.use()` middleware, which in Express runs before the parser.
- **File uploads**: the body is streamed, but each file is kept in memory until its part ends and is handed whole to
  the storage (no streaming to disk), so the body is limited by `body_limit` (10 MiB by default).
- **Pipes work on JSON**: pipes receive and return `serde_json::Value`; the final value is deserialized into the
  parameter's type.
- **Attribute order**: class-level attributes (`#[use_guards]`, `#[set_metadata]`, ...) must go below
  `#[controller]`, because Rust processes attributes in order.
- **Interceptors without RxJS**: `CallHandler::handle()` returns a `Future`, not an `Observable`. You can transform,
  time, cache and apply timeouts, but not compose operators.
- **Express-style middleware**: it receives the `Request` and a `Next`. There is no mutable `res` object; the
  response is changed on the result of `next.run(req)` (handlers do have one: `#[res]`).

## License

Corvis is released under the MIT license (see [`LICENSE`](LICENSE)). The copyright notices and licenses of the
projects whose behavior, API or logic it replicates are in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
