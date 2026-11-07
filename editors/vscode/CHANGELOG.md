# Cambios

## 0.1.0

Primera versión.

- 57 snippets (`cv-*` / `corvis-*`) para módulos, controladores y rutas, providers, guards, pipes,
  interceptors, filtros, middleware, gateways, eventos y cron, DTOs con `#[derive(Validate)]` y con
  `zod_dto!`, modelos y consultas de corvis-orm, Swagger y tests. Todos se compilan contra Corvis.
- Comandos **Corvis: Generate…** (y uno por schematic), **New Project…**, **Start Application**,
  **Build Application**, **Show Project Info** y **Go to Route…**, más el submenú **Corvis** en el
  explorador de archivos. La CLI se busca en el ajuste `corvis.cli.path`, en el `PATH`, en
  `~/.cargo/bin` o se ejecuta con `cargo run` desde el repositorio de Corvis.
- Vista **Corvis** con los paneles **Modules** y **Routes** (método y path completo, con prefijo
  global y versionado por URI) y navegación al código.
- Gramática TextMate inyectada en Rust para los atributos de Corvis, decoraciones de color que
  funcionan junto con rust-analyzer y el path completo de cada ruta junto a su atributo.
