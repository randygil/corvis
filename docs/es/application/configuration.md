---
status: documentado
---

# Configuración

Una aplicación suele necesitar valores que cambian entre entornos: la URL de la base de datos, el puerto, secretos de firma. Corvis los centraliza en `ConfigModule`, que lee el entorno del proceso y los archivos `.env`, y los expone a través del provider `ConfigService`.

## Registrar el módulo

`ConfigModule::for_root` devuelve un [módulo dinámico](/es/fundamentals/dynamic-modules) que registra y exporta `ConfigService`. Lo normal es importarlo una vez en el módulo raíz y marcarlo como global para no repetir el import en cada módulo:

```rust
use corvis::prelude::*;

#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
    ],
)]
pub struct AppModule;
```

A partir de ahí cualquier provider puede pedirlo como campo `Arc<ConfigService>`. Así lo usa el servicio de autenticación de `examples/tertulia`:

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

Y fuera de la inyección, en `main`, se obtiene del contenedor con `app.get`:

```rust
let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
```

## Leer valores

| Método | Comportamiento |
|---|---|
| `get::<T>(clave)` | `Option<T>`; `None` si la clave no existe o no se puede convertir a `T` |
| `get_or(clave, defecto)` | El valor o `defecto` |
| `try_get::<T>(clave)` | `corvis::Result<T>`; error `Configuration key "X" does not exist` si falta |
| `get_or_throw::<T>(clave)` | El valor; hace `panic` si falta |
| `set(clave, valor)` | Cambia o agrega un valor en tiempo de ejecución |

Los valores del entorno son texto, pero `get` los convierte al tipo pedido: `get::<u16>("PORT")` interpreta `"3000"` como número y `get::<bool>("LOG_QUERIES")` acepta `"true"`. Para secretos obligatorios, prefiere `try_get` o `get` con un error explícito, como hace el `AppModule` de Tertulia al configurar el JWT:

```rust
let secret = config.get::<String>("JWT_SECRET").filter(|s| !s.trim().is_empty());
let secret = secret.ok_or_else(|| corvis::anyhow::anyhow!("JWT_SECRET is required"))?;
```

## Opciones

`ConfigModuleOptions` tiene estos campos (todos con valor por defecto, así que se combinan con `..Default::default()`):

| Campo | Por defecto | Uso |
|---|---|---|
| `is_global` | `false` | Registra el módulo como global |
| `env_file_path` | `vec![".env"]` | Archivos `.env` a leer; si una clave se repite entre archivos, gana el primero |
| `ignore_env_file` | `false` | No leer ningún `.env` |
| `ignore_env_vars` | `false` | No leer el entorno del proceso |
| `load` | vacío | Fábricas de configuración propia (ver abajo) |
| `validate` | `None` | Función que valida las variables al arrancar |
| `expand_variables` | `false` | Expande `$VAR`, `${VAR}` y `${VAR:-defecto}` en los valores de los `.env` |

El orden de precedencia es: variables del proceso, luego los archivos `.env` (en el orden de `env_file_path`) y, para claves con puntos, las fábricas de `load`. Un archivo que no existe se ignora sin error.

### Validar al arrancar

`validate` recibe todas las variables ya combinadas. Si devuelve `Err`, la creación de `ConfigService` falla y la aplicación no arranca:

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

El mensaje que verás en el arranque empieza con `Config validation error:`.

## Formato de los archivos `.env`

Los archivos se leen con la gramática de `dotenv` de Node.js (`corvis::config::parse_dotenv` la expone por si la necesitas):

- `CLAVE=valor`, `CLAVE: valor` y `export CLAVE=valor`.
- Un `#` sin comillas empieza un comentario y corta el valor.
- Comillas simples, dobles o invertidas; los valores entre comillas pueden ocupar varias líneas y, dentro de comillas dobles, `\n` es un salto de línea.
- Dentro de un mismo archivo, si una clave se repite gana la última.

Los valores se toman **tal cual**: `JWT_SECRET=pa$$w0rd` vale exactamente `pa$$w0rd`. La expansión de variables solo ocurre con `expand_variables: true` y solo en los valores que vienen de archivos; los del entorno del proceso nunca se expanden.

## Configuración propia con `load`

Además de variables planas, puedes cargar objetos JSON y leerlos con rutas de puntos. `corvis::config::register_as` crea una fábrica con espacio de nombres:

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

// En un provider:
let host: String = self.config.get_or_throw("database.host");
let port: u16 = self.config.get_or("database.port", 5432);
```

Una closure que devuelve un objeto también se convierte en fábrica (sin espacio de nombres: sus claves quedan en la raíz): `load: vec![(|| json!({ "featureX": true })).into()]`.

Las variables del entorno tienen prioridad: si existe una variable llamada literalmente `database.host`, `get("database.host")` devuelve esa.

### Configuración por módulo

`ConfigModule::for_feature(fábrica)` agrega un espacio de nombres al `ConfigService` existente desde un módulo de funcionalidad, y además registra su valor bajo el token `"CONFIGURATION(<nombre>)"`:

```rust
#[module(imports = [ConfigModule::for_feature(register_as("payments", || json!({ "currency": "USD" })))])]
pub struct PaymentsModule;

#[injectable]
pub struct PaymentsService {
    #[inject("CONFIGURATION(payments)")]
    payments: Arc<Value>,
}
```

`for_feature` resuelve `ConfigService` desde el contexto del módulo, así que `ConfigModule::for_root` tiene que estar registrado con `is_global: true`.

## En las pruebas

`ConfigService::from_map` crea un servicio con valores fijos, útil para sustituir el real con `override_provider` en un módulo de pruebas (ver [Testing](/es/fundamentals/testing)):

```rust
let values = std::collections::HashMap::from([("PORT".to_string(), "0".to_string())]);
let config = ConfigService::from_map(values);
```
