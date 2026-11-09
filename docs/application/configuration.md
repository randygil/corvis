---
status: documented
---

# Configuration

An application usually needs values that change between environments: the database URL, the port, signing secrets. Corvis centralizes them in `ConfigModule`, which reads the process environment and `.env` files, and exposes them through the `ConfigService` provider.

## Registering the module

`ConfigModule::for_root` returns a [dynamic module](/fundamentals/dynamic-modules) that registers and exports `ConfigService`. The usual approach is to import it once in the root module and mark it as global so you do not repeat the import in every module:

```rust
use corvis::prelude::*;

#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
    ],
)]
pub struct AppModule;
```

From then on any provider can request it as an `Arc<ConfigService>` field. This is how the authentication service in `examples/tertulia` uses it:

```rust
#[injectable]
pub struct AuthService {
    jwt: Arc<JwtService>,
    config: Arc<ConfigService>,
}

impl AuthService {
    pub fn login(&self, username: &str) -> HttpResult<Value> {
        let moderators: String = self.config.get_or("MODERATORS", "admin".to_string());
        // ...
    }
}
```

And outside injection, in `main`, you get it from the container with `app.get`:

```rust
let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
```

## Reading values

| Method | Behavior |
|---|---|
| `get::<T>(key)` | `Option<T>`; `None` if the key does not exist or cannot be converted to `T` |
| `get_or(key, default)` | The value or `default` |
| `try_get::<T>(key)` | `corvis::Result<T>`; error `Configuration key "X" does not exist` if missing |
| `get_or_throw::<T>(key)` | The value; `panic`s if missing |
| `set(key, value)` | Changes or adds a value at runtime |

Environment values are text, but `get` converts them to the requested type: `get::<u16>("PORT")` interprets `"3000"` as a number and `get::<bool>("LOG_QUERIES")` accepts `"true"`. For required secrets, prefer `try_get` or `get` with an explicit error, as Tertulia's `AppModule` does when configuring the JWT:

```rust
let secret = config.get::<String>("JWT_SECRET").filter(|s| !s.trim().is_empty());
let secret = secret.ok_or_else(|| corvis::anyhow::anyhow!("JWT_SECRET is required"))?;
```

## Options

`ConfigModuleOptions` has these fields (all with a default value, so you combine them with `..Default::default()`):

| Field | Default | Use |
|---|---|---|
| `is_global` | `false` | Registers the module as global |
| `env_file_path` | `vec![".env"]` | `.env` files to read; if a key is repeated across files, the first one wins |
| `ignore_env_file` | `false` | Do not read any `.env` |
| `ignore_env_vars` | `false` | Do not read the process environment |
| `load` | empty | Custom configuration factories (see below) |
| `validate` | `None` | Function that validates the variables at startup |
| `expand_variables` | `false` | Expands `$VAR`, `${VAR}` and `${VAR:-default}` in the values of `.env` files |

The order of precedence is: process variables, then the `.env` files (in the order of `env_file_path`) and, for keys with dots, the `load` factories. A file that does not exist is ignored without error.

### Validating at startup

`validate` receives all the variables already merged. If it returns `Err`, creating `ConfigService` fails and the application does not start:

```rust
fn validate_env(env: &std::collections::HashMap<String, String>) -> Result<(), String> {
    if !env.contains_key("DATABASE_URL") {
        return Err("DATABASE_URL is required".into());
    }
    Ok(())
}

ConfigModule::for_root(ConfigModuleOptions {
    is_global: true,
    validate: Some(validate_env),
    ..Default::default()
})
```

The message you will see at startup begins with `Config validation error:`.

## Async registration

When the options themselves depend on another provider, for example the path of the `.env` file, use `ConfigModule::for_root_async(is_global, factory)`. The factory returns `ConfigModuleOptions`, and `ConfigService` is built from them exactly as `for_root` does. `is_global` is the argument; the `is_global` field of the returned options is ignored.

```rust
ConfigModule::for_root_async(true, |ctx| async move {
    let paths = ctx.get::<EnvPaths>().await?; // from a global module
    Ok(ConfigModuleOptions { env_file_path: paths.files.clone(), ..Default::default() })
})
```

The factory cannot resolve `ConfigService`, since it is the provider being built: that fails at startup with a circular dependency error. See [Dynamic modules](/fundamentals/dynamic-modules#convention-for-async-variants) for the convention shared by all async variants.

## Format of `.env` files

Files are read with the grammar of Node.js's `dotenv` (`corvis::config::parse_dotenv` exposes it in case you need it):

- `KEY=value`, `KEY: value` and `export KEY=value`.
- An unquoted `#` starts a comment and ends the value.
- Single, double or backtick quotes; quoted values can span several lines and, inside double quotes, `\n` is a line break.
- Within a single file, if a key is repeated the last one wins.

Values are taken **as is**: `JWT_SECRET=pa$$w0rd` is exactly `pa$$w0rd`. Variable expansion only happens with `expand_variables: true` and only on values that come from files; process environment values are never expanded.

## Custom configuration with `load`

Besides flat variables, you can load JSON objects and read them with dotted paths. `corvis::config::register_as` creates a namespaced factory:

```rust
use corvis::config::register_as;

fn database() -> corvis::config::ConfigFactory {
    register_as("database", || json!({ "host": "localhost", "port": 5432 }))
}

ConfigModule::for_root(ConfigModuleOptions {
    is_global: true,
    load: vec![database()],
    ..Default::default()
})

// In a provider:
let host: String = self.config.get_or_throw("database.host");
let port: u16 = self.config.get_or("database.port", 5432);
```

A closure that returns an object also becomes a factory (without a namespace: its keys stay at the root): `load: vec![(|| json!({ "featureX": true })).into()]`.

Environment variables take priority: if a variable literally named `database.host` exists, `get("database.host")` returns that one.

### Per-module configuration

`ConfigModule::for_feature(factory)` adds a namespace to the existing `ConfigService` from a feature module, and also registers its value under the token `"CONFIGURATION(<name>)"`:

```rust
#[module(imports = [ConfigModule::for_feature(register_as("payments", || json!({ "currency": "USD" })))])]
pub struct PaymentsModule;

#[injectable]
pub struct PaymentsService {
    #[inject("CONFIGURATION(payments)")]
    payments: Arc<Value>,
}
```

`for_feature` resolves `ConfigService` from the module context, so `ConfigModule::for_root` must be registered with `is_global: true`.

## In tests

`ConfigService::from_map` creates a service with fixed values, useful for replacing the real one with `override_provider` in a test module (see [Testing](/fundamentals/testing)):

```rust
let values = std::collections::HashMap::from([("PORT".to_string(), "0".to_string())]);
let config = ConfigService::from_map(values);
```
