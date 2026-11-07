---
status: unavailable
---

# Visión general

Corvis no tiene una herramienta visual para explorar el grafo de módulos o las dependencias de una aplicación en ejecución.

## Alternativas

- **Introspección desde código.** `app.modules_info()` devuelve los módulos, sus imports, si son globales y sus controladores con las rutas, sin necesidad de `app.init()` (ver [Módulos](/es/modules#inspeccionar-el-grafo)).
- **Logs de arranque.** Al arrancar, Corvis registra una línea por módulo (`InstanceLoader`) y una por ruta montada (`RoutesResolver`, `RouterExplorer`).
- **Extensión de VS Code.** El repositorio incluye en `editors/vscode` una extensión con un explorador de módulos, controladores y rutas, snippets de los atributos y comandos que ejecutan la CLI `corvis`.
