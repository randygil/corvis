# Changelog

## Unreleased

- New setting `corvis.new.corvisPath`. When it is set, **Corvis: New Project** runs `corvis new` with `CORVIS_PATH`
  set to that folder, so the generated project depends on a local checkout of Corvis. It takes the `crates/corvis`
  folder or the repository root, must be absolute (`~` is expanded) and must contain a `Cargo.toml`; otherwise the
  extension shows an error and does not run the CLI. Until Corvis is published on crates.io, fill it in so that new
  projects build.
- The README and this changelog are now written in English.

## 0.1.0

First version.

- 57 snippets (`cv-*` / `corvis-*`) for modules, controllers and routes, providers, guards, pipes,
  interceptors, filters, middleware, gateways, events and cron, DTOs with `#[derive(Validate)]` and with
  `zod_dto!`, corvis-orm models and queries, Swagger and tests. All of them compile against Corvis.
- Commands **Corvis: Generate…** (and one per schematic), **New Project…**, **Start Application**,
  **Build Application**, **Show Project Info** and **Go to Route…**, plus the **Corvis** submenu in the
  file explorer. The CLI is looked up in the `corvis.cli.path` setting, in the `PATH`, in
  `~/.cargo/bin`, or run with `cargo run` from the Corvis repository.
- **Corvis** view with the **Modules** and **Routes** panels (method and full path, with global
  prefix and URI versioning) and navigation to the code.
- TextMate grammar injected into Rust for Corvis attributes, color decorations that work
  alongside rust-analyzer, and the full path of each route next to its attribute.
