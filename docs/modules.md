---
status: documentado
---

# Módulos

Un **módulo** agrupa controladores y providers que trabajan juntos y decide cuáles de ellos puede usar el resto de la aplicación. Toda aplicación tiene un módulo raíz, el que recibe `CorvisFactory::create::<AppModule>()`, y a partir de él Corvis recorre los `imports` para construir el grafo completo.

## Declarar un módulo

Un módulo es un `struct` con el atributo `#[module(...)]`:

```rust
use corvis::prelude::*;

#[module(
    controllers = [RoomsController, StatsController],
    providers = [ChatGateway, RoomsService, FirehoseService],
    exports = [ChatGateway, RoomsService],
)]
pub struct ChatModule;
```

El atributo acepta cinco claves, todas opcionales:

| Clave | Contenido |
|---|---|
| `imports` | Otros módulos cuyos exports necesita este módulo: tipos (`AuthModule`) o módulos dinámicos (`ConfigModule::for_root(...)`). |
| `controllers` | Los controladores que este módulo monta en el router. |
| `providers` | Los providers que el contenedor crea en este módulo: tipos `#[injectable]` o expresiones `Provider` (`Provider::use_value(...)`, ver [Providers personalizados](/fundamentals/custom-providers)). |
| `exports` | Lo que este módulo deja usar a quien lo importe: tipos de providers, tokens de texto (`"API_KEY"`) o módulos importados (re-export). |
| `global` | Marca el módulo como global: `global`, `global = true`. |

Cada clave recibe una lista entre corchetes o un único elemento (`imports = AuthModule`). Una clave desconocida es un error de compilación.

## El módulo raíz de Tertulia

```rust
#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        EventEmitterModule::for_root(EventEmitterOptions::default()),
        JwtModule::register_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let secret = config.get::<String>("JWT_SECRET").filter(|s| !s.trim().is_empty());
            let secret = secret.ok_or_else(|| corvis::anyhow::anyhow!("JWT_SECRET is required"))?;
            Ok(JwtModuleOptions::new(secret).expires_in(config.get_or("JWT_EXPIRES_IN", "12h".to_string())))
        }),
        OrmModule::for_root_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let url: String = config.get_or("DATABASE_URL", "file:./tertulia.db".to_string());
            Ok(OrmModuleOptions::new(url).push::<(Room, ChatMessage)>())
        }),
        AuthModule,
        ChatModule,
    ],
)]
pub struct AppModule;
```

El módulo raíz no tiene controladores ni providers propios: compone la aplicación a partir de cuatro módulos de infraestructura (configuración, eventos, JWT y base de datos, los cuatro globales: `EventEmitterOptions::default()` ya trae `global: true`) y dos módulos de funcionalidad, `AuthModule` y `ChatModule`. Es la forma recomendada de organizar una aplicación: un módulo por área, con su controlador, sus servicios y lo que expone.

```text
AppModule
├── ConfigModule        (global)  exporta ConfigService
├── EventEmitterModule  (global)  exporta EventEmitter
├── JwtModule           (global)  exporta JwtService
├── OrmModule           (global)  exporta OrmService
├── AuthModule                    AuthController · AuthService → exporta AuthService
└── ChatModule                    RoomsController, StatsController · ChatGateway, RoomsService,
                                  FirehoseService → exporta ChatGateway, RoomsService
```

## Encapsulación

Cuando un provider o un controlador pide una dependencia, el contenedor la busca **desde el módulo que lo registra** y solo en estos lugares, en este orden:

1. Los providers del propio módulo.
2. Lo que exportan los módulos de sus `imports`, incluido lo que esos módulos re-exportan.
3. Lo que exportan los módulos globales.

Nada más. En particular:

- Un módulo **no ve** los providers del módulo que lo importa. `ChatModule` no puede inyectar nada que `AppModule` registre en sus `providers` salvo que lo reciba por un import o un módulo global.
- Un módulo no ve los providers de un módulo hermano si no lo importa. `ChatModule` no importa `AuthModule`, así que no podría pedir `Arc<AuthService>`; no lo necesita, porque autentica con `JwtService`, que llega desde `JwtModule` (global).
- Registrar un provider **no** lo expone. Sin `exports`, un provider es privado de su módulo.
- Un módulo global tampoco expone sus providers por estar registrado: solo cuenta lo que pone en `exports`.

Con estas reglas, en Tertulia:

| Quién pide | Qué | Dónde lo encuentra |
|---|---|---|
| `RoomsController` (`ChatModule`) | `Arc<RoomsService>`, `Arc<ChatGateway>` | Providers del propio `ChatModule` |
| `StatsController` (`ChatModule`) | `Arc<ConfigService>` | Export de `ConfigModule`, que es global |
| `RoomsService` (`ChatModule`) | `Arc<OrmService>` | Export de `OrmModule` (global, por el `true` de `for_root_async`) |
| `RoomsService` (`ChatModule`) | `Arc<EventEmitter>` | Export de `EventEmitterModule`, que es global por defecto (`EventEmitterOptions::default()`) |
| `AuthService` (`AuthModule`) | `Arc<JwtService>`, `Arc<ConfigService>` | Exports de dos módulos globales |

Si una dependencia no es visible, el arranque falla y el mensaje nombra el módulo donde se buscó:

```text
Corvis can't resolve dependencies of the NeedsCats (?). Please make sure that the argument
CatsService at index [0] is available in the BrokenModule context.
```

## Compartir providers

Una instancia de provider pertenece al módulo que la registra. Todos los módulos que importan ese módulo y reciben el provider por su `exports` comparten **la misma** instancia. En Tertulia, el `RoomsService` que usa `ChatGateway` es el mismo objeto que usa `RoomsController`.

Lo contrario también es cierto: si dos módulos ponen el mismo tipo en sus `providers`, cada uno crea su propia instancia. Para tener una sola, regístrala en un módulo, expórtala e importa ese módulo donde la necesites.

```rust
#[module(providers = [UsersService], exports = [UsersService])]
pub struct UsersModule;

#[module(imports = [UsersModule], controllers = [AuthController], providers = [AuthService], exports = [AuthService])]
pub struct AuthModule;   // AuthService recibe el UsersService de UsersModule
```

Importar el mismo módulo estático desde varios sitios no crea copias: el grafo tiene un único nodo por tipo de módulo, así que `UsersModule` se instancia una sola vez aunque lo importen `AuthModule` y `AppModule`.

## Qué se puede exportar

```rust
#[module(
    imports = [CommonModule],
    providers = [
        CatsService,
        Provider::use_value("API_KEY", "secret".to_string()),
    ],
    exports = [
        CatsService,     // un provider propio, por tipo
        "API_KEY",       // un provider propio, por token de texto
        CommonModule,    // re-export: quien importe este módulo ve los exports de CommonModule
    ],
)]
pub struct CatsModule;
```

- Un **tipo** o un **token** debe ser un provider del propio módulo o algo que uno de sus `imports` ya exporte (así se re-exporta un provider suelto).
- Un **módulo** solo se puede exportar si está en los `imports` del mismo módulo.

Exportar algo que no cumple estas reglas hace fallar el arranque:

```text
Corvis cannot export a provider/module that is not a part of the currently processed module
(CatsModule). Please verify whether the exported X is available in this particular context.
```

También se aceptan expresiones que se convierten en `ExportItem`, como `Token::of::<CatsService>()` o un `String` con el nombre del token.

## Módulos globales

Un módulo global hace visibles sus exports en **todos** los módulos de la aplicación sin que tengan que importarlo. Es útil para infraestructura que casi todo usa (configuración, base de datos, JWT); para lo demás, los imports explícitos documentan mejor las dependencias.

```rust
#[module(global, providers = [AuditService], exports = [AuditService])]
pub struct AuditModule;

// equivalentes:
#[module(global = true, providers = [AuditService], exports = [AuditService])]
pub struct AuditModule2;

#[module(providers = [AuditService], exports = [AuditService])]
#[global]
pub struct AuditModule3;
```

::: warning Posición de `#[global]`
`#[global]` debe ir **debajo** de `#[module(...)]`. Encima, Rust lo procesa antes que el atributo `#[module]` y la compilación falla con `cannot find attribute 'global' in this scope`.
:::

Dos condiciones para que un módulo global funcione:

1. **Tiene que estar en el grafo.** Un módulo global se registra cuando algún módulo lo importa (normalmente el raíz). Ser global evita repetir el import, no el primero.
2. **Tiene que exportar.** Igual que cualquier módulo, solo comparte lo que está en `exports`.

Los módulos dinámicos deciden si son globales con `.global(bool)`. Los módulos incluidos lo exponen como opción: `ConfigModuleOptions { is_global: true, .. }`, `CacheOptions { is_global: true, .. }`, `JwtModule::register_async(true, ..)`, `OrmModule::for_root_async(true, ..)`. `ScheduleModule` es siempre global.

## El módulo también es un provider

El `struct` del módulo se construye con las mismas reglas que un `#[injectable]`: sus campos `Arc<T>` se inyectan y puede implementar los hooks del ciclo de vida. Además, si implementa `CorvisModule`, su método `configure` registra middleware para las rutas de la aplicación:

```rust
#[module(imports = [/* ... */], controllers = [AppController], providers = [AppService])]
pub struct AppModule;

impl CorvisModule for AppModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<LoggerMiddleware>().for_routes("*");
    }
}
```

Ver [Middleware](/middleware).

## Módulos dinámicos

Un módulo dinámico es un módulo cuya lista de providers se completa en el momento de importarlo, normalmente a partir de opciones: `ConfigModule::for_root(...)`, `JwtModule::register_async(...)`, `OrmModule::for_root_async(...)`. Se escribe con un método que devuelve `DynamicModule`, y se explica en [Módulos dinámicos](/fundamentals/dynamic-modules).

A diferencia de los módulos estáticos, **cada llamada** a `for_root(...)` o `register(...)` en un `imports` crea un nodo nuevo en el grafo, con sus propias instancias. Llama a esos métodos una vez (en el módulo raíz, o en un módulo de infraestructura) y comparte el resultado haciéndolo global o exportándolo.

## Inspeccionar el grafo

`app.modules_info()` devuelve los módulos de la aplicación con su nombre, si son globales, los índices de sus imports y sus controladores con las rutas. No necesita `app.init()`:

```rust
let app = CorvisFactory::create::<AppModule>().await?;
for m in app.modules_info().iter().filter(|m| !m.internal) {
    println!("{} global={} controllers={}", m.name, m.global, m.controllers.len());
}
```

La lista incluye un módulo interno (`InternalCoreModule`, con `internal: true`) que aporta `Reflector` y `HttpAdapterHost` a toda la aplicación.

## Orden de inicialización

Los providers se crean al arrancar, módulo por módulo, y el log muestra una línea por módulo:

```text
LOG [InstanceLoader] AppModule dependencies initialized +1ms
```

Los hooks del ciclo de vida (`on_module_init`, `on_application_bootstrap` y los de cierre) se ejecutan empezando por los módulos más alejados del raíz. Ver [Eventos del ciclo de vida](/fundamentals/lifecycle-events).
