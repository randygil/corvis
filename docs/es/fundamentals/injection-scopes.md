---
status: partial
---

# Scopes de inyección

El scope de un provider decide cuántas instancias crea el contenedor y cuándo. Corvis tiene dos:

| Scope | Instancias | Cuándo se crea |
|---|---|---|
| `Scope::Default` (singleton) | Una por módulo que registra el provider, compartida por todos sus consumidores. | Al arrancar la aplicación. |
| `Scope::Transient` | Una nueva para cada consumidor que la inyecta. | Cada vez que se resuelve. |

No existe un scope por petición. Más abajo verás cómo llevar datos de una petición sin él.

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

## Datos por petición

Pedir `#[injectable(scope = "request")]` es un error de compilación:

```text
request-scoped providers are not supported; use transient or default scope
```

La alternativa es guardar los datos de la petición en la propia petición. `Request` tiene un almacén de extensiones por tipo: un guard o un middleware hace `req.insert(valor)` y cualquier pieza posterior lo lee con `req.get::<T>()`. Así lo hace `AuthGuard` de Tertulia con el usuario autenticado:

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

- Scope `REQUEST` y la inyección de la petición actual en un provider.
- Providers durables y estrategias de identificación de contexto.
- Propagación automática del scope a los consumidores.
