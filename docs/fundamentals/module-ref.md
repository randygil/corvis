---
status: documented
---

# Module reference

`ModuleRef` gives programmatic access to the injection container from inside the application. It is useful when you don't know which provider you need until the moment you use it, to resolve transient providers on demand, to create instances of types that aren't registered, and to break [circular dependencies](/fundamentals/circular-dependency).

## Getting a `ModuleRef`

Declare a field of type `ModuleRef` in any `#[injectable]`; it takes no attribute:

```rust
#[injectable]
pub struct ExportService {
    module_ref: ModuleRef,
}
```

The `ModuleRef` is located in the **module that owns the provider**: lookups that respect encapsulation start from there.

Other ways to get it:

- In a factory: `ctx.module_ref()` (see [Custom providers](/fundamentals/custom-providers#factories-use-factory)).
- From the application: `app.module_ref()`, located in the root module. `app.get::<T>()` and `app.get_token::<T>(..)` are shortcuts for `app.module_ref().get...`.

## Methods

| Method | Returns | Looks up |
|---|---|---|
| `get::<T>()` | `Result<Arc<T>, DiError>` | An **existing** instance of `T`: first with the module's visibility and, if it doesn't find it, across the whole application. |
| `get_token::<T>(token)` | `Result<Arc<T>, DiError>` | The same, by token (`"TOKEN"` or `Token`). |
| `get_token_in_context::<T>(token)` | `Result<Arc<T>, DiError>` | Only what is visible from the module: its providers, the exports of its imports, and the global modules. It is the same as what an `Arc<T>` field would see. |
| `get_strict::<T>()` | `Result<Arc<T>, DiError>` | Only the providers registered in the module itself. |
| `resolve::<T>().await` | `Result<Arc<T>, DiError>` | Resolves like injection; for a transient provider it creates a new instance. |
| `create::<T>().await` | `Result<T, DiError>` | Builds an `#[injectable]` that is **not** registered, injecting its fields from the module. It returns the value, not an `Arc`. |
| `module_name()` | `String` | The module's name. |

The `get*` methods are synchronous and build nothing: if the instance doesn't exist (for example, a transient provider, or a token nobody registered), they return `DiError::NotFound`. If the token exists but with another type, `DiError::TypeMismatch`.

::: warning `get` does not respect encapsulation
Like the non-strict option in other containers, `get` ends up looking across the **whole** application, including private providers of other modules. If you want the behavior of field injection, use `get_token_in_context`; if you want to limit yourself to the module itself, `get_strict`.
:::

## Examples

Choosing a provider by name at runtime:

```rust
#[injectable]
pub struct ExportService {
    module_ref: ModuleRef,
}

impl ExportService {
    pub fn exporter(&self, format: &str) -> HttpResult<Arc<dyn Exporter>> {
        self.module_ref
            .get_token_in_context::<dyn Exporter>(format!("EXPORTER:{format}"))
            .map_err(|_| BadRequestException::new(format!("Unknown format {format}")))
    }
}

// providers = [
//     Provider::use_class_as::<CsvExporter, dyn Exporter>("EXPORTER:csv", |e| e),
//     Provider::use_class_as::<JsonExporter, dyn Exporter>("EXPORTER:json", |e| e),
//     ExportService,
// ]
```

A new instance of a transient provider on each operation:

```rust
pub async fn run(&self) -> corvis::Result<()> {
    let timer = self.module_ref.resolve::<OperationTimer>().await?;
    // ...
    Ok(())
}
```

Building an unregistered type with its dependencies injected:

```rust
#[injectable]
pub struct ReportBuilder {
    rooms: Arc<RoomsService>,
    #[default]
    lines: Vec<String>,
}

let builder: ReportBuilder = self.module_ref.create::<ReportBuilder>().await?;
```

Each call to `create` produces a new value; the container doesn't store it.

## In tests

`TestingModule` and the test application expose `get` the same way, so you can get a provider (or its double) to make assertions. See [Testing](/fundamentals/testing).
