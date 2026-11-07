---
status: documented
---

# Static files

To serve a single-page application (SPA), a web client or a folder of uploaded files, Corvis offers two paths that share the same implementation:

- **`ServeStaticModule`**, a module that you import in `AppModule` and configure with options;
- **`corvis::serve_static(root)`**, a middleware that you register with `app.use_middleware`.

Both behave like the `express.static` middleware: they only handle `GET` and `HEAD`, send `ETag`, `Last-Modified` and `Cache-Control`, answer conditional requests (`304`) and ranges (`206`), ignore *dotfiles* (`.env`, `.git`) and read the file from disk in chunks instead of loading it entirely into memory.

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

`for_root` takes a list: you can serve several folders, each under its own prefix.

| `ServeStaticOptions` | Default | Effect |
|---|---|---|
| `ServeStaticOptions::new(root)` | | Folder with the files |
| `.serve_root("/static")` | `/` | URL prefix under which they are served |
| `.exclude(&["/api/(.*)"])` | None | Routes that are never looked up on disk |
| `.spa_fallback(true)` | `false` | For `GET` routes without an extension that do not exist, responds with `index.html` (the client routes of an SPA) |
| `.dotfiles(Dotfiles::Deny)` | `Dotfiles::Ignore` | `Ignore` responds 404, `Deny` 403, `Allow` serves them |
| `.follow_symlinks(false)` | `true` | Reject paths that go through symbolic links or leave the root |

Directories serve their `index.html`.

## The `serve_static` middleware

Tertulia serves its web client like this (`src/lib.rs`):

```rust
app.use_middleware(corvis::serve_static(PUBLIC_DIR));
```

With `.mount(path)` it is mounted under a prefix, which is removed before looking up the file. A typical case is serving user uploads:

```rust
app.use_middleware(corvis::serve_static(std::env::current_dir()?.join("uploads")).mount("/uploads"));
```

The options are those of `express.static`:

| Method | Effect |
|---|---|
| `.mount("/uploads")` | Handles that path and everything under it |
| `.fallthrough(bool)` | With `true` (default), 4xx errors before the file is found pass to the next handler, so the app responds with its usual 404 |
| `.redirect(bool)` | Redirect `/dir` to `/dir/` with `301` |
| `.max_age(ms)`, `.immutable(bool)`, `.cache_control(bool)` | `Cache-Control` header |
| `.index(&["index.html"])` | Index files of directories (`&[]` for none) |
| `.dotfiles(Dotfiles::...)` | *Dotfiles* policy |
| `.etag(bool)`, `.last_modified(bool)`, `.accept_ranges(bool)` | Cache and range headers |
| `.extensions(&["html"])` | Extensions to try if the file does not exist (`/about` → `about.html`) |
| `.follow_symlinks(bool)` | Follow symbolic links (default `true`) |

## Security

- Static files **do not go through guards** (neither global nor controller guards). A file with the same path as a protected endpoint is served before the endpoint. If a folder must not be public, protect it with a middleware of your own in front, or do not serve it.
- Do not serve the project folder or one that contains secrets; although *dotfiles* are ignored by default, any other file in that folder is accessible.
- For user uploads, generate the file name on the server and consider `Content-Disposition: attachment` for types that the browser could execute. See [File upload](/http/file-upload).
