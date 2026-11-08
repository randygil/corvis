---
status: documented
---

# Versioning

Versioning lets you publish several versions of the same route at once, for example `/v1/cats/1`
and `/v2/cats/1`, to change a contract without breaking existing clients. You enable it once in
the application and then each controller or handler declares its version.

## Enabling it

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.set_global_prefix("api");
app.enable_versioning(VersioningOptions::uri().default_version("1"));
```

There are three strategies:

| Strategy | Constructor | The version travels in |
|---|---|---|
| URI | `VersioningOptions::uri()` | The path: `/api/v1/cats` (`v` prefix by default, configurable with `.prefix("version-")`) |
| Header | `VersioningOptions::header("Custom-Header")` | A header: `Custom-Header: 2` |
| Media type | `VersioningOptions::media_type("v=")` | The `Accept` header: `Accept: application/json;v=2` |

`.default_version(...)` assigns a version to the controllers and routes that do not declare any.

There is no custom strategy (a function that extracts the version from the request).

## Declaring versions

On the controller, with the `version` option of `#[controller]` or with `#[version]` below it:

```rust
#[controller(path = "cats", version = "1")]
pub struct CatsController { /* ... */ }
```

On a handler, with `#[version]`. A handler without a version inherits the controller's and, if that has
none either, the default version:

```rust
#[routes]
impl CatsController {
    #[get(":id")]
    fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
        self.cats_service.find_one(id)
    }

    #[get(":id")]
    #[version("2")]
    fn find_one_v2(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Value> {
        let cat = self.cats_service.find_one(id)?;
        Ok(json!({ "cat": cat, "apiVersion": 2 }))
    }
}
```

With `VersioningOptions::uri().default_version("1")`, the previous example publishes
`GET /api/v1/cats/:id` and `GET /api/v2/cats/:id`.

A route can respond to several versions: `#[version("1", "2")]` or `#[version(["1", "2"])]`.

## Version-neutral routes

`VERSION_NEUTRAL` marks a route that does not depend on the version. With the URI strategy it is published without
the version segment; with header or media type it responds to any requested version and to
requests that indicate none.

```rust
#[get("health")]
#[version(VERSION_NEUTRAL)]
async fn health(&self) -> HttpResult<Value> {
    // GET /api/health
}
```

## How the route is chosen with header or media type

With these strategies all versions share the same path, and Corvis chooses the handler on each
request:

- If the request indicates a version, the route that declares it wins; if there is none, the
  `VERSION_NEUTRAL` route.
- If it indicates none, only routes without a version or with `VERSION_NEUTRAL` respond. Keep in mind
  that `default_version` assigns a version to every route that does not declare another, so, with a
  default version, a request without the header gets `404` except on neutral routes.
- If none matches, the response is `404 Cannot GET /path`.

## Startup logs

When registering routes, the log indicates the version of each one:

```text
LOG [RoutesResolver] CatsController {/api/v1/cats}: (version: 1)
LOG [RouterExplorer] Mapped {/api/v2/cats/:id, GET} route (version: 2)
```
