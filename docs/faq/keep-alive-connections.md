---
status: documented
---

# Keep-alive connections

With HTTP/1.1, a client can reuse the same TCP connection for several requests
(*keep-alive*). Corvis always supports it and closes idle connections after
`keep_alive_timeout`, which defaults to 5 seconds, the same value as Node.js.

## Changing the timeout

```rust
use std::time::Duration;
use corvis::ServerOptions;

let mut app = CorvisFactory::create::<AppModule>().await?;
app.set_server_options(ServerOptions::new().keep_alive_timeout(Duration::from_secs(65)));
app.listen(3000).await
```

`.keep_alive_timeout(None)` disables the limit.

## Behind a load balancer

If a load balancer or proxy reuses connections to the application, Corvis's idle timeout
must be **greater** than the load balancer's. Otherwise, Corvis may close a connection just
as the load balancer sends a request over it, and the client gets a sporadic `502`. For
example, with a load balancer that closes at 60 seconds, use 65.

## What counts as idle

Only the time between requests on the same connection. A request in progress (a slow handler or a
[Server-Sent Events](/http/server-sent-events) response) does not count as idle.

The server's other timeouts are independent:

| Option | Default | What it limits |
|---|---|---|
| `headers_timeout` | 60 s | Time to receive the headers; protects against clients that send them drip by drip |
| `request_timeout` | 300 s | Time to receive the body; responds `408` |
| `keep_alive_timeout` | 5 s | Inactivity between requests |

See [HTTP adapters](/http/performance) for the rest of `ServerOptions`.
