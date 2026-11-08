---
status: unavailable
---

# Mail

Corvis does not include a module for sending mail (SMTP transports, templates or sending queues).

## Alternative

Use a Rust mail crate (for example `lettre`) inside a provider of your own and configure it with `ConfigService`:

```rust
#[injectable]
pub struct MailService {
    config: Arc<ConfigService>,
}

impl MailService {
    pub async fn send_welcome(&self, to: &str) -> corvis::Result<()> {
        let host: String = self.config.try_get("SMTP_HOST")?;
        // Build and send the message with the crate you choose.
        Ok(())
    }
}
```

To avoid blocking the HTTP response, trigger the send from an event listener (see [Events](/application/events)).
