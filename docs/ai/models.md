---
status: unavailable
---

# Models

Corvis does not include a language model integration: there is no AI module, provider clients, tools, agents or embeddings.

## Alternative

A client for any provider can be registered as a provider with an async factory (`Provider::use_factory`) and read its key with `ConfigService`; see [Custom providers](/fundamentals/custom-providers) and [Configuration](/application/configuration). Corvis does not supply that client.
