---
status: documented
---

# Providers

Un **provider** es cualquier valor que el contenedor de inyección de dependencias sabe crear y entregar a quien lo necesite: servicios, repositorios, clientes de otras APIs, configuraciones, fábricas. Los controladores no crean sus dependencias; las declaran, y Corvis las resuelve al arrancar la aplicación.

En esta página verás la forma más común, un `struct` marcado con `#[injectable]`. Las variantes (valores, fábricas, alias, trait objects) están en [Providers personalizados](/es/fundamentals/custom-providers).

## Un servicio

En Tertulia, `RoomsService` guarda las salas y el historial de mensajes. Lo usan tanto el gateway de WebSockets como el controlador REST:

```rust
use corvis::prelude::*;
use corvis_orm::{OrmError, OrmService};

#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

impl RoomsService {
    pub async fn find(&self, name: &str) -> Result<Option<Room>, OrmError> {
        self.db.room().find_unique(json!({ "where": { "name": name } })).await
    }

    pub async fn create(&self, dto: CreateRoomDto, created_by: &str) -> Result<Room, OrmError> {
        let room: Room = self.db.room().create(json!({ "data": { /* ... */ } })).await?;
        self.events.emit("room.created", room.clone());
        Ok(room)
    }
}
```

`#[injectable]` genera la implementación del trait `Injectable`: un constructor asíncrono que pide cada campo al contenedor. No escribes `new()`; los campos **son** la lista de dependencias.

Para que el contenedor lo conozca, el servicio se registra en los `providers` de un módulo:

```rust
#[module(
    controllers = [RoomsController, StatsController],
    providers = [ChatGateway, RoomsService, FirehoseService],
    exports = [ChatGateway, RoomsService],
)]
pub struct ChatModule;
```

## Cómo se resuelve cada campo

El constructor generado mira el tipo de cada campo:

| Campo | Qué recibe |
|---|---|
| `Arc<T>` | El provider registrado con el token `T`. Si no está visible desde el módulo, el arranque falla con un error que nombra el campo. |
| `Option<Arc<T>>` | El provider si está visible; `None` si no. Es la forma de declarar una dependencia opcional. |
| `ModuleRef` | Una referencia al contenedor, situada en el módulo dueño del provider. Ver [Referencia al módulo](/es/fundamentals/module-ref). |
| `#[inject("TOKEN")] campo: Arc<T>` | El provider registrado con el token de texto `"TOKEN"`. |
| `#[inject("TOKEN")] campo: T` | Lo mismo, pero clonando el valor (`T: Clone`). Útil para valores pequeños como un `String` o un `u16`. |
| `#[init(expr)] campo: T` | El resultado de evaluar `expr`. No pasa por el contenedor. |
| `#[default] campo: T` | `T::default()`. |
| Cualquier otro tipo | `T::default()`, si `T` implementa `Default`; si no, es un error de compilación. |

Un ejemplo que combina varias formas (`ChatGateway`, de Tertulia):

```rust
#[websocket_gateway(path = "/chat")]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
    jwt: Arc<JwtService>,              // por tipo, viene de JwtModule (global)
    rooms: Arc<RoomsService>,          // por tipo, del mismo módulo
    firehose: Arc<FirehoseService>,
    #[init(Logger::new("ChatGateway"))]
    logger: Logger,                    // valor propio, sin contenedor
}
```

Y uno con token de texto y estado interno:

```rust
#[injectable]
pub struct CatsService {
    #[default]
    cats: Mutex<Vec<Cat>>,
    #[inject("PREFIX")]
    prefix: String,                    // Provider::use_value("PREFIX", "Sir ".to_string())
}
```

::: tip Estado mutable
Un provider por defecto es un singleton compartido entre todos sus consumidores a través de `Arc`, así que recibe `&self`. El estado que cambia va dentro de un `Mutex`, `RwLock`, atómicos o un `Arc` de estado compartido, como `cats: Mutex<Vec<Cat>>` en el ejemplo anterior.
:::

Para hacer opcional una dependencia, declara el campo como `Option<Arc<T>>`: vale `None` cuando ningún provider es visible. El atributo `#[optional]` no existe y se rechaza al compilar.

## Tokens

Cada provider se identifica con un `Token`:

- `Token::of::<T>()`: el token de un tipo. Es el que usan `providers = [RoomsService]` y los campos `Arc<RoomsService>`.
- `Token::named("API_KEY")`: un token de texto. Los `&str` y `String` se convierten solos en `Token`, por eso `Provider::use_value("API_KEY", ...)` y `#[inject("API_KEY")]` funcionan sin escribir `Token::named`.

Dos tokens de tipo son iguales si el tipo es el mismo; dos tokens de texto, si el texto es el mismo. Un token de tipo nunca coincide con uno de texto.

## Registro y ciclo de vida

- **Cuándo se crean.** `CorvisFactory::create` recorre el grafo de módulos y crea todas las instancias antes de devolver la aplicación (salvo los providers *transient*, que se crean al inyectarse). Una dependencia que falta o un ciclo se detectan al arrancar, no en la primera petición.
- **Cuántas instancias.** Una por provider y por módulo que lo registra. Si dos módulos ponen `RoomsService` en sus `providers`, cada uno tendrá la suya. Para compartir una sola, regístrala en un módulo y expórtala (ver [Módulos](/es/modules)).
- **Scopes.** Por defecto, singleton. `#[injectable(scope = "transient")]` crea una instancia por consumidor. No hay scope por petición. Ver [Scopes de inyección](/es/fundamentals/injection-scopes).
- **Hooks.** Si el struct implementa `OnModuleInit`, `OnApplicationBootstrap`, `OnModuleDestroy`, `BeforeApplicationShutdown` u `OnApplicationShutdown`, Corvis lo detecta sin registrarlo en ningún sitio. `RoomsService` usa `OnApplicationBootstrap` para crear las salas por defecto:

```rust
#[async_trait]
impl OnApplicationBootstrap for RoomsService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        for (name, topic) in DEFAULT_ROOMS {
            if self.find(name).await?.is_none() {
                self.db.room().create(json!({ "data": { "name": name, "topic": topic, "createdBy": "tertulia" } })).await?;
            }
        }
        Ok(())
    }
}
```

Más detalles en [Eventos del ciclo de vida](/es/fundamentals/lifecycle-events).

## Cuando falta una dependencia

Si un campo `Arc<T>` no se puede resolver, el arranque falla con un mensaje que indica el provider, la posición del argumento (marcada con `?`) y el módulo:

```text
Corvis can't resolve dependencies of the NeedsCats (?). Please make sure that the argument
CatsService at index [0] is available in the BrokenModule context.
```

Las causas habituales son tres: el provider no está en los `providers` del módulo, está en otro módulo que no lo exporta, o ese otro módulo no está en los `imports`. Los campos con `#[init]` o `#[default]` aparecen como `-` en la lista.

## Obtener un provider fuera de la inyección

Desde `main` o desde un test puedes pedir una instancia ya creada a la aplicación:

```rust
let app = CorvisFactory::create::<AppModule>().await?;
let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 5150);
```

`app.get::<T>()` busca por tipo y `app.get_token::<T>("TOKEN")` por token de texto. Dentro de un provider, usa un campo `ModuleRef` (ver [Referencia al módulo](/es/fundamentals/module-ref)).

## Guards, pipes e interceptors también son providers

Los guards, pipes, interceptors, filtros y middleware que referencias por tipo (`#[use_guards(AuthGuard)]`) se construyen con el mismo mecanismo, así que pueden tener campos `Arc<T>`. Si están registrados como providers del módulo, se usa esa instancia; si no, Corvis los construye en el contexto del módulo del controlador y guarda la instancia. Por eso `AuthGuard` de Tertulia puede recibir `Arc<JwtService>`:

```rust
#[injectable]
pub struct AuthGuard {
    jwt: Arc<JwtService>,
}
```
