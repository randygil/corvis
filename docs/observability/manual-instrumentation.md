---
status: unavailable
---

# Manual instrumentation

Corvis has no API for creating your own spans or metrics inside a service.

## Alternative

Record important events with `Logger` (in JSON mode so they can be processed) and measure durations with `std::time::Instant` inside your services or in an [interceptor](/interceptors). If you need real spans, use a Rust tracing crate in your code; Corvis does not integrate it for you.
