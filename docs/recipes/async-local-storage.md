---
status: partial
---

# Async local storage

Corvis **does not offer implicit per-request storage**, that is, data that any function called during a request can read without receiving it as a parameter. The closest equivalent is a request-scoped provider that handlers, guards and interceptors take with a parameter.

## Alternatives

**A request-scoped provider.** Declare a provider with `#[injectable(scope = "request")]`: one instance per request, shared by everything that asks for it during that request. It can inject the current `Request`. Take it with `Scoped<T>` (see [Injection scopes](/fundamentals/injection-scopes)):

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

In a guard or interceptor: `Scoped::<RequestContext>::from_context(ctx).await?`. `examples/cats-app` does this with a request id.
It is still explicit: a singleton service cannot inject it (that is a startup error), so the handler hands the context to the services it calls.

**Request extensions.** A middleware, guard or interceptor stores a value, and any later piece of the same request reads it by its type:

```rust
#[derive(Clone)]
pub struct RequestId(pub String);

app.use_fn(|req: Request, next: Next| async move {
    let id = req.header("x-request-id").map(str::to_string).unwrap_or_else(|| "no-id".into());
    req.insert(RequestId(id));
    next.run(req).await
});
```

```rust
// In a guard or interceptor:
let id = ctx.request().get::<RequestId>();
```

**A `FromContext` parameter.** To read that value in a handler without repeating code, implement `FromContext` and use it as a parameter without an attribute:

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

`examples/tertulia` does this with the authenticated user (`ChatUser`). See [Custom decorators](/custom-decorators).

The difference from implicit storage is that services cannot read the value on their own: the handler passes it to them as an argument. Implicit access without a parameter (an `AsyncLocalStorage` equivalent) is not available.
