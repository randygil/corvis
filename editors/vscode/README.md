# Corvis para VS Code

Extensión para trabajar con [Corvis](https://github.com/randygil/corvis), el framework web
modular para Rust (módulos, inyección de dependencias, controladores y decoradores). Incluye
snippets de los atributos reales del framework, comandos que ejecutan la CLI `corvis`, un
explorador de módulos, controladores y rutas, y resaltado propio para `#[controller]`, `#[get]`,
`#[injectable]` y el resto de los "decoradores".

## Qué incluye

### Snippets

Todos los prefijos empiezan con `cv-` (también funcionan con `corvis-`, por ejemplo
`corvis-controller`). Cada snippet se expande y se compila contra Corvis antes de publicarse
(ver [Desarrollo](#desarrollo)).

| Prefijo | Genera |
|---|---|
| `cv-main` | `#[corvis::main]` con `CorvisFactory::create`, pipes globales y `listen` |
| `cv-module`, `cv-module-empty`, `cv-module-global` | `#[module(imports, controllers, providers, exports)]`, `#[module]`, `#[module(global, ...)]` |
| `cv-dynamic-module` | Módulo con `for_root(...) -> DynamicModule` |
| `cv-configure` | `impl CorvisModule` que aplica un middleware con `consumer.apply::<M>()` |
| `cv-controller`, `cv-controller-empty` | `#[controller]` + `#[routes]` con CRUD, o con un solo `#[get]` |
| `cv-get`, `cv-get-id`, `cv-post`, `cv-put`, `cv-patch`, `cv-delete` | Handlers con `#[param("id", ParseIntPipe)]` y `#[body]` |
| `cv-get-query`, `cv-route-roles` | `#[query]`; ruta con `#[set_metadata(ROLES_KEY, [...])]` y `#[use_guards]` |
| `cv-body`, `cv-param`, `cv-query`, `cv-header-param`, `cv-body-zod` | Parámetros decorados (`@Body()`, `@Param()`, `@Query()`, `@Headers()`, `ZodValidationPipe`) |
| `cv-service`, `cv-dep`, `cv-inject` | `#[injectable]`, campo `Arc<T>` inyectado, `#[inject("TOKEN")]` |
| `cv-use-value`, `cv-use-factory` | `Provider::use_value(...)`, `Provider::use_factory(...)` |
| `cv-guard`, `cv-roles-guard` | `CanActivate`; guard de roles con `Reflector::get_all_and_override` |
| `cv-interceptor`, `cv-pipe`, `cv-filter`, `cv-middleware` | `CorvisInterceptor`, `PipeTransform`, `ExceptionFilter`, `CorvisMiddleware` |
| `cv-param-decorator` | Parámetro propio con `FromContext` (`createParamDecorator`) |
| `cv-on-module-init` | `OnModuleInit`, `OnApplicationBootstrap` u `OnModuleDestroy` |
| `cv-on-event`, `cv-cron` | `#[handlers]` con `#[on_event]`, `#[cron]` e `#[interval]` |
| `cv-gateway` | `#[websocket_gateway]` con `#[subscribe_message]` |
| `cv-dto` | DTO con `#[derive(Validate)]` (class-validator) |
| `cv-zod-dto`, `cv-zod-schema`, `cv-zod-object`, `cv-zod-query` | `zod_dto!(Dto, z::object! {..})`, función de schema, `z::object!`, query con `z::coerce` |
| `cv-model`, `cv-orm-service` | Modelo `#[derive(Model)]` de corvis-orm y servicio con `Arc<OrmService>` |
| `cv-find-many`, `cv-find-unique`, `cv-create`, `cv-update`, `cv-upsert`, `cv-delete-record`, `cv-count`, `cv-transaction` | Consultas de corvis-orm con argumentos JSON (`where`, `include`, `orderBy`...) |
| `cv-api-controller`, `cv-api-route`, `cv-swagger-setup` | `#[api_tags]`, `#[api_operation]`, `#[api_ok_response]`; `DocumentBuilder` + `SwaggerModule::setup` |
| `cv-test` | `#[corvis::test]` con `Test::create_testing_module()` y `request(&app)` |
| `cv-use` | `use corvis::prelude::*;` |

Los snippets suponen `use corvis::prelude::*;` en el archivo. Los de modelos de corvis-orm usan
`use corvis_orm::prelude::*;` y los de Swagger, `use corvis_swagger::*;`.

### Comandos de la CLI

| Comando | Qué hace |
|---|---|
| **Corvis: Generate…** | Elige el schematic (resource, module, controller, service, guard, interceptor, pipe, filter, middleware, gateway), el nombre y las opciones `--flat` / `--no-spec`, y ejecuta `corvis g` en la raíz del proyecto |
| **Corvis: Generate Resource (CRUD)…**, **Generate Module…**, ... | Lo mismo, con el schematic ya elegido |
| **Corvis: New Project…** | `corvis new <nombre>` en la carpeta que elijas; al terminar ofrece abrirla |
| **Corvis: Start Application** / **(watch)** | `corvis start` / `corvis start --watch` en una tarea de la terminal |
| **Corvis: Build Application (release)** | `corvis build --release` |
| **Corvis: Show Project Info** | `corvis info` en el panel de salida "Corvis" |
| **Corvis: Go to Route…** | Busca una ruta por método, path o handler y abre el archivo en esa línea |

En el explorador de archivos, el clic derecho sobre una carpeta muestra el submenú **Corvis** con
los mismos schematics. Si la carpeta está directamente dentro de `src/` (por ejemplo
`src/users`), los archivos se crean ahí con `--path users`, que es lo que admite la CLI. Al
terminar se abre el archivo principal generado, y los mensajes `CREATE`/`UPDATE` quedan en el panel
de salida **Corvis**.

La extensión busca la CLI en este orden:

1. El ajuste `corvis.cli.path`.
2. `corvis` en el `PATH`, o `~/.cargo/bin/corvis`.
3. `cargo run -q --manifest-path <repo>/crates/corvis-cli/Cargo.toml --`, donde `<repo>` es el
   ajuste `corvis.frameworkPath`, una carpeta del workspace que sea el repositorio de Corvis, o
   la ruta de la dependencia `corvis = { path = "..." }` del `Cargo.toml` del proyecto.

Para instalar la CLI: `cargo install --path crates/corvis-cli` desde el repositorio de Corvis.

### Explorador de la aplicación

La vista **Corvis** de la barra de actividad tiene dos paneles, construidos a partir de los
archivos `.rs` del workspace (sin compilar):

- **Modules**: cada `#[module]` con sus imports (enlazados al módulo importado), controladores con
  sus rutas, providers (con sus `#[cron]`, `#[on_event]` y `#[subscribe_message]`) y exports. El
  módulo que recibe `CorvisFactory::create::<M>()` aparece primero, marcado como `root`.
- **Routes**: los controladores ordenados por path, cada uno con sus rutas `GET /api/v1/cats/:id`.

El path completo tiene en cuenta `app.set_global_prefix("...")` y el versionado por URI
(`enable_versioning(VersioningOptions::uri().default_version("1"))`, `#[controller(version = ..)]`,
`#[version(..)]`). Al hacer clic se abre el archivo en la línea del item, y el ícono de copiar
de una ruta copia su path. Si el workspace tiene varios crates, se agrupan por crate.

### Resaltado

- Una gramática TextMate inyectada en Rust resalta los atributos de Corvis por categoría: DI
  (`#[module]`, `#[controller]`, `#[injectable]`, `#[routes]`, `#[handlers]`...), rutas
  (`#[get]`, `#[post]`...), enhancers (`#[use_guards]`, `#[set_metadata]`, `#[http_code]`...),
  parámetros (`#[body]`, `#[param]`, `#[query]`...), handlers (`#[cron]`, `#[on_event]`...),
  Swagger (`#[api_*]`), class-validator y corvis-orm. También resalta `:id` y `*` dentro de los
  paths de las rutas, las claves `imports =`, `controllers =`, etc., las llamadas a `z::...` y
  `zod_dto!`. Además, los atributos con corchetes anidados (`controllers = [A, B]`) ya no
  cortan el resaltado como pasa con la gramática de Rust que trae VS Code.
- Con rust-analyzer activo, el resaltado semántico reemplaza los colores TextMate de los nombres
  de atributos. Por eso la extensión también colorea esos nombres con decoraciones del editor,
  que se dibujan encima. Los colores se cambian en `workbench.colorCustomizations`:

  ```jsonc
  "workbench.colorCustomizations": {
    "corvis.attribute.di": "#C586C0",
    "corvis.attribute.http": "#4EC9B0",
    "corvis.attribute.enhancer": "#DCDCAA",
    "corvis.attribute.param": "#9CDCFE",
    "corvis.routeHint": "#8B949E"
  }
  ```

- Junto a cada `#[get]`, `#[post]`, etc. aparece el path completo de la ruta (`GET /api/v1/cats/:id`).

Las decoraciones solo se aplican en workspaces cuyo `Cargo.toml` depende de Corvis.

## Instalación

Desde el archivo `.vsix`:

```bash
code --install-extension corvis-vscode-0.1.0.vsix
```

O en VS Code: vista de extensiones → menú `...` → **Install from VSIX...**.

Requiere VS Code 1.90 o superior. Para los comandos de la CLI hace falta Rust (`cargo`) y la
CLI `corvis` o el repositorio de Corvis (ver arriba).

## Ajustes

| Ajuste | Valor por defecto | Descripción |
|---|---|---|
| `corvis.cli.path` | `""` | Ruta al ejecutable `corvis` |
| `corvis.frameworkPath` | `""` | Carpeta del repositorio de Corvis, para usar `cargo run -p corvis-cli` |
| `corvis.generate.flat` | `false` | Preselecciona `--flat` |
| `corvis.generate.spec` | `true` | Si es `false`, preselecciona `--no-spec` |
| `corvis.generate.openGeneratedFile` | `true` | Abre el archivo principal generado |
| `corvis.decorations.enabled` | `true` | Colorea los atributos de Corvis con decoraciones |
| `corvis.routeHints.enabled` | `true` | Muestra el path completo junto a cada ruta |
| `corvis.explorer.exclude` | `**/{target,node_modules,.git}/**` | Archivos que no se indexan |

## Limitaciones

- El explorador lee el código fuente con un escáner propio, no con rust-analyzer. No expande
  macros ni resuelve `use`: enlaza los items por nombre dentro de cada crate. Los paths que no son
  literales (`#[get(PATH)]`) se muestran tal cual.
- `corvis g --path` solo admite carpetas directamente dentro de `src/`. Si eliges una carpeta más
  profunda, la extensión te avisa y genera en `src/<nombre>/`.
- La gramática TextMate no sabe si el crate usa Corvis, así que también colorea `#[get]` en otros
  frameworks como actix o rocket. Las decoraciones sí se limitan a proyectos Corvis.

## Desarrollo

```bash
cd editors/vscode
npm install
npm run compile            # tsc -> out/
npm test                   # pruebas del escáner (node --test)
npm run verify-snippets    # expande cada snippet y ejecuta cargo check en un crate de prueba
node scripts/smoke.mjs <carpeta-temporal>   # VS Code real: activa la extensión y ejecuta corvis g
npm run package            # genera dist/corvis-vscode-<versión>.vsix
```

`scripts/verify-snippets.mjs` escribe por defecto en `../corvis-tests/tmp/vscode` (fuera del
workspace de Cargo de Corvis); puedes cambiarlo con `--crate <carpeta>`. Cada opción de los
snippets con alternativas (`${1|get,post|}`) se compila por separado. La gramática se genera con
`node scripts/build-grammar.mjs` y el ícono con `python scripts/make-icon.py` (Pillow).

Para probar la extensión sin empaquetarla, abre `editors/vscode` en VS Code y presiona `F5`
(**Run Extension**).

## Licencia

MIT
