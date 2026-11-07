---
status: no-disponible
---

# Async local storage

Corvis **no ofrece un almacenamiento por petición implícito**, es decir, datos que cualquier función llamada durante una petición puede leer sin recibirlos por parámetro. Tampoco existe el scope de inyección por petición: los providers son singleton o transient (ver [Scopes de inyección](/es/fundamentals/injection-scopes)).

## Alternativas

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

La diferencia con un almacenamiento implícito es que los servicios no pueden leer el valor por su cuenta: el handler se lo pasa como argumento.
