# Corvis for VS Code

Extension for working with [Corvis](https://github.com/randygil/corvis), the modular web
framework for Rust (modules, dependency injection, controllers and decorators). It includes
snippets for the framework's real attributes, commands that run the `corvis` CLI, an
explorer for modules, controllers and routes, and its own highlighting for `#[controller]`, `#[get]`,
`#[injectable]` and the rest of the "decorators".

## What is included

### Snippets

All prefixes start with `cv-` (they also work with `corvis-`, for example
`corvis-controller`). Every snippet is expanded and compiled against Corvis before being published
(see [Development](#development)).

| Prefix | Generates |
|---|---|
| `cv-main` | `#[corvis::main]` with `CorvisFactory::create`, global pipes and `listen` |
| `cv-module`, `cv-module-empty`, `cv-module-global` | `#[module(imports, controllers, providers, exports)]`, `#[module]`, `#[module(global, ...)]` |
| `cv-dynamic-module` | Module with `for_root(...) -> DynamicModule` |
| `cv-configure` | `impl CorvisModule` that applies a middleware with `consumer.apply::<M>()` |
| `cv-controller`, `cv-controller-empty` | `#[controller]` + `#[routes]` with CRUD, or with a single `#[get]` |
| `cv-get`, `cv-get-id`, `cv-post`, `cv-put`, `cv-patch`, `cv-delete` | Handlers with `#[param("id", ParseIntPipe)]` and `#[body]` |
| `cv-get-query`, `cv-route-roles` | `#[query]`; route with `#[set_metadata(ROLES_KEY, [...])]` and `#[use_guards]` |
| `cv-body`, `cv-param`, `cv-query`, `cv-header-param`, `cv-body-zod` | Decorated parameters (`@Body()`, `@Param()`, `@Query()`, `@Headers()`, `ZodValidationPipe`) |
| `cv-service`, `cv-dep`, `cv-inject` | `#[injectable]`, injected `Arc<T>` field, `#[inject("TOKEN")]` |
| `cv-use-value`, `cv-use-factory` | `Provider::use_value(...)`, `Provider::use_factory(...)` |
| `cv-guard`, `cv-roles-guard` | `CanActivate`; roles guard with `Reflector::get_all_and_override` |
| `cv-interceptor`, `cv-pipe`, `cv-filter`, `cv-middleware` | `CorvisInterceptor`, `PipeTransform`, `ExceptionFilter`, `CorvisMiddleware` |
| `cv-param-decorator` | Custom parameter with `FromContext` (`createParamDecorator`) |
| `cv-on-module-init` | `OnModuleInit`, `OnApplicationBootstrap` or `OnModuleDestroy` |
| `cv-on-event`, `cv-cron` | `#[handlers]` with `#[on_event]`, `#[cron]` and `#[interval]` |
| `cv-gateway` | `#[websocket_gateway]` with `#[subscribe_message]` |
| `cv-dto` | DTO with `#[derive(Validate)]` (class-validator) |
| `cv-zod-dto`, `cv-zod-schema`, `cv-zod-object`, `cv-zod-query` | `zod_dto!(Dto, z::object! {..})`, schema function, `z::object!`, query with `z::coerce` |
| `cv-model`, `cv-orm-service` | corvis-orm `#[derive(Model)]` model and service with `Arc<OrmService>` |
| `cv-find-many`, `cv-find-unique`, `cv-create`, `cv-update`, `cv-upsert`, `cv-delete-record`, `cv-count`, `cv-transaction` | corvis-orm queries with JSON arguments (`where`, `include`, `orderBy`...) |
| `cv-api-controller`, `cv-api-route`, `cv-swagger-setup` | `#[api_tags]`, `#[api_operation]`, `#[api_ok_response]`; `DocumentBuilder` + `SwaggerModule::setup` |
| `cv-test` | `#[corvis::test]` with `Test::create_testing_module()` and `request(&app)` |
| `cv-use` | `use corvis::prelude::*;` |

The snippets assume `use corvis::prelude::*;` in the file. The corvis-orm model snippets use
`use corvis_orm::prelude::*;` and the Swagger ones use `use corvis_swagger::*;`.

### CLI commands

| Command | What it does |
|---|---|
| **Corvis: Generate…** | Picks the schematic (resource, module, controller, service, guard, interceptor, pipe, filter, middleware, gateway), the name and the `--flat` / `--no-spec` options, and runs `corvis g` at the project root |
| **Corvis: Generate Resource (CRUD)…**, **Generate Module…**, ... | The same, with the schematic already chosen |
| **Corvis: New Project…** | `corvis new <name>` in the folder you pick; when it finishes it offers to open it. With `corvis.new.corvisPath` set, it passes that folder as `CORVIS_PATH` (see below) |
| **Corvis: Start Application** / **(watch)** | `corvis start` / `corvis start --watch` in a terminal task |
| **Corvis: Build Application (release)** | `corvis build --release` |
| **Corvis: Show Project Info** | `corvis info` in the "Corvis" output panel |
| **Corvis: Go to Route…** | Searches a route by method, path or handler and opens the file at that line |

In the file explorer, right-clicking a folder shows the **Corvis** submenu with the same
schematics. If the folder is directly inside `src/` (for example
`src/users`), the files are created there with `--path users`, which is what the CLI supports. When
it finishes, the main generated file is opened, and the `CREATE`/`UPDATE` messages are written to the
**Corvis** output panel.

The extension looks for the CLI in this order:

1. The `corvis.cli.path` setting.
2. `corvis` in the `PATH`, or `~/.cargo/bin/corvis`.
3. `cargo run -q --manifest-path <repo>/crates/corvis-cli/Cargo.toml --`, where `<repo>` is the
   `corvis.frameworkPath` setting, a workspace folder that is the Corvis repository, or
   the path of the `corvis = { path = "..." }` dependency in the project's `Cargo.toml`.

To install the CLI: `cargo install --path crates/corvis-cli` from the Corvis repository.

**New Project** and the framework dependency. `corvis new` writes `corvis = { version = "<CLI version>" }` unless
`CORVIS_PATH` points at a local checkout. If the `corvis.new.corvisPath` setting is not empty, **New Project** runs
the CLI with `CORVIS_PATH` set to it (only for that command; an empty setting leaves the environment untouched). Until
Corvis is published on crates.io, fill it in so the new project can resolve `corvis`. The value is the `crates/corvis`
folder of your checkout (the repository root is also accepted), absolute, with `~` expanded, and it must contain a
`Cargo.toml`; otherwise the extension shows an error and does not run the CLI.

### Application explorer

The **Corvis** view in the activity bar has two panels, built from the workspace's
`.rs` files (without compiling):

- **Modules**: each `#[module]` with its imports (linked to the imported module), controllers with
  their routes, providers (with their `#[cron]`, `#[on_event]` and `#[subscribe_message]`) and exports. The
  module that receives `CorvisFactory::create::<M>()` appears first, marked as `root`.
- **Routes**: the controllers sorted by path, each with its `GET /api/v1/cats/:id` routes.

The full path takes into account `app.set_global_prefix("...")` and URI versioning
(`enable_versioning(VersioningOptions::uri().default_version("1"))`, `#[controller(version = ..)]`,
`#[version(..)]`). Clicking opens the file at the item's line, and the copy icon
of a route copies its path. If the workspace has several crates, they are grouped by crate.

### Highlighting

- A TextMate grammar injected into Rust highlights Corvis attributes by category: DI
  (`#[module]`, `#[controller]`, `#[injectable]`, `#[routes]`, `#[handlers]`...), routes
  (`#[get]`, `#[post]`...), enhancers (`#[use_guards]`, `#[set_metadata]`, `#[http_code]`...),
  parameters (`#[body]`, `#[param]`, `#[query]`...), handlers (`#[cron]`, `#[on_event]`...),
  Swagger (`#[api_*]`), class-validator and corvis-orm. It also highlights `:id` and `*` inside route
  paths, the `imports =`, `controllers =`, etc. keys, `z::...` calls and
  `zod_dto!`. In addition, attributes with nested brackets (`controllers = [A, B]`) no longer
  break the highlighting as they do with the Rust grammar that ships with VS Code.
- With rust-analyzer active, semantic highlighting replaces the TextMate colors of attribute
  names. That is why the extension also colors those names with editor decorations,
  which are drawn on top. The colors are changed in `workbench.colorCustomizations`:

  ```jsonc
  "workbench.colorCustomizations": {
    "corvis.attribute.di": "#C586C0",
    "corvis.attribute.http": "#4EC9B0",
    "corvis.attribute.enhancer": "#DCDCAA",
    "corvis.attribute.param": "#9CDCFE",
    "corvis.routeHint": "#8B949E"
  }
  ```

- Next to each `#[get]`, `#[post]`, etc. the full path of the route appears (`GET /api/v1/cats/:id`).

Decorations are only applied in workspaces whose `Cargo.toml` depends on Corvis.

## Installation

From the `.vsix` file:

```bash
code --install-extension corvis-vscode-0.1.0.vsix
```

Or in VS Code: extensions view → `...` menu → **Install from VSIX...**.

Requires VS Code 1.90 or later. The CLI commands need Rust (`cargo`) and the
`corvis` CLI or the Corvis repository (see above).

## Settings

| Setting | Default | Description |
|---|---|---|
| `corvis.cli.path` | `""` | Path to the `corvis` executable |
| `corvis.frameworkPath` | `""` | Folder of the Corvis repository, to use `cargo run -p corvis-cli` |
| `corvis.new.corvisPath` | `""` | Passed as `CORVIS_PATH` to `corvis new` (**New Project**): the `crates/corvis` folder of a local checkout, or the repository root |
| `corvis.generate.flat` | `false` | Preselects `--flat` |
| `corvis.generate.spec` | `true` | When `false`, preselects `--no-spec` |
| `corvis.generate.openGeneratedFile` | `true` | Opens the main generated file |
| `corvis.decorations.enabled` | `true` | Colors Corvis attributes with decorations |
| `corvis.routeHints.enabled` | `true` | Shows the full path next to each route |
| `corvis.explorer.exclude` | `**/{target,node_modules,.git}/**` | Files that are not indexed |

## Limitations

- The explorer reads the source code with its own scanner, not with rust-analyzer. It does not expand
  macros or resolve `use`: it links items by name inside each crate. Paths that are not
  literals (`#[get(PATH)]`) are shown as they are.
- `corvis g --path` only supports folders directly inside `src/`. If you pick a deeper
  folder, the extension warns you and generates in `src/<name>/`.
- The TextMate grammar does not know whether the crate uses Corvis, so it also colors `#[get]` in other
  frameworks such as actix or rocket. The decorations are limited to Corvis projects.

## Development

```bash
cd editors/vscode
npm install
npm run compile            # tsc -> out/
npm test                   # scanner tests (node --test)
npm run verify-snippets    # expands every snippet and runs cargo check in a test crate
node scripts/smoke.mjs <temp-folder>   # real VS Code: activates the extension and runs corvis g
npm run package            # produces dist/corvis-vscode-<version>.vsix
```

`scripts/verify-snippets.mjs` writes by default to `../corvis-tests/tmp/vscode` (outside the
Corvis Cargo workspace); you can change it with `--crate <folder>`. Each snippet option
with alternatives (`${1|get,post|}`) is compiled separately. The grammar is generated with
`node scripts/build-grammar.mjs` and the icon with `python scripts/make-icon.py` (Pillow).

To try the extension without packaging it, open `editors/vscode` in VS Code and press `F5`
(**Run Extension**).

## License

MIT
