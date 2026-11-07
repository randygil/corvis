//! Configuration: `ConfigModule.forRoot()`, `ConfigService`, `registerAs()`.
//!
//! ```ignore
//! #[module(imports = [ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() })])]
//! pub struct AppModule;
//!
//! // anywhere:
//! let port: u16 = config.get_or("PORT", 3000);
//! let host: String = config.get_or_throw("database.host");
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, Token};
use crate::logger::Logger;

/// A namespaced configuration factory (`registerAs('database', () => ({...}))`).
#[derive(Clone)]
pub struct ConfigFactory {
    pub namespace: Option<String>,
    pub factory: Arc<dyn Fn() -> Value + Send + Sync>,
}

/// `registerAs(namespace, factory)`.
pub fn register_as(namespace: impl Into<String>, factory: impl Fn() -> Value + Send + Sync + 'static) -> ConfigFactory {
    ConfigFactory { namespace: Some(namespace.into()), factory: Arc::new(factory) }
}

impl<F: Fn() -> Value + Send + Sync + 'static> From<F> for ConfigFactory {
    fn from(f: F) -> Self {
        ConfigFactory { namespace: None, factory: Arc::new(f) }
    }
}

/// Options for `ConfigModule::for_root`.
#[derive(Clone)]
pub struct ConfigModuleOptions {
    /// Register the module as global (no need to import it in other modules).
    pub is_global: bool,
    /// `.env` files to load (first one wins for duplicated keys). Default: `[".env"]`.
    pub env_file_path: Vec<String>,
    pub ignore_env_file: bool,
    /// Do not read the process environment.
    pub ignore_env_vars: bool,
    /// Custom configuration factories (`load: [configuration]`).
    pub load: Vec<ConfigFactory>,
    /// Validates the environment; return `Err(message)` to abort bootstrap.
    pub validate: Option<fn(&HashMap<String, String>) -> Result<(), String>>,
    /// Expand `${VAR}` references inside values.
    pub expand_variables: bool,
}

impl Default for ConfigModuleOptions {
    fn default() -> Self {
        ConfigModuleOptions {
            is_global: false,
            env_file_path: vec![".env".into()],
            ignore_env_file: false,
            ignore_env_vars: false,
            load: Vec::new(),
            validate: None,
            expand_variables: false,
        }
    }
}

/// Reads configuration values from environment variables, `.env` files and `load` factories.
pub struct ConfigService {
    env: RwLock<HashMap<String, String>>,
    internal: RwLock<Value>,
}

impl Default for ConfigService {
    fn default() -> Self {
        ConfigService::from_env()
    }
}

impl ConfigService {
    /// A service with the process environment only.
    pub fn from_env() -> Self {
        ConfigService { env: RwLock::new(std::env::vars().collect()), internal: RwLock::new(Value::Object(Map::new())) }
    }

    /// A service with explicit values (useful in tests).
    pub fn from_map(values: HashMap<String, String>) -> Self {
        ConfigService { env: RwLock::new(values), internal: RwLock::new(Value::Object(Map::new())) }
    }

    pub(crate) fn load(options: &ConfigModuleOptions) -> Result<Self, String> {
        let mut env: HashMap<String, String> = HashMap::new();
        if !options.ignore_env_file {
            for file in &options.env_file_path {
                if !Path::new(file).exists() {
                    continue;
                }
                match dotenvy::from_path_iter(file) {
                    Ok(iter) => {
                        for (k, v) in iter.flatten() {
                            env.entry(k).or_insert(v);
                        }
                    }
                    Err(e) => Logger::new("ConfigModule").warn(format!("Unable to parse {file}: {e}")),
                }
            }
        }
        if !options.ignore_env_vars {
            // Process env vars take precedence over .env files (dotenv semantics).
            for (k, v) in std::env::vars() {
                env.insert(k, v);
            }
        }
        if options.expand_variables {
            let snapshot = env.clone();
            for v in env.values_mut() {
                *v = expand(v, &snapshot);
            }
        }
        if let Some(validate) = options.validate {
            validate(&env).map_err(|e| format!("Config validation error: {e}"))?;
        }
        let mut internal = Map::new();
        for f in &options.load {
            merge_factory(&mut internal, f);
        }
        Ok(ConfigService { env: RwLock::new(env), internal: RwLock::new(Value::Object(internal)) })
    }

    pub(crate) fn add_factory(&self, f: &ConfigFactory) {
        if let Value::Object(map) = &mut *self.internal.write().unwrap() {
            merge_factory(map, f);
        }
    }

    fn raw(&self, key: &str) -> Option<Value> {
        if let Some(v) = self.env.read().unwrap().get(key) {
            return Some(Value::String(v.clone()));
        }
        let internal = self.internal.read().unwrap();
        let mut cur = &*internal;
        for part in key.split('.') {
            cur = cur.get(part)?;
        }
        Some(cur.clone())
    }

    /// `configService.get<T>(key)`; supports dot paths into `load`ed configuration.
    /// String values are converted to the requested type (`get::<u16>("PORT")`).
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.raw(key).and_then(|v| crate::common::lenient::from_value(v).ok())
    }

    /// `configService.get(key, defaultValue)`.
    pub fn get_or<T: DeserializeOwned>(&self, key: &str, default: T) -> T {
        self.get(key).unwrap_or(default)
    }

    /// `configService.getOrThrow(key)`. Panics when the key is missing.
    pub fn get_or_throw<T: DeserializeOwned>(&self, key: &str) -> T {
        match self.get(key) {
            Some(v) => v,
            None => panic!("Configuration key \"{key}\" does not exist"),
        }
    }

    /// Like `get_or_throw` but returning a `Result`.
    pub fn try_get<T: DeserializeOwned>(&self, key: &str) -> crate::Result<T> {
        self.get(key).ok_or_else(|| anyhow::anyhow!("Configuration key \"{key}\" does not exist"))
    }

    /// `configService.set(key, value)`.
    pub fn set(&self, key: &str, value: impl Into<String>) {
        self.env.write().unwrap().insert(key.to_string(), value.into());
    }
}

fn merge_factory(map: &mut Map<String, Value>, f: &ConfigFactory) {
    let v = (f.factory)();
    match &f.namespace {
        Some(ns) => {
            map.insert(ns.clone(), v);
        }
        None => {
            if let Value::Object(o) = v {
                for (k, v) in o {
                    map.insert(k, v);
                }
            }
        }
    }
}

fn expand(value: &str, env: &HashMap<String, String>) -> String {
    let re = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
    re.replace_all(value, |c: &regex::Captures<'_>| env.get(&c[1]).cloned().unwrap_or_default()).to_string()
}

/// `ConfigModule`.
#[derive(Default)]
pub struct ConfigModule;
crate::default_injectable!(ConfigModule);

impl Module for ConfigModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<ConfigModule>("ConfigModule").module_provider(Provider::class::<ConfigModule>())
    }
}

impl ConfigModule {
    /// `ConfigModule.forRoot(options)`.
    pub fn for_root(options: ConfigModuleOptions) -> DynamicModule {
        let global = options.is_global;
        DynamicModule::new::<ConfigModule>()
            .provider(Provider::use_factory(Token::of::<ConfigService>(), move |_ctx| {
                let options = options.clone();
                async move { ConfigService::load(&options).map_err(anyhow::Error::msg) }
            }))
            .export(Token::of::<ConfigService>())
            .global(global)
    }

    /// `ConfigModule.forFeature(registerAs('db', ...))` – adds a namespace to the
    /// existing `ConfigService` and exposes the namespace value under `"CONFIGURATION(<ns>)"`.
    pub fn for_feature(factory: ConfigFactory) -> DynamicModule {
        let ns = factory.namespace.clone().unwrap_or_default();
        let token = Token::named(format!("CONFIGURATION({ns})"));
        DynamicModule::new::<ConfigModule>()
            .provider(Provider::use_factory(token.clone(), move |ctx| {
                let factory = factory.clone();
                async move {
                    let svc = ctx.get::<ConfigService>().await?;
                    svc.add_factory(&factory);
                    Ok((factory.factory)())
                }
            }))
            .export(token)
    }
}
