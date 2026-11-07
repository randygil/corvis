---
status: no-disponible
---

# Commander

Corvis **no tiene un módulo para escribir aplicaciones de línea de comandos** (comandos, opciones y preguntas declarados con atributos sobre providers).

## Alternativa: la aplicación sin servidor + un parser de argumentos

Una herramienta de consola puede reutilizar tus módulos y servicios creando la aplicación sin abrir el servidor HTTP. El análisis de argumentos lo hace `std::env::args` o cualquier crate de Rust dedicado a eso:

```rust
// src/bin/admin.rs
use corvis::prelude::*;
use mi_api::{AppModule, UsersService};

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.init().await?;
    let users = app.get::<UsersService>()?;
    match args.first().map(String::as_str) {
        Some("count") => println!("{}", users.count().await?),
        _ => eprintln!("uso: admin count"),
    }
    app.close().await
}
```

```bash
cargo run --bin admin -- count
```

Para que `src/bin/admin.rs` pueda importar `AppModule`, el proyecto necesita un `src/lib.rs` que exponga los módulos (como hace `examples/tertulia`). Los providers, la configuración y los lifecycle hooks funcionan igual que en el servidor. Ver [Aplicaciones standalone](/es/standalone-applications).
