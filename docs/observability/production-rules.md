---
status: unavailable
---

# Production rules

Corvis has no rules or alerts system for the behavior of the application in production.

## Alternative

Alerts are configured in the tool that receives your logs or queries your health endpoint. From the application, the most useful thing is to emit JSON logs with clear context (see [Logging](/application/logger)) and expose a [Health checks](/reliability/terminus) endpoint.
