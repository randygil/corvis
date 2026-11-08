---
status: unavailable
---

# Choosing an APM

Corvis has no integrations with APM providers, so there is no specific recommendation.

## Alternative

Any APM that accepts JSON logs on standard output works with `ConsoleLogger::new().json(true)`. For traces, choose one that has an SDK for Rust and wire it up from your code (see [OpenTelemetry](/observability/opentelemetry)).
