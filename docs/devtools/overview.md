---
status: unavailable
---

# Overview

Corvis has no visual tool for exploring the module graph or the dependencies of a running application.

## Alternatives

- **Introspection from code.** `app.modules_info()` returns the modules, their imports, whether they are global and their controllers with the routes, without needing `app.init()` (see [Modules](/modules#inspecting-the-graph)).
- **Startup logs.** On startup, Corvis logs one line per module (`InstanceLoader`) and one per mounted route (`RoutesResolver`, `RouterExplorer`).
- **VS Code extension.** The repository includes in `editors/vscode` an extension with an explorer of modules, controllers and routes, attribute snippets and commands that run the `corvis` CLI.
