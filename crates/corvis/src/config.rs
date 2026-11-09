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
use std::sync::{Arc, LazyLock, RwLock};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, Token};
use crate::logger::Logger;

type EnvValidator = fn(&HashMap<String, String>) -> Result<(), String>;

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
    pub validate: Option<EnvValidator>,
    /// `expandVariables`: expand `$VAR`, `${VAR}` and `${VAR:-default}` inside the values read
    /// from `.env` files (`dotenv-expand`). Off by default: values are taken verbatim, so a
    /// secret such as `pa$$w0rd` is not altered.
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
                match std::fs::read(file) {
                    Ok(bytes) => {
                        // Inside a file the last duplicate wins (`dotenv.parse`); across files
                        // the first file wins (`ConfigModule.forRoot({ envFilePath: [...] })`).
                        let parsed: HashMap<String, String> = parse_dotenv(&String::from_utf8_lossy(&bytes)).into_iter().collect();
                        for (k, v) in parsed {
                            env.entry(k).or_insert(v);
                        }
                    }
                    Err(e) => Logger::new("ConfigModule").warn(format!("Unable to read {file}: {e}")),
                }
            }
        }
        let process: HashMap<String, String> = if options.ignore_env_vars { HashMap::new() } else { std::env::vars().collect() };
        if options.expand_variables {
            // `dotenv-expand`: only the values read from `.env` files are expanded; values of the
            // process environment win over the file and are used verbatim.
            env.retain(|k, _| !process.contains_key(k));
            let file_values = env.clone();
            for v in env.values_mut() {
                *v = expand(v, &process, &file_values, 0);
            }
        }
        // Process env vars take precedence over .env files (dotenv semantics).
        env.extend(process);
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

/// Parses the contents of a `.env` file with the grammar of Node's `dotenv` (v16) and
/// returns the pairs in file order (when collected into a map, later duplicates win).
///
/// - `KEY=value`, `KEY: value`, `export KEY=value`; keys are `[A-Za-z0-9_.-]+`.
/// - Unquoted values are trimmed and end at the first `#` (comment).
/// - `'single'`, `"double"` and `` `backtick` `` quoted values may span several lines; inside
///   double quotes `\n` and `\r` become a newline and a carriage return.
/// - `$VAR` is never expanded here (see [`ConfigModuleOptions::expand_variables`]).
pub fn parse_dotenv(src: &str) -> Vec<(String, String)> {
    static LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r#"(?m)^\s*(?:export\s+)?([A-Za-z0-9_.-]+)(?:\s*=\s*?|:\s+?)(\s*'(?:\\'|[^'])*'|\s*"(?:\\"|[^"])*"|\s*`(?:\\`|[^`])*`|[^#\r\n]+)?\s*(?:#.*)?$"#,
        )
        .expect("dotenv LINE regex")
    });
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let lines = src.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    for c in LINE.captures_iter(&lines) {
        let raw = c.get(2).map(|m| m.as_str()).unwrap_or("").trim();
        let first = raw.chars().next();
        let mut value = match first {
            Some(q @ ('\'' | '"' | '`')) if raw.len() >= 2 && raw.ends_with(q) => raw[1..raw.len() - 1].to_string(),
            _ => raw.to_string(),
        };
        if first == Some('"') {
            value = value.replace("\\n", "\n").replace("\\r", "\r");
        }
        out.push((c[1].to_string(), value));
    }
    out
}

/// `dotenv-expand`: `$VAR`, `${VAR}`, `${VAR:-default}` and `\$` (a literal `$`). References
/// are looked up in the process environment first, then in the other `.env` values (expanded
/// recursively, up to a fixed depth); unknown variables expand to an empty string.
fn expand(value: &str, process: &HashMap<String, String>, file: &HashMap<String, String>, depth: usize) -> String {
    static VAR: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\\\$|\$\{([A-Za-z0-9_]+)(?::-([^}]*))?\}|\$([A-Za-z0-9_]+)").expect("expand regex"));
    const MAX_DEPTH: usize = 16;
    VAR.replace_all(value, |c: &regex::Captures<'_>| {
        if &c[0] == "\\$" {
            return "$".to_string();
        }
        let name = c.get(1).or_else(|| c.get(3)).map(|m| m.as_str()).unwrap_or("");
        if let Some(v) = process.get(name) {
            return v.clone();
        }
        let found = file.get(name).map(String::as_str).filter(|v| !v.is_empty());
        match found.or_else(|| c.get(2).map(|m| m.as_str())) {
            Some(v) if depth < MAX_DEPTH => expand(v, process, file, depth + 1),
            Some(v) => v.to_string(),
            None => String::new(),
        }
    })
    .into_owned()
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

    /// `ConfigModule.forRootAsync({ useFactory })`: the options come from `factory`, which can resolve other
    /// providers (of global modules or of modules added with `.import(..)`). The module is global when `is_global`
    /// is `true`; the `is_global` field of the returned options is ignored because globality is fixed before any
    /// factory runs. The factory cannot resolve `ConfigService` itself (circular dependency).
    pub fn for_root_async<F, Fut>(is_global: bool, factory: F) -> DynamicModule
    where
        F: Fn(crate::di::ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = crate::Result<ConfigModuleOptions>> + Send + 'static,
    {
        let factory = Arc::new(factory);
        DynamicModule::new::<ConfigModule>()
            .provider(Provider::use_factory(Token::of::<ConfigService>(), move |ctx| {
                let factory = factory.clone();
                async move {
                    let options = factory(ctx).await?;
                    ConfigService::load(&options).map_err(anyhow::Error::msg)
                }
            }))
            .export(Token::of::<ConfigService>())
            .global(is_global)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn load_file(contents: &str, expand_variables: bool) -> ConfigService {
        let dir = std::env::temp_dir().join(format!("corvis-config-{}-{}", std::process::id(), rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".env");
        std::fs::write(&file, contents).unwrap();
        let options = ConfigModuleOptions {
            env_file_path: vec![file.to_string_lossy().into_owned()],
            ignore_env_vars: true,
            expand_variables,
            ..Default::default()
        };
        let svc = ConfigService::load(&options).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        svc
    }

    /// Secrets with `$` are read verbatim, like Node's `dotenv` (no `dotenv-expand`).
    #[test]
    fn dotenv_does_not_expand_dollar_by_default() {
        let svc = load_file(
            "JWT_SECRET_A=pa$$w0rd-Xk9\nJWT_SECRET_B=\"s3cr3t$HOME-tail\"\nJWT_SECRET_C=abc${UNDEFINED_VAR}def\nJWT_SECRET_D='literal$HOME'\nJWT_SECRET_E=$NOPE\n",
            false,
        );
        assert_eq!(svc.get::<String>("JWT_SECRET_A").as_deref(), Some("pa$$w0rd-Xk9"));
        assert_eq!(svc.get::<String>("JWT_SECRET_B").as_deref(), Some("s3cr3t$HOME-tail"));
        assert_eq!(svc.get::<String>("JWT_SECRET_C").as_deref(), Some("abc${UNDEFINED_VAR}def"));
        assert_eq!(svc.get::<String>("JWT_SECRET_D").as_deref(), Some("literal$HOME"));
        assert_eq!(svc.get::<String>("JWT_SECRET_E").as_deref(), Some("$NOPE"));
    }

    /// The `dotenv` grammar: comments, quotes, `export`, multi-line values, `\n` in double quotes,
    /// last duplicate wins inside a file.
    #[test]
    fn dotenv_grammar_matches_node() {
        let svc = load_file(
            "\u{feff}# comment\nexport A=1\nB = spaced value   # trailing comment\nC=\"line1\nline2\"\nD='no \n escape'\nE=\"multi\nline\"\nF=\nG=hash#inside\nH=\"quoted # not a comment\" # comment\nA=2\nI: colon\nJ=`backtick`\nK=\"a\\\"b\"\r\nL=crlf\r\n",
            false,
        );
        let get = |k: &str| svc.get::<String>(k);
        assert_eq!(get("A").as_deref(), Some("2"));
        assert_eq!(get("B").as_deref(), Some("spaced value"));
        assert_eq!(get("C").as_deref(), Some("line1\nline2"));
        assert_eq!(get("D").as_deref(), Some("no \n escape"));
        assert_eq!(get("E").as_deref(), Some("multi\nline"));
        assert_eq!(get("F").as_deref(), Some(""));
        assert_eq!(get("G").as_deref(), Some("hash"));
        assert_eq!(get("H").as_deref(), Some("quoted # not a comment"));
        assert_eq!(get("I").as_deref(), Some("colon"));
        assert_eq!(get("J").as_deref(), Some("backtick"));
        assert_eq!(get("K").as_deref(), Some("a\\\"b"));
        assert_eq!(get("L").as_deref(), Some("crlf"));
    }

    /// With `expand_variables: true`, `$VAR`, `${VAR}`, `${VAR:-default}` and `\$` work like
    /// `dotenv-expand`.
    #[test]
    fn dotenv_expands_only_when_requested() {
        let svc = load_file(
            "HOST=example.com\nURL=https://$HOST/api\nURL2=https://${HOST}:${PORT:-8080}\nPRICE=\\$5\nCHAIN=$URL/v1\nMISSING=[$NOPE]\n",
            true,
        );
        let get = |k: &str| svc.get::<String>(k);
        assert_eq!(get("URL").as_deref(), Some("https://example.com/api"));
        assert_eq!(get("URL2").as_deref(), Some("https://example.com:8080"));
        assert_eq!(get("PRICE").as_deref(), Some("$5"));
        assert_eq!(get("CHAIN").as_deref(), Some("https://example.com/api/v1"));
        assert_eq!(get("MISSING").as_deref(), Some("[]"));
    }
}

#[allow(dead_code)]
fn  throwaway_misformatted( ) {}
