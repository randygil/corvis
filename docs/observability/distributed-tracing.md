---
status: unavailable
---

# Distributed tracing

Corvis does not propagate trace context between services (headers such as `traceparent`) and does not create per-request spans.

## Alternative

A [middleware](/middleware) or an [interceptor](/interceptors) can read the incoming trace header with `req.header("traceparent")` and store it on the request with `req.insert(value)`, so handlers can include it in their logs and in outgoing calls.
