---
status: unavailable
---

# Socket activation

Corvis **does not support socket activation** (receiving an already open socket from systemd or another supervisor). `app.listen` always opens its own socket from an address: a port (`3000`), a string (`"0.0.0.0:3000"`) or a `SocketAddr`.

## Alternatives

- **Orderly restarts**: with `app.enable_shutdown_hooks()`, on receiving `SIGINT` or `SIGTERM` the server stops accepting connections and runs the shutdown lifecycle hooks. Combine it with a load balancer or a reverse proxy that spreads traffic between two instances while one restarts. See [Deployment](/deployment).
- **Privileged ports**: instead of having the process open port 80 or 443, listen on a high port and leave the public port to the reverse proxy.
