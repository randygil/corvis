---
status: documented
---

# Recarga en caliente

Rust compila a un binario nativo, así que no hay sustitución de módulos en un proceso en marcha. Lo que sí puedes tener es **recompilar y reiniciar** la aplicación cada vez que guardas un archivo:

```bash
cargo install --locked watchexec-cli     # una sola vez
corvis start --watch
```

`corvis start --watch` ejecuta `cargo run` bajo [watchexec](https://github.com/watchexec/watchexec): vigila el proyecto, recompila cuando cambia un archivo y vuelve a lanzar el servidor. `--release` y los argumentos tras `--` se incluyen: `corvis start --watch --release -- --port 4000` reinicia `cargo run --release -- --port 4000`. El comando exacto y las opciones de instalación están en [Uso](/es/cli/usages#corvis-start). Si watchexec no está instalado, `corvis start --watch` se detiene con un mensaje que muestra `cargo install --locked watchexec-cli`.

En Windows, watchexec solo puede terminar el servidor de forma forzada al reiniciarlo, así que los hooks de apagado no se ejecutan en cada reinicio durante el desarrollo. En Linux y macOS envía primero una señal de terminación.

## Tiempos de compilación

En proyectos medianos, el tiempo de cada reinicio lo marca la compilación. Algunas medidas que ayudan:

- **Compila en modo debug** durante el desarrollo (lo que hace `corvis start`); deja `--release` para producción.
- **Divide el proyecto en crates** dentro de un [workspace](/es/cli/monorepo): Cargo solo recompila los que cambiaron.
- **Ten en cuenta la carpeta `target`**: cada configuración (debug, release, tests) genera su propia copia de las dependencias y puede ocupar varios GB.

## Estado entre reinicios

Cada reinicio crea un proceso nuevo, así que el estado en memoria (cachés, sesiones, conexiones WebSocket) se pierde. Los clientes WebSocket deben reconectarse; el cliente de `examples/tertulia` lo hace solo.
