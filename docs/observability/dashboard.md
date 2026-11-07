---
status: unavailable
---

# Dashboard

Corvis does not include a web dashboard to view requests, errors or application metrics.

## Alternative

Send the JSON logs (`ConsoleLogger::new().json(true)`) to the visualization tool you already use. For instance health, expose an endpoint with [Health checks](/reliability/terminus).
