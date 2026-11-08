---
status: unavailable
---

# OpenTelemetry

There is no OpenTelemetry integration: Corvis does not export traces, metrics or logs in that format.

## Alternative

You can initialize the OpenTelemetry crates for Rust in your `main` and create spans from a global [interceptor](/interceptors), which has access to the request (`ctx.request()`) and to the handler result. Corvis does not do this for you and does not propagate the context.
