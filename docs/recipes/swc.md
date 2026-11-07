---
status: no-disponible
---

# SWC (compilador rápido)

Esta receta no aplica a Corvis. En los frameworks de TypeScript, SWC reemplaza al compilador de TypeScript para acelerar el paso de TypeScript a JavaScript. Una aplicación de Corvis es Rust: la compila `rustc` a través de Cargo y no hay un paso de transpilación que sustituir.

Lo que sí influye en el tiempo de compilación:

- **Modo debug en desarrollo.** `corvis start` y `cargo run` compilan sin optimizaciones, mucho más rápido que `--release`.
- **Compilación incremental.** Cargo la activa por defecto en debug: solo recompila lo que cambió.
- **Separar en crates.** En un [workspace de Cargo](/cli/monorepo), modificar un crate no obliga a recompilar los demás.
- **Reinicio automático.** `corvis start --watch` recompila y reinicia al guardar; ver [Recarga en caliente](/recipes/hot-reload).
