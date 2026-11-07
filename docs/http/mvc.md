---
status: unavailable
---

# Model-View-Controller

Corvis has no template engine integration: there is no decorator to render a view
(`@Render`), and no configuration for a views folder or engine (Handlebars, Pug, etc.).

What does exist:

- **Returning HTML.** A handler can return `Html(...)` or `Response::html(...)`, which respond with
  `Content-Type: text/html; charset=utf-8`. You can generate that HTML with any template crate
  from the Rust ecosystem (for example `askama`, `minijinja` or `tera`) and wrap the result:

  ```rust
  use corvis::Html; // not in the prelude

  #[get]
  fn index(&self) -> Html<String> {
      let name = "Corvis";
      Html(format!("<h1>Hello, {name}</h1>"))
  }
  ```

  If you interpolate user data, escape it: `format!` does not do it. Template engines
  usually escape by default.

- **Serving static files.** `corvis::serve_static(folder)` or `ServeStaticModule` serve
  HTML, CSS, JavaScript and images, for example for an SPA. `examples/tertulia` serves its
  web client this way from `public/`. See [Static files](/recipes/serve-static).

- **Redirects.** `Redirect::to("/login")`, `Redirect::permanent(..)` or `#[redirect("url", 301)]`.
