---
status: no-disponible
---

# REPL

Corvis **no incluye un REPL** para inspeccionar los providers de una aplicación en marcha o llamar a sus métodos de forma interactiva. Rust es un lenguaje compilado y no hay un intérprete donde evaluar expresiones contra la aplicación.

## Alternativas

**Un binario o test de exploración.** Crea la aplicación sin servidor HTTP y usa el contenedor desde código:

```rust
// src/bin/explore.rs
use corvis::prelude::*;
use mi_api::{AppModule, CatsService};

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.init().await?;
    let cats = app.get::<CatsService>()?;
    println!("{:?}", cats.find_all());
    app.close().await
}
```

```bash
cargo run --bin explore
```

Ver [Aplicaciones standalone](/standalone-applications).

**Los tests.** `Test::create_testing_module()` compila un módulo, permite reemplazar providers y obtener cualquiera con `module.get::<T>()`. Es la forma más rápida de probar un servicio aislado; ver [Testing](/fundamentals/testing).
