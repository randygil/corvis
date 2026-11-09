---
status: partial
---

# Async local storage

Corvis **no ofrece un almacenamiento por petición implícito**, es decir, datos que cualquier función llamada durante una petición puede leer sin recibirlos por parámetro. El equivalente más cercano es un provider con scope por petición que los handlers, guards e interceptors reciben por parámetro.

## Alternativas

**Un provider con scope por petición.** Declara un provider con `#[injectable(scope = "request")]`: una instancia por petición, compartida por todo lo que la pida durante esa petición. Puede inyectar la `Request` actual. Recíbelo con `Scoped<T>` (ver [Scopes de inyección](/es/fundamentals/injection-scopes)):

```rust
#[injectable(scope = "request")]
pub struct RequestContext {
    req: Request,
}

impl RequestContext {
    pub fn request_id(&self) -> String {
        self.req.header("x-request-id").unwrap_or("no-id").to_string()
    }
}

#[get]
fn list(&self, ctx: Scoped<RequestContext>) -> Value { /* ctx.request_id() */ }
```

En un guard o interceptor: `Scoped::<RequestContext>::from_context(ctx).await?`. `examples/cats-app` lo hace con un id de petición.
Sigue siendo explícito: un servicio singleton no puede inyectarlo (es un error de arranque), así que el handler entrega el contexto a los servicios que llama.

**Las extensiones de la request.** Un middleware, guard o interceptor guarda un valor y cualquier pieza posterior de la misma petición lo lee por su tipo:

```rust
#[derive(Clone)]
pub struct RequestId(pub String);

app.use_fn(|req: Request, next: Next| async move {
    let id = req.header("x-request-id").map(str::to_string).unwrap_or_else(|| "sin-id".into());
    req.insert(RequestId(id));
    next.run(req).await
});
```

```rust
// En un guard o interceptor:
let id = ctx.request().get::<RequestId>();
```

**Un parámetro `FromContext`.** Para leer ese valor en un handler sin repetir código, implementa `FromContext` y úsalo como parámetro sin atributo:

```rust
#[async_trait]
impl FromContext for RequestId {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ctx.request().get::<RequestId>().ok_or_else(|| BadRequestException::new("missing request id"))
    }
}

#[get]
fn list(&self, request_id: RequestId) -> Value { /* ... */ }
```

Así lo hace `examples/tertulia` con el usuario autenticado (`ChatUser`). Ver [Decoradores personalizados](/es/custom-decorators).

La diferencia con un almacenamiento implícito es que los servicios no pueden leer el valor por su cuenta: el handler se lo pasa como argumento. El acceso implícito sin parámetro (un equivalente de `AsyncLocalStorage`) no está disponible.
