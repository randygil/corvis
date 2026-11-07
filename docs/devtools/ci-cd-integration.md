---
status: no-disponible
---

# Integración CI/CD

No hay una integración de Corvis con sistemas de CI/CD (por ejemplo, para publicar o comparar el grafo de la aplicación en cada cambio).

## Alternativa

Un pipeline de un proyecto Corvis es un pipeline de Cargo:

```bash
cargo build --release
cargo test --workspace
```

El repositorio de Corvis usa además `cargo deny check` (configurado en `deny.toml`: avisos de seguridad, licencias permitidas y fuentes) y `cargo audit` (`.cargo/audit.toml`); puedes reutilizar esa configuración. Las pruebas de extremo a extremo se escriben con las utilidades de [Testing](/fundamentals/testing) y no necesitan levantar un puerto.
