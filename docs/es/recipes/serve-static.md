---
status: documented
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
| `.mount("/uploads")` | Atiende esa ruta y todo lo que cuelga de ella (sin distinguir mayúsculas y por segmentos completos) y le quita el prefijo |
| `.fallthrough(bool)` | Con `true` (por defecto) los métodos distintos de `GET`/`HEAD` y los errores 4xx previos a encontrar el archivo (codificación inválida, `..`, *dotfiles*, archivo inexistente) pasan al siguiente handler, así la app responde su 404 habitual |
| `.redirect(bool)` | Redirigir `/dir` a `/dir/` con `301`, `Location: /dir/` y el mismo HTML de `express.static`; con barra final sirve `index.html` o sigue de largo |
| `.max_age(ms)`, `.immutable(bool)`, `.cache_control(bool)` | Cabecera `Cache-Control` |
| `.index(&["index.html"])` | Archivos índice de los directorios (`&[]` para ninguno) |
| `.dotfiles(Dotfiles::...)` | Política de *dotfiles* |
| `.etag(bool)`, `.last_modified(bool)`, `.accept_ranges(bool)` | Cabeceras de caché y rangos |
| `.extensions(&["html"])` | Extensiones a probar si el archivo no existe (`/about` → `about.html`) |
| `.follow_symlinks(bool)` | Seguir enlaces simbólicos (por defecto `true`) |

Los archivos llevan `Accept-Ranges: bytes`, `Cache-Control: public, max-age=N`, `Last-Modified`, un `ETag`
débil (`W/"<tamaño hex>-<mtime hex>"`) y el `Content-Type` de `mime-types`. También se responden GET
condicionales (`304`, `412`) y rangos simples (`206`, `416`). El cuerpo se lee del disco por partes de hasta
64 KiB (solo el rango pedido), nunca el archivo completo: un `Range: bytes=0-0` sobre un archivo de 256 MiB no
reserva 256 MiB. `StreamableFile::from_path` lee igual. `ServeStaticModule::for_root` usa este mismo
middleware (*dotfiles* ignorados por defecto, `Range`, `ETag` y la redirección `301` de los directorios).

## Seguridad

- La política de *dotfiles* también se aplica a los nombres reales que hay en disco bajo la raíz canónica, no solo a los segmentos de la URL. En Windows, los nombres cortos 8.3 (`ENV~1` → `.env`, `GIT~1` → `.git`) y los puntos finales (`ENV~1.`) responden igual que `.env`: 404 con `Ignore`, 403 con `Deny`. Un enlace dentro de la raíz que apunta a un *dotfile* cuenta como *dotfile*. Con `Dotfiles::Allow` (y `follow_symlinks(true)`) no se hace ninguna comprobación extra.
- En Windows no se sirve ningún segmento con `:` (`a.txt:hidden`, `a.txt::$DATA`, flujos alternativos de NTFS): responde 404.
- Los enlaces simbólicos y las uniones se siguen por defecto, como en `express.static`. Con `follow_symlinks(false)` (en `serve_static` o en `ServeStaticOptions`) cualquier ruta que pase por un enlace o unión, o cuya ruta canónica quede fuera de la raíz canónica, responde 404.
- Los archivos estáticos **no pasan por los guards** (ni globales ni de controlador). Un archivo con la misma ruta que un endpoint protegido se sirve antes que el endpoint. Si una carpeta no debe ser pública, protégela con un middleware propio delante, o no la sirvas.
- No sirvas la carpeta del proyecto ni una que contenga secretos; aunque los *dotfiles* se ignoran por defecto, cualquier otro archivo de esa carpeta queda accesible.
- Para subidas de usuarios, genera el nombre del archivo en el servidor y considera `Content-Disposition: attachment` para tipos que el navegador podría ejecutar. Ver [Subida de archivos](/es/http/file-upload).
