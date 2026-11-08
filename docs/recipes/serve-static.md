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
| `.mount("/uploads")` | Handles that path and everything under it (case-insensitive, by whole segments) and removes the prefix |
| `.fallthrough(bool)` | With `true` (default), methods other than `GET`/`HEAD` and 4xx errors before the file is found (invalid encoding, `..`, *dotfiles*, missing file) pass to the next handler, so the app responds with its usual 404 |
| `.redirect(bool)` | Redirect `/dir` to `/dir/` with `301`, `Location: /dir/` and the same HTML body as `express.static`; with a trailing slash it serves `index.html` or moves on |
| `.max_age(ms)`, `.immutable(bool)`, `.cache_control(bool)` | `Cache-Control` header |
| `.index(&["index.html"])` | Index files of directories (`&[]` for none) |
| `.dotfiles(Dotfiles::...)` | *Dotfiles* policy |
| `.etag(bool)`, `.last_modified(bool)`, `.accept_ranges(bool)` | Cache and range headers |
| `.extensions(&["html"])` | Extensions to try if the file does not exist (`/about` → `about.html`) |
| `.follow_symlinks(bool)` | Follow symbolic links (default `true`) |

Files carry `Accept-Ranges: bytes`, `Cache-Control: public, max-age=N`, `Last-Modified`, a weak `ETag`
(`W/"<size in hex>-<mtime in hex>"`) and the `Content-Type` from `mime-types`. Conditional `GET`s (`304`, `412`)
and single ranges (`206`, `416`) are answered too. The body is read from disk in chunks of up to 64 KiB (only the
requested range), never the whole file: a `Range: bytes=0-0` on a 256 MiB file does not allocate 256 MiB.
`StreamableFile::from_path` reads the same way. `ServeStaticModule::for_root` uses this same middleware
(*dotfiles* ignored by default, `Range`, `ETag` and the `301` redirect of directories).

## Security

- The *dotfiles* policy is also applied to the real names on disk under the canonical root, not only to the URL segments. On Windows, 8.3 short names (`ENV~1` → `.env`, `GIT~1` → `.git`) and trailing dots (`ENV~1.`) answer the same as `.env`: 404 with `Ignore`, 403 with `Deny`. A link inside the root that points to a *dotfile* counts as a *dotfile*. With `Dotfiles::Allow` (and `follow_symlinks(true)`) no extra check is made.
- On Windows no segment containing `:` is served (`a.txt:hidden`, `a.txt::$DATA`, NTFS alternate streams): it responds 404.
- Symbolic links and junctions are followed by default, as in `express.static`. With `follow_symlinks(false)` (on `serve_static` or `ServeStaticOptions`) any path that goes through a link or junction, or whose canonical path ends up outside the canonical root, responds 404.
- Static files **do not go through guards** (neither global nor controller guards). A file with the same path as a protected endpoint is served before the endpoint. If a folder must not be public, protect it with a middleware of your own in front, or do not serve it.
- Do not serve the project folder or one that contains secrets; although *dotfiles* are ignored by default, any other file in that folder is accessible.
- For user uploads, generate the file name on the server and consider `Content-Disposition: attachment` for types that the browser could execute. See [File upload](/http/file-upload).
