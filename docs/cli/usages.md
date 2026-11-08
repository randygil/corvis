---
status: documented
---

# Usage

Reference for the `corvis` commands. Options start with `--` and can go in any position after the command.

## `corvis new`

```bash
corvis new <name> [--corvis-path <path>] [--corvis-version <requirement>] [--skip-git]
```

Creates the project folder (the name in kebab-case) with `Cargo.toml`, `.env`, `.gitignore` and `src/` with `main.rs`, `app_module.rs`, `app_controller.rs` and `app_service.rs`. It fails if the folder already exists.

| Option | Effect |
|---|---|
| `--corvis-path <path>` | Dependency `corvis = { path = "<path>" }`. Without it, `CORVIS_PATH` or the copy the CLI was built from is used |
| `--corvis-version <requirement>` | Dependency `corvis = { version = "<requirement>" }` (a Cargo version requirement) |
| `--skip-git` | Does not run `git init` |

The name must start with a letter and contain only letters, digits, `-` and `_`. `--corvis-version` must be a Cargo version requirement (for example `0.1`, `^0.1.2` or `>=0.1, <0.3`) and `--corvis-path` cannot contain quotes or control characters. This way nothing is written outside `src/` and no text is injected into the generated files. `corvis generate` applies the same name rule.

## `corvis generate`

```bash
corvis generate <schematic> <name> [--flat] [--path <module>] [--no-spec]
corvis g <schematic> <name> ...
```

It runs inside a project: it searches upward for the first folder with `Cargo.toml` and `src/`.

### Schematics

| Schematic | Alias | Generates | Registers it in |
|---|---|---|---|
| `module` | `mo` | `XModule` | `imports` of `AppModule` |
| `controller` | `co` | `XController` with a `GET` and a test | `controllers` of the folder's module, or of `AppModule` |
| `service` | `s` | `XService` (`#[injectable]`) with a test | `providers` of the folder's module, or of `AppModule` |
| `resource` | `res` | Module, CRUD controller, service, DTOs, entity and test | `imports` of `AppModule` |
| `guard` | `gu` | `XGuard` (`CanActivate`) | Not registered |
| `interceptor` | `itc`, `in` | `XInterceptor` (`CorvisInterceptor`) | Not registered |
| `pipe` | `pi` | `XPipe` (`PipeTransform`) | Not registered |
| `filter` | `f` | `XFilter` (`ExceptionFilter`) | Not registered |
| `middleware` | `mi` | `XMiddleware` (`CorvisMiddleware`) | Not registered |
| `gateway` | `ga` | `XGateway` (`#[websocket_gateway]`) | `providers` of the folder's module, or of `AppModule` |

Guards, interceptors, pipes, filters and middleware are not registered because they are used with `#[use_guards(...)]`, `#[use_interceptors(...)]`, `consumer.apply::<...>()` or the `use_global_*` methods, depending on the case.

### Where the files are written

With `corvis g service users`:

```text
src/
├── main.rs          # + mod users;
├── app_module.rs    # + providers = [UsersService] (if src/users/users_module.rs does not exist)
└── users/
    ├── mod.rs       # + pub mod users_service; pub use users_service::UsersService;
    └── users_service.rs
```

If `src/users/users_module.rs` exists (for example, because you ran `corvis g module users` earlier), the service is registered in `UsersModule` instead of `AppModule`. The CLI also adds the necessary `use` lines.

The exact rule is: it registers in the first file that exists of `src/<folder>/<name>_module.rs`, `src/<folder>/<folder>_module.rs` and `src/app_module.rs` (with `--flat`, always `src/app_module.rs`). So `corvis g controller profile --path users` registers `ProfileController` in `UsersModule` when `src/users/users_module.rs` exists, and the CLI prints the `UPDATE` line with the file it changed. If it falls back to `AppModule` while the folder has other `*_module.rs` files, it prints a `NOTE` naming them so you can move the registration if that was not intended.

| Option | Effect |
|---|---|
| `--flat` | Writes the file in `src/` (`src/users_service.rs`) instead of a folder |
| `--path <module>` | Uses another folder: `corvis g controller profile --path users` writes to `src/users/`. It must be a valid Rust module name |
| `--no-spec` | Omits the `#[cfg(test)]` module from the generated file |

If a file already exists, the command fails without overwriting it. When the CLI does not recognize the shape of a file it has to modify, it reports `SKIP` and you have to do the registration by hand.

Type names are derived from the argument: `user-profile`, `user_profile` and `UserProfile` produce `UserProfileService` in `user_profile_service.rs`, with the route `user-profile` in controllers.

## `corvis start`

```bash
corvis start [--watch] [--release] [-- <program arguments>]
```

| Form | Runs |
|---|---|
| `corvis start` | `cargo run` |
| `corvis start --release` (or `--prod`) | `cargo run --release` |
| `corvis start -- --my-flag` | `cargo run -- --my-flag` |
| `corvis start --watch` | `cargo watch -x run` |

Without `--watch`, the command first prints the profile it runs (`Running the debug profile ...` or `Running the release profile`).

`--watch` needs [`cargo-watch`](https://crates.io/crates/cargo-watch) (`cargo install cargo-watch`). Before running it, the CLI looks for `cargo-watch` in the directories of `PATH` and in `$CARGO_HOME/bin` (default `~/.cargo/bin`); if it is not there, the command fails with `cargo-watch is not installed` and tells you to run `cargo install cargo-watch` or to use `corvis start` without `--watch`. In watch mode, `--release` and the arguments after `--` are ignored.

## `corvis build`

```bash
corvis build [--release]
```

Runs `cargo build` (debug profile), or `cargo build --release` with `--release` (or `--prod`). It prints the profile first: `Building the debug profile ...` or `Building the release profile`.

## `corvis info`

Shows the operating system and architecture, `rustc --version`, `cargo --version` and the CLI version. Useful to attach to a bug report.
