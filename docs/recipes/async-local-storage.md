---
status: unavailable
---

# Async local storage

Corvis **does not offer implicit per-request storage**, that is, data that any function called during a request can read without receiving it as a parameter. There is no request-scoped injection either: providers are singleton or transient (see [Injection scopes](/fundamentals/injection-scopes)).

## Alternatives

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

The difference from implicit storage is that services cannot read the value on their own: the handler passes it to them as an argument.
