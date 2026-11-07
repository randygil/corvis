---
status: documentado
---

# Server-Sent Events

Server-Sent Events (SSE) es una forma sencilla de empujar eventos del servidor al navegador sobre
una conexión HTTP normal. A diferencia de un WebSocket, el canal va en un solo sentido, lo que lo
hace ideal para paneles, notificaciones o progreso de tareas.

## Declarar un endpoint SSE

Marca el handler con `#[sse("ruta")]` (es una ruta `GET`) y devuelve un `Sse` construido a partir de
un `Stream` de `MessageEvent`:

```rust
use std::time::Duration;
use corvis::prelude::*;

#[controller("stats")]
pub struct StatsController {
    chat: Arc<ChatGateway>,
    rooms: Arc<RoomsService>,
    config: Arc<ConfigService>,
}

#[routes]
impl StatsController {
    /// `GET /api/stats/stream`: un evento `stats` al conectarse y luego cada `STATS_INTERVAL_MS`.
    #[sse("stream")]
    fn stream(&self) -> Sse {
        let every = Duration::from_millis(self.config.get_or("STATS_INTERVAL_MS", 2000u64));
        let state = (self.chat.clone(), self.rooms.clone(), 0u64);
        let events = corvis::futures::stream::unfold(state, move |(chat, rooms, n)| async move {
            if n > 0 {
                corvis::tokio::time::sleep(every).await;
            }
            let data = snapshot(&chat, &rooms).await;
            Some((MessageEvent::new(data).event("stats").id(n.to_string()), (chat, rooms, n + 1)))
        });
        Sse::new(events)
    }
}
```

El ejemplo es de `examples/tertulia` (`src/stats/mod.rs`). En `examples/cats-app` hay otro que
termina después de cinco eventos: cuando el stream devuelve `None`, la respuesta se cierra.

## `MessageEvent`

| Método | Campo en el cable |
|---|---|
| `MessageEvent::new(data)` | `data:` (cualquier valor `Serialize`; un string se envía tal cual, lo demás como JSON) |
| `.event("nombre")` | `event:`; el cliente lo escucha con `addEventListener("nombre", ...)` |
| `.id("42")` | `id:`; el navegador lo reenvía en `Last-Event-ID` al reconectar |
| `.retry(ms)` | `retry:`; tiempo de reconexión sugerido |

Por seguridad, `data` se divide en una línea `data:` por cada salto (`\r\n`, `\r` o `\n`) y a `event`
e `id` se les quitan los saltos de línea. Un texto del usuario no puede inyectar campos ni eventos.

## Cabeceras

La respuesta lleva `Content-Type: text/event-stream`, `Cache-Control: no-cache`,
`Connection: keep-alive` y `X-Accel-Buffering: no` (para que nginx no acumule los eventos).

Una respuesta SSE en curso no cuenta como inactividad para `keep_alive_timeout`: la conexión se
mantiene abierta mientras el stream produzca eventos.

## En el cliente

```js
const source = new EventSource('/api/stats/stream')
source.addEventListener('stats', (e) => {
  const stats = JSON.parse(e.data)
  console.log(stats.connections)
})
```

## Guards e interceptors

Una ruta `#[sse]` es una ruta más: pasa por los middleware, guards e interceptors de siempre. Ten en
cuenta que `EventSource` no permite enviar cabeceras, así que un guard que exige
`Authorization: Bearer` rechazará la conexión desde el navegador. Usa una cookie o un token en la
query para esos casos.

Para una comunicación en los dos sentidos, mira [WebSockets](/websockets/gateways).
