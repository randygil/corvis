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
| `corvis start --watch --release` | `cargo watch -x "run --release"` |
| `corvis start --watch -- --port 4000` | `cargo watch -x "run -- --port 4000"` |

Las opciones de `corvis` (`--watch`, `--release`, `--prod`) se leen solo antes del primer `--`. Todo lo que va después de `--` llega a tu programa sin cambios, así que `corvis start -- --release` ejecuta `cargo run -- --release` (una compilación debug que recibe `--release`).
Sin `--watch`, el comando imprime antes el perfil que ejecuta (`Running the debug profile ...` o `Running the release profile`).

`--watch` necesita [`cargo-watch`](https://crates.io/crates/cargo-watch) (`cargo install cargo-watch`). Antes de ejecutarlo, la CLI busca `cargo-watch` en los directorios de `PATH` y en `$CARGO_HOME/bin` (por defecto `~/.cargo/bin`); si no está, el comando falla con `cargo-watch is not installed` e indica ejecutar `cargo install cargo-watch` o usar `corvis start` sin `--watch`. En modo watch, `--release` y los argumentos tras `--` pasan al comando vigilado. Como `cargo watch` ejecuta ese comando a través de un shell, el modo watch solo acepta argumentos formados por letras, dígitos y `- _ . / : = , + @`; cualquier otro argumento (por ejemplo uno con un espacio) hace que el comando falle antes de iniciar Cargo, y puedes usar `corvis start` sin `--watch`.

## `corvis build`

```bash
corvis build [--release]
```

Ejecuta `cargo build` (perfil debug), o `cargo build --release` con `--release` (o `--prod`). Antes imprime el perfil: `Building the debug profile ...` o `Building the release profile`.

## `corvis posture`

```bash
corvis posture [--json] [--release] [--timeout <segundos>] [-- <argumentos del programa>]
```

Imprime el informe de postura de rutas de la aplicación del proyecto actual: cada ruta con sus guards, o `public`, o la exención y su motivo, y `UNPROTECTED` para una ruta que no tiene nada de eso. Funciona con cualquier modo de postura de la aplicación (off, warn o enforce) y sirve para la revisión de código y la CI.

Ejecuta `cargo build` (con `--release` si se indica) y luego `cargo run` con `CORVIS_ROUTE_POSTURE_REPORT` en `text` (`json` con `--json`). La aplicación calcula el informe al inicializarse, después de compilar las rutas y antes de que corra cualquier hook de inicialización o se abra un socket, lo imprime y termina. La aplicación nunca queda escuchando. Necesita el mismo entorno que `corvis start` (por ejemplo una base de datos accesible si un provider se conecta al crear la aplicación), porque los providers se crean antes de calcular el informe. Los listeners `upgrade` en bruto registrados en hooks no se cuentan.

| Salida | Dónde |
|---|---|
| El informe (tabla, o JSON con `--json`) | stdout, y nada más |
| La salida de Cargo y las líneas de log de la aplicación | stderr |

La aplicación escribe el informe entre las líneas `----- BEGIN CORVIS ROUTE POSTURE REPORT -----` y `----- END CORVIS ROUTE POSTURE REPORT -----` porque sus logs comparten stdout; el comando quita esas líneas, así que `corvis posture --json | jq .` funciona.

| Código de salida | Significado |
|---|---|
| `0` | Se imprimió el informe y nada está sin protección |
| `1` | Se imprimió el informe y al menos una ruta está sin protección |
| `2` | No hay informe: falló la compilación, la aplicación no pudo arrancar (el mensaje muestra su código de salida), venció el tiempo límite o los argumentos eran inválidos |

`--timeout <segundos>` (120 por defecto) es el tiempo que tiene la aplicación, una vez en ejecución, para imprimir el informe. Si nunca llega a la inicialización (por ejemplo se bloquea antes de llamar a `listen`), el comando la detiene y sale con 2. La compilación no cuenta para el tiempo límite. Las opciones de `corvis` se leen solo antes del primer `--`; todo lo que va después llega a tu programa. `--watch` no está soportado.

Los proyectos nuevos creados con `corvis new` llaman a `require_route_posture()` en `main` y marcan su ruta `GET /` con `#[public]`, así que `corvis posture` sale con 0 para ellos.

## `corvis info`

Muestra el sistema operativo y la arquitectura, `rustc --version`, `cargo --version` y la versión de la CLI. Útil para adjuntar a un reporte de errores.
