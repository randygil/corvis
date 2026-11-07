---
status: documentado
---

# Archivos estáticos

Para servir una aplicación de una sola página (SPA), un cliente web o una carpeta de archivos subidos, Corvis ofrece dos caminos que comparten la misma implementación:

- **`ServeStaticModule`**, un módulo que se importa en `AppModule` y se configura con opciones;
- **`corvis::serve_static(raíz)`**, un middleware que se registra con `app.use_middleware`.

Ambos se comportan como el middleware `express.static`: solo atienden `GET` y `HEAD`, envían `ETag`, `Last-Modified` y `Cache-Control`, responden peticiones condicionales (`304`) y rangos (`206`), ignoran los *dotfiles* (`.env`, `.git`) y leen el archivo del disco por partes en lugar de cargarlo entero en memoria.

## `ServeStaticModule`

```rust
use corvis::prelude::*;

#[module(
    imports = [
        ServeStaticModule::for_root(vec![
            ServeStaticOptions::new("./client/dist")
                .exclude(&["/api/(.*)"])
                .spa_fallback(true),
        ]),
    ],
    controllers = [AppController],
)]
pub struct AppModule;
```

`for_root` recibe una lista: puedes servir varias carpetas, cada una en su prefijo.

| `ServeStaticOptions` | Por defecto | Efecto |
|---|---|---|
| `ServeStaticOptions::new(raíz)` | | Carpeta con los archivos |
| `.serve_root("/static")` | `/` | Prefijo de URL bajo el que se sirven |
| `.exclude(&["/api/(.*)"])` | Ninguna | Rutas que nunca se buscan en disco |
| `.spa_fallback(true)` | `false` | Para rutas `GET` sin extensión que no existen, responde `index.html` (las rutas del cliente de una SPA) |
| `.dotfiles(Dotfiles::Deny)` | `Dotfiles::Ignore` | `Ignore` responde 404, `Deny` 403, `Allow` los sirve |
| `.follow_symlinks(false)` | `true` | Rechazar rutas que pasan por enlaces simbólicos o salen de la raíz |

Los directorios sirven su `index.html`.

## El middleware `serve_static`

Tertulia sirve su cliente web así (`src/lib.rs`):

```rust
app.use_middleware(corvis::serve_static(PUBLIC_DIR));
```

Con `.mount(ruta)` se monta bajo un prefijo, que se quita antes de buscar el archivo. Un caso típico es servir las subidas de los usuarios:

```rust
app.use_middleware(corvis::serve_static(std::env::current_dir()?.join("uploads")).mount("/uploads"));
```

Las opciones son las de `express.static`:

| Método | Efecto |
|---|---|
| `.mount("/uploads")` | Atiende esa ruta y todo lo que cuelga de ella |
| `.fallthrough(bool)` | Con `true` (por defecto) los errores 4xx previos a encontrar el archivo pasan al siguiente handler, así la app responde su 404 habitual |
| `.redirect(bool)` | Redirigir `/dir` a `/dir/` con `301` |
| `.max_age(ms)`, `.immutable(bool)`, `.cache_control(bool)` | Cabecera `Cache-Control` |
| `.index(&["index.html"])` | Archivos índice de los directorios (`&[]` para ninguno) |
| `.dotfiles(Dotfiles::...)` | Política de *dotfiles* |
| `.etag(bool)`, `.last_modified(bool)`, `.accept_ranges(bool)` | Cabeceras de caché y rangos |
| `.extensions(&["html"])` | Extensiones a probar si el archivo no existe (`/about` → `about.html`) |
| `.follow_symlinks(bool)` | Seguir enlaces simbólicos (por defecto `true`) |

## Seguridad

- Los archivos estáticos **no pasan por los guards** (ni globales ni de controlador). Un archivo con la misma ruta que un endpoint protegido se sirve antes que el endpoint. Si una carpeta no debe ser pública, protégela con un middleware propio delante, o no la sirvas.
- No sirvas la carpeta del proyecto ni una que contenga secretos; aunque los *dotfiles* se ignoran por defecto, cualquier otro archivo de esa carpeta queda accesible.
- Para subidas de usuarios, genera el nombre del archivo en el servidor y considera `Content-Disposition: attachment` para tipos que el navegador podría ejecutar. Ver [Subida de archivos](/http/file-upload).
