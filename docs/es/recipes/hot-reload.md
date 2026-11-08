---
status: documented
---

# Recarga en caliente

Rust compila a un binario nativo, así que no hay sustitución de módulos en un proceso en marcha. Lo que sí puedes tener es **recompilar y reiniciar** la aplicación cada vez que guardas un archivo:

```bash
cargo install cargo-watch     # una sola vez
corvis start --watch
```

`corvis start --watch` ejecuta `cargo watch -x run`: vigila el proyecto, recompila cuando cambia un archivo y vuelve a lanzar el servidor. Sin la CLI, el comando equivalente es el mismo `cargo watch -x run`.

## Tiempos de compilación

En proyectos medianos, el tiempo de cada reinicio lo marca la compilación. Algunas medidas que ayudan:

- **Compila en modo debug** durante el desarrollo (lo que hace `corvis start`); deja `--release` para producción.
- **Divide el proyecto en crates** dentro de un [workspace](/es/cli/monorepo): Cargo solo recompila los que cambiaron.
- **Ten en cuenta la carpeta `target`**: cada configuración (debug, release, tests) genera su propia copia de las dependencias y puede ocupar varios GB.

## Estado entre reinicios

Cada reinicio crea un proceso nuevo, así que el estado en memoria (cachés, sesiones, conexiones WebSocket) se pierde. Los clientes WebSocket deben reconectarse; el cliente de `examples/tertulia` lo hace solo.
