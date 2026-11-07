---
status: partial
---

# Webhooks

A webhook is a request that an external service (a payment gateway, a code repository,
an email provider) sends to your API when something happens. Corvis does not ship a dedicated module for
webhooks, but it has the pieces to receive them properly: access to the exact body that arrived, guards to
verify the signature and providers to process the event.

## Verifying the signature with the original body

Almost all providers sign the body with HMAC and send the signature in a header. The signature is
computed over the **original bytes**, not over the already parsed JSON (serializing it again can
change whitespace or key order). `req.raw_body()` returns those bytes, also when the body
is JSON and the body parser already parsed it.

A guard is the natural place for verification. This example uses the `hmac`, `sha2` and `hex` crates
(dependencies of your application, not of Corvis):

```toml
[dependencies]
hmac = "0.12"
sha2 = "0.10"
hex = "0.4"
```

```rust
use corvis::prelude::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[injectable]
pub struct WebhookSignatureGuard {
    config: Arc<ConfigService>,
}

#[async_trait]
impl CanActivate for WebhookSignatureGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let req = ctx.request();
        let secret: String = self.config.get("WEBHOOK_SECRET").ok_or(UnauthorizedException)?;
        let signature = req
            .header("x-signature")
            .and_then(|s| hex::decode(s).ok())
            .ok_or_else(|| UnauthorizedException::new("Missing signature"))?;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| UnauthorizedException)?;
        mac.update(req.raw_body());
        // `verify_slice` compares in constant time.
        Ok(mac.verify_slice(&signature).is_ok())
    }
}
```

If the signature does not match, the guard returns `false` and the provider receives `403`. The header
name and the format (hex, base64, `sha256=` prefix) depend on each provider.

## The controller

```rust
#[controller("webhooks")]
pub struct WebhooksController {
    events: Arc<EventEmitter>,
}

#[routes]
impl WebhooksController {
    #[post("payments")]
    #[http_code(200)]
    #[use_guards(WebhookSignatureGuard)]
    fn payments(&self, #[body] payload: Value) -> Value {
        // Respond quickly and process later.
        self.events.emit("payment.webhook", payload);
        json!({ "received": true })
    }
}
```

- `#[http_code(200)]`: many providers expect `200` and not the `201` that a `POST` returns by
  default.
- If you have global authentication guards (for example an `AuthGuard` with JWT), mark this route
  as public with the metadata that guard checks: the provider does not send your token.
- `EventEmitter::emit` delivers the event to the `#[on_event]` handlers, which run separately. This way the
  endpoint responds in milliseconds and the provider's retries are not triggered by timeouts. See
  [Events](/application/events).

## Content types other than JSON

If the provider sends another `Content-Type` (for example `text/plain` or `application/xml`), the body
parser does not parse it: `#[body]` receives `null` and the content remains in `req.raw_body()`. Receive the
full request with `#[req] req: Request`. See [Raw body](/faq/raw-body).

## What Corvis does not include

- There is no event deduplication or delivery log. Store the event identifier (providers
  usually send it) and discard repeated ones; see
  [Idempotency keys](/reliability/idempotency).
- There is no client for *sending* webhooks to third parties.
