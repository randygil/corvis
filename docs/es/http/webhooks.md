---
status: partial
---

# Webhooks

Un webhook es una petición que un servicio externo (una pasarela de pagos, un repositorio de código,
un proveedor de correo) envía a tu API cuando ocurre algo. Corvis no trae un módulo específico para
webhooks, pero tiene las piezas para recibirlos bien: acceso al cuerpo exacto que llegó, guards para
verificar la firma y providers para procesar el evento.

## Verificar la firma con el cuerpo original

Casi todos los proveedores firman el cuerpo con HMAC y envían la firma en una cabecera. La firma se
calcula sobre los **bytes originales**, no sobre el JSON ya parseado (volver a serializarlo puede
cambiar espacios u orden de claves). `req.raw_body()` devuelve esos bytes, también cuando el cuerpo
es JSON y el body parser ya lo parseó.

Un guard es el lugar natural para la verificación. Este ejemplo usa los crates `hmac`, `sha2` y `hex`
(dependencias de tu aplicación, no de Corvis):

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
        // `verify_slice` compara en tiempo constante.
        Ok(mac.verify_slice(&signature).is_ok())
    }
}
```

Si la firma no coincide, el guard devuelve `false` y el proveedor recibe `403`. El nombre de la
cabecera y el formato (hex, base64, prefijo `sha256=`) dependen de cada proveedor.

## El controlador

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
        // Responde rápido y procesa después.
        self.events.emit("payment.webhook", payload);
        json!({ "received": true })
    }
}
```

- `#[http_code(200)]`: muchos proveedores esperan `200` y no el `201` que un `POST` devuelve por
  defecto.
- Si tienes guards globales de autenticación (por ejemplo un `AuthGuard` con JWT), marca esta ruta
  como pública con la metadata que ese guard consulte: el proveedor no envía tu token.
- `EventEmitter::emit` entrega el evento a los handlers `#[on_event]`, que se ejecutan aparte. Así el
  endpoint responde en milisegundos y los reintentos del proveedor no se disparan por timeouts. Ver
  [Eventos](/es/application/events).

## Tipos de contenido distintos de JSON

Si el proveedor envía otro `Content-Type` (por ejemplo `text/plain` o `application/xml`), el body
parser no lo parsea: `#[body]` recibe `null` y el contenido sigue en `req.raw_body()`. Recibe la
request completa con `#[req] req: Request`. Ver [Cuerpo sin procesar](/es/faq/raw-body).

## Lo que no incluye Corvis

- No hay deduplicación de eventos ni registro de entregas. Guarda el identificador del evento (los
  proveedores suelen enviarlo) y descarta los repetidos; ver
  [Claves de idempotencia](/es/reliability/idempotency).
- No hay un cliente para *enviar* webhooks a terceros.
