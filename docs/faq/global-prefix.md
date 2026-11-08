---
status: documented
---

# Global prefix

To hang all routes under a common prefix (for example `/api`), call
`set_global_prefix` before `listen`:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.set_global_prefix("api");
app.listen(3000).await
```

With that, a `#[controller("cats")]` with `#[get(":id")]` responds at `GET /api/cats/:id`. If you also
enable [URI versioning](/http/versioning), the version goes after the prefix:
`/api/v1/cats/:id`.

## Excluding controllers from the prefix

`set_global_prefix_with_exclude` takes a list of routes that stay outside the prefix:

```rust
app.set_global_prefix_with_exclude("api/v1", &["health"]);
```

::: warning The exclusion applies per controller route
Each item in the list is compared with the **controller route** (the argument of
`#[controller]`), not with the full route of each handler, and it does not distinguish HTTP methods. The example
leaves a `#[controller("health")]` without the prefix, but not a `#[get("health")]` handler inside a
controller without a route. If you need to exclude a single endpoint, move it to its own controller.
:::

Patterns accept wildcards (`"admin/*"`), with the same rules as `for_routes` in
[middleware](/middleware).

## What the prefix does not cover

The prefix only affects controller routes. It does not change:

- the routes of `#[websocket_gateway(path = "...")]`;
- files served with `corvis::serve_static` or `ServeStaticModule`;
- the routes mounted by `SwaggerModule::setup` or Scalar, which receive their own route (in Swagger,
  `SwaggerCustomOptions { use_global_prefix: true, .. }` prepends it; see
  [OpenAPI](/openapi/introduction)).

`app.global_prefix()` returns the configured prefix.
