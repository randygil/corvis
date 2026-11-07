---
status: unavailable
---

# Lazy-loading modules

Corvis has no lazy loading of modules: there is no loader that adds a module to the container after startup. The whole module graph is walked and instantiated in `CorvisFactory::create`, before the first request is served.

In other environments lazy loading exists mostly to reduce the cold start time of serverless functions, where loading code is expensive. In a Rust binary all the code is already compiled and linked, so the remaining cost is that of **building** the providers.

## Alternatives

- **Build on demand.** If a provider is expensive to create and only used sometimes, don't register it as a singleton: mark it transient and resolve it with `module_ref.resolve::<T>().await`, or build it when needed with `module_ref.create::<T>().await`. See [Module reference](/fundamentals/module-ref).
- **Deferred initialization inside the provider.** A `tokio::sync::OnceCell` field (with `#[default]`) lets you create the expensive resource the first time it is used, keeping a single registered provider.
- **Choosing modules at compile time.** Cargo *features* and `#[cfg(feature = "...")]` let you include or exclude entire modules from the binary.
- **Separate contexts.** `CorvisFactory::create_application_context::<AnotherModule>()` creates an independent container (without an HTTP server) from another root module, for example for a one-off task. See [Standalone applications](/standalone-applications).
