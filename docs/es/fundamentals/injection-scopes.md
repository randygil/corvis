---
status: partial
---

# Scopes de inyección

El scope de un provider decide cuántas instancias crea el contenedor y cuándo. Corvis tiene tres:

| Scope | Instancias | Cuándo se crea |
|---|---|---|
| `Scope::Default` (singleton) | Una por módulo que registra el provider, compartida por todos sus consumidores. | Al arrancar la aplicación. |
| `Scope::Transient` | Una nueva para cada consumidor que la inyecta. | Cada vez que se resuelve. |
| `Scope::Request` | Como máximo una por petición HTTP y por módulo que registra el provider. | La primera vez que se resuelve durante una petición. |

## Singleton (por defecto)

Es el scope de todo provider que no diga lo contrario y el adecuado en la gran mayoría de los casos: servicios sin estado, clientes de base de datos, cachés, configuración. Como la instancia es compartida y concurrente, su estado mutable va detrás de un `Mutex`, `RwLock` o atómicos.

## Transient

Declara un provider transient en el atributo:

```rust
#[injectable(scope = "transient")]
pub struct OperationTimer {
    #[init(std::time::Instant::now())]
    started: std::time::Instant,
}
```

o al registrarlo, sobre cualquier `Provider`:

```rust
providers = [Provider::use_class::<OperationTimer>("TIMER").scope(Scope::Transient)]
```

Cada punto de inyección (cada campo `Arc<OperationTimer>`, incluso dos en el mismo struct) recibe su propia instancia, y `ModuleRef::resolve::<OperationTimer>()` crea una nueva en cada llamada.

Consecuencias que conviene conocer:

- **No se crean al arrancar.** El contenedor no instancia los providers transient por adelantado; los construye al inyectarlos. Sus dependencias sí se comprueban al arrancar: un provider transient de clase (o un alias de uno) que pide algo que su módulo no ve hace fallar el arranque con el error habitual de "can't resolve dependencies", aunque nadie lo inyecte todavía. Los campos opcionales (`Option<Arc<T>>`) se omiten, y los providers de factory no se comprueban porque sus dependencias son código: el error de una factory transient aparece la primera vez que se resuelve. Prefiere los providers de clase cuando importe validar pronto.
- **No se guardan.** Por eso `ModuleRef::get::<T>()` y `app.get::<T>()`, que solo devuelven instancias existentes, no encuentran un provider transient. Usa `module_ref.resolve::<T>().await`.
- **No reciben hooks del ciclo de vida.** Los hooks (`on_module_init`, `on_application_bootstrap`, los de cierre) se ejecutan sobre las instancias guardadas, así que un provider transient no los recibe.
- **El scope no se propaga.** Un singleton que depende de un transient recibe una sola instancia al crearse y la conserva.

## Scope por petición

Declara un provider con scope por petición en el atributo:

```rust
#[injectable(scope = "request")]
pub struct RequestContext {
    req: Request,            // la petición que se está atendiendo
    clock: Arc<Clock>,       // un singleton, como siempre
}
```

o al registrarlo: `Provider::use_class::<RequestContext>("CTX").scope(Scope::Request)`. El atributo acepta
exactamente `"default"`, `"transient"` y `"request"` (o las rutas `Scope::*`); cualquier otra cosa es un error de
compilación.

La instancia se crea la primera vez que algo la resuelve durante una petición, y cada resolución posterior en esa
petición (desde un middleware, un guard, el handler u otro provider con scope por petición) devuelve la misma. Dos
peticiones nunca comparten instancia, ni siquiera cuando se ejecutan a la vez. Cuando la petición termina (respuesta
enviada, error, pánico o el cliente se desconecta) las instancias se liberan.

Un provider con scope por petición se consume en cada petición, con `ModuleRef::resolve_for`. Inyecta un `ModuleRef`
y recibe el `Request` como parámetro del handler:

```rust
#[controller("cats")]
pub struct CatsController {
    module_ref: ModuleRef,
}

#[routes]
impl CatsController {
    #[get("me")]
    async fn me(&self, #[req] req: Request) -> String {
        let ctx = self.module_ref.resolve_for::<RequestContext>(&req).await.unwrap();
        ctx.user_id()
    }
}
```

`resolve_for::<T>(&req)` funciona con un provider de cualquier scope y sigue las reglas de visibilidad habituales
(solo un provider con scope por petición necesita una petición que se esté atendiendo; los demás se resuelven como con
`resolve`);
`resolve_token_for::<T>("TOKEN", &req)` hace lo mismo con un token propio. Un provider de factory lee la petición con
`ctx.request()?`:

```rust
Provider::use_factory("TENANT", |ctx| async move {
    Ok(Tenant(ctx.request()?.header("x-tenant").unwrap_or("none").to_string()))
})
.scope(Scope::Request)
```

Qué puede depender de qué:

- Un provider con scope por petición puede depender de singletons, de providers transient y de otros providers con
  scope por petición, y puede tener un campo `Request`. Un provider transient construido para él puede hacer lo mismo.
- Un **singleton, un controller o un gateway no pueden depender de un provider con scope por petición** ni de un
  `Request` (directamente, a través de un alias o de providers transient). Corvis no convierte al consumidor en uno con
  scope por petición, lo que reconstruiría todo un grafo de controllers en cada petición; falla al arrancar con
  `DiError::RequestScopedDependency`, que nombra al consumidor, la dependencia y la cadena, y apunta a `resolve_for`.
- Los enhancers globales (`Provider::app_guard` y similares) y los enhancers referenciados por tipo
  (`#[use_guards(T)]`) no pueden tener scope por petición, porque se crean una sola vez. Resuelve el provider con
  scope por petición dentro de ellos con `ModuleRef::resolve_for`.
- Las dependencias declaradas de los providers de clase con scope por petición se validan al arrancar como las de los
  transient (dependencia que falta, ciclos), sin crearlos. Los providers de factory no se comprueban.

Consecuencias que conviene conocer:

- **No se crean al arrancar, no se guardan y no reciben hooks del ciclo de vida.** `ModuleRef::get::<T>()` y
  `app.get::<T>()` devuelven `NotFound` para ellos.
- **Fuera de una petición no se pueden resolver.** En el handler de mensajes o el handshake de un gateway, un contexto
  de aplicación independiente, un hook del ciclo de vida, una tarea programada, un listener de eventos, una factory
  singleton o una tarea lanzada que sobrevive a la respuesta, resolver uno falla con `DiError::OutsideRequest`, que
  nombra el provider. Un `Request` que clonaste y conservaste después de la respuesta tampoco resuelve nada.
- **Un provider resuelto por primera vez en un middleware ve una petición sin parámetros de ruta.** El enrutado aún
  no ha coincidido; las extensiones son compartidas y siempre están al día, así que un valor que inserte un guard
  después sí es visible.
- **Un override de pruebas puede ocultar un error de scope.** `override_provider::<T>().use_value(..)` sustituye un
  provider con scope por petición por un valor singleton, así que un consumidor singleton arranca en la prueba y falla
  en producción.
- **Las aplicaciones sin providers con scope por petición no pagan nada por petición.**

## Datos por petición

Para datos simples no necesitas un provider: guárdalos en la propia petición. `Request` tiene un almacén de extensiones por tipo: un guard o un middleware hace `req.insert(valor)` y cualquier pieza posterior lo lee con `req.get::<T>()`. Así lo hace `AuthGuard` de Tertulia con el usuario autenticado:

```rust
#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}
```

Para recibir ese dato como parámetro del handler, implementa `FromContext` (ver [Decoradores personalizados](/es/custom-decorators)):

```rust
#[async_trait]
impl FromContext for ChatUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ChatUser::of(ctx).ok_or_else(|| UnauthorizedException::new("Unauthorized"))
    }
}

#[get("me")]
#[use_guards(AuthGuard)]
fn me(&self, user: ChatUser) -> ChatUser {
    user
}
```

En los gateways de WebSockets, el equivalente es el almacén del cliente: `client.insert(valor)` y `client.get::<T>()` (ver [Gateways](/es/websockets/gateways)).

## Lo que no está disponible

- Providers durables y estrategias de identificación de contexto.
- Propagación automática del scope a los consumidores: un singleton que depende de un provider con scope por petición es un error de arranque.
- `INQUIRER`: un provider transient no puede saber qué clase lo inyectó.
