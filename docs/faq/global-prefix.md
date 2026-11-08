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

Each item is compared with the **full route of each handler** (the controller route joined with the
handler route, without the prefix and without the version segment), and it does not distinguish HTTP methods.
That is how a single endpoint stays outside the prefix:

```rust
// `GET /status` and `GET /api/users`; `/api/status` and `/users` answer 404.
app.set_global_prefix_with_exclude("api", &["status"]);
```

To leave every route of a controller outside the prefix, end the item with a wildcard:

```rust
// `GET /health` and `GET /health/live`.
app.set_global_prefix_with_exclude("api", &["health/*"]);
```

::: warning An item that names only the controller excludes only its route
`"health"` leaves `GET /health` (a `#[get]` in `#[controller("health")]`) outside the prefix, but not
`GET /api/health/live`. Earlier versions excluded every route of the controller with that item; write
`"health/*"` for that.
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
