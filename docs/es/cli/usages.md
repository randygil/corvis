---
status: documented
---

# Uso

Referencia de los comandos de `corvis`. Las opciones empiezan con `--` y pueden ir en cualquier posición después del comando.

## `corvis new`

```bash
corvis new <nombre> [--corvis-path <ruta>] [--corvis-version <requisito>] [--skip-git]
```

Crea la carpeta del proyecto (el nombre en kebab-case) con `Cargo.toml`, `.env`, `.gitignore` y `src/` con `main.rs`, `app_module.rs`, `app_controller.rs` y `app_service.rs`. Falla si la carpeta ya existe.

| Opción | Efecto |
|---|---|
| `--corvis-path <ruta>` | Dependencia `corvis = { path = "<ruta>" }`. Sin ella se usa `CORVIS_PATH` o la copia desde la que se compiló la CLI |
| `--corvis-version <requisito>` | Dependencia `corvis = { version = "<requisito>" }` (un requisito de versión de Cargo) |
| `--skip-git` | No ejecuta `git init` |

El nombre debe empezar con una letra y contener solo letras, dígitos, `-` y `_`. `--corvis-version` debe ser un requisito de versión de Cargo (por ejemplo `0.1`, `^0.1.2` o `>=0.1, <0.3`) y `--corvis-path` no puede contener comillas ni caracteres de control. Así nada se escribe fuera de `src/` ni se inyecta texto en los archivos generados. `corvis generate` aplica la misma regla de nombres.

## `corvis generate`

```bash
corvis generate <schematic> <nombre> [--flat] [--path <módulo>] [--no-spec]
corvis g <schematic> <nombre> ...
```

Se ejecuta dentro de un proyecto: busca hacia arriba la primera carpeta con `Cargo.toml` y `src/`.

### Schematics

| Schematic | Alias | Genera | Lo registra en |
|---|---|---|---|
| `module` | `mo` | `XModule` | `imports` de `AppModule` |
| `controller` | `co` | `XController` con un `GET` y un test | `controllers` del módulo de la carpeta, o de `AppModule` |
| `service` | `s` | `XService` (`#[injectable]`) con un test | `providers` del módulo de la carpeta, o de `AppModule` |
| `resource` | `res` | Módulo, controlador CRUD, servicio, DTOs, entidad y test | `imports` de `AppModule` |
| `guard` | `gu` | `XGuard` (`CanActivate`) | No se registra |
| `interceptor` | `itc`, `in` | `XInterceptor` (`CorvisInterceptor`) | No se registra |
| `pipe` | `pi` | `XPipe` (`PipeTransform`) | No se registra |
| `filter` | `f` | `XFilter` (`ExceptionFilter`) | No se registra |
| `middleware` | `mi` | `XMiddleware` (`CorvisMiddleware`) | No se registra |
| `gateway` | `ga` | `XGateway` (`#[websocket_gateway]`) | `providers` del módulo de la carpeta, o de `AppModule` |

Guards, interceptors, pipes, filtros y middleware no se registran porque se usan con `#[use_guards(...)]`, `#[use_interceptors(...)]`, `consumer.apply::<...>()` o los métodos `use_global_*`, según el caso.

### Dónde se escriben los archivos

Con `corvis g service users`:

```text
src/
├── main.rs          # + mod users;
├── app_module.rs    # + providers = [UsersService] (si no existe src/users/users_module.rs)
└── users/
    ├── mod.rs       # + pub mod users_service; pub use users_service::UsersService;
    └── users_service.rs
```

Si `src/users/users_module.rs` existe (por ejemplo, porque antes ejecutaste `corvis g module users`), el servicio se registra en `UsersModule` en lugar de `AppModule`. La CLI agrega también las líneas `use` necesarias.

La regla exacta es: se registra en el primer archivo que exista entre `src/<carpeta>/<nombre>_module.rs`, `src/<carpeta>/<carpeta>_module.rs` y `src/app_module.rs` (con `--flat`, siempre `src/app_module.rs`). Así, `corvis g controller profile --path users` registra `ProfileController` en `UsersModule` cuando existe `src/users/users_module.rs`, y la CLI imprime la línea `UPDATE` con el archivo modificado. Si recurre a `AppModule` mientras la carpeta tiene otros archivos `*_module.rs`, imprime un `NOTE` con sus nombres para que muevas el registro si no era lo que querías.

| Opción | Efecto |
|---|---|
| `--flat` | Escribe el archivo en `src/` (`src/users_service.rs`) en lugar de una carpeta |
| `--path <módulo>` | Usa otra carpeta: `corvis g controller profile --path users` escribe en `src/users/`. Debe ser un nombre de módulo de Rust válido |
| `--no-spec` | Omite el módulo `#[cfg(test)]` del archivo generado |

Si un archivo ya existe, el comando falla sin sobrescribirlo. Cuando la CLI no reconoce la forma de un archivo que debe modificar, lo indica con `SKIP` y tienes que hacer el registro a mano.

Los nombres de los tipos se derivan del argumento: `user-profile`, `user_profile` y `UserProfile` producen `UserProfileService` en `user_profile_service.rs`, con la ruta `user-profile` en los controladores.

## `corvis start`

```bash
corvis start [--watch] [--release] [-- <argumentos del programa>]
```

| Forma | Ejecuta |
|---|---|
| `corvis start` | `cargo run` |
| `corvis start --release` (o `--prod`) | `cargo run --release` |
| `corvis start -- --mi-flag` | `cargo run -- --mi-flag` |
| `corvis start --watch` | `cargo watch -x run` |

Sin `--watch`, el comando imprime antes el perfil que ejecuta (`Running the debug profile ...` o `Running the release profile`).

`--watch` necesita [`cargo-watch`](https://crates.io/crates/cargo-watch) (`cargo install cargo-watch`). Antes de ejecutarlo, la CLI busca `cargo-watch` en los directorios de `PATH` y en `$CARGO_HOME/bin` (por defecto `~/.cargo/bin`); si no está, el comando falla con `cargo-watch is not installed` e indica ejecutar `cargo install cargo-watch` o usar `corvis start` sin `--watch`. En modo watch se ignoran `--release` y los argumentos tras `--`.

## `corvis build`

```bash
corvis build [--release]
```

Ejecuta `cargo build` (perfil debug), o `cargo build --release` con `--release` (o `--prod`). Antes imprime el perfil: `Building the debug profile ...` o `Building the release profile`.

## `corvis info`

Muestra el sistema operativo y la arquitectura, `rustc --version`, `cargo --version` y la versión de la CLI. Útil para adjuntar a un reporte de errores.
