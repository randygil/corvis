---
status: no-disponible
---

# Correo

Corvis no incluye un módulo para enviar correo (transportes SMTP, plantillas o colas de envío).

## Alternativa

Usa un crate de correo de Rust (por ejemplo `lettre`) dentro de un provider propio y configúralo con `ConfigService`:

```rust
#[injectable]
pub struct MailService {
    config: Arc<ConfigService>,
}

impl MailService {
    pub async fn send_welcome(&self, to: &str) -> corvis::Result<()> {
        let host: String = self.config.try_get("SMTP_HOST")?;
        // Construye y envía el mensaje con el crate que elijas.
        Ok(())
    }
}
```

Para no bloquear la respuesta HTTP, dispara el envío desde un listener de eventos (ver [Eventos](/application/events)).
