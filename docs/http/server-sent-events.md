---
status: documented
---

# Server-Sent Events

Server-Sent Events (SSE) is a simple way to push events from the server to the browser over
a regular HTTP connection. Unlike a WebSocket, the channel goes in one direction only, which makes it
ideal for dashboards, notifications or task progress.

## Declaring an SSE endpoint

Mark the handler with `#[sse("path")]` (it is a `GET` route) and return an `Sse` built from
a `Stream` of `MessageEvent`:

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
    /// `GET /api/stats/stream`: a `stats` event on connect and then every `STATS_INTERVAL_MS`.
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

The example is from `examples/tertulia` (`src/stats/mod.rs`). In `examples/cats-app` there is another one that
ends after five events: when the stream returns `None`, the response is closed.

## `MessageEvent`

| Method | Field on the wire |
|---|---|
| `MessageEvent::new(data)` | `data:` (any `Serialize` value; a string is sent as is, anything else as JSON) |
| `.event("name")` | `event:`; the client listens for it with `addEventListener("name", ...)` |
| `.id("42")` | `id:`; the browser resends it in `Last-Event-ID` when reconnecting |
| `.retry(ms)` | `retry:`; suggested reconnection time |

For safety, `data` is split into one `data:` line for each line break (`\r\n`, `\r` or `\n`), and line
breaks are removed from `event` and `id`. User text cannot inject fields or events.

## Headers

The response carries `Content-Type: text/event-stream`, `Cache-Control: no-cache`,
`Connection: keep-alive` and `X-Accel-Buffering: no` (so nginx does not accumulate events).

An ongoing SSE response does not count as inactivity for `keep_alive_timeout`: the connection
stays open as long as the stream produces events.

## On the client

```js
const source = new EventSource('/api/stats/stream')
source.addEventListener('stats', (e) => {
  const stats = JSON.parse(e.data)
  console.log(stats.connections)
})
```

## Guards and interceptors

An `#[sse]` route is just another route: it goes through the usual middleware, guards and interceptors. Keep in
mind that `EventSource` does not allow sending headers, so a guard that requires
`Authorization: Bearer` will reject the connection from the browser. Use a cookie or a token in the
query for those cases.

For two-way communication, see [WebSockets](/websockets/gateways).
