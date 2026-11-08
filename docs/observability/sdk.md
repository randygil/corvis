---
status: unavailable
---

# SDK

Corvis has no observability SDK that instruments the application automatically (traces, metrics and errors sent to a service).

## Alternative

Combine the JSON logs from `ConsoleLogger`, a global interceptor that measures each request, and a global exception filter (see [Overview](/observability/overview)).
