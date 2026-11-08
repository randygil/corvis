---
status: unavailable
---

# Modelo-Vista-Controlador

Corvis no tiene integración con motores de plantillas: no hay un decorador para renderizar una vista
(`@Render`), ni configuración de carpeta de vistas o motor (Handlebars, Pug, etc.).

Lo que sí existe:

- **Devolver HTML.** Un handler puede devolver `Html(...)` o `Response::html(...)`, que responden con
  `Content-Type: text/html; charset=utf-8`. Puedes generar ese HTML con cualquier crate de plantillas
  del ecosistema Rust (por ejemplo `askama`, `minijinja` o `tera`) y envolver el resultado:

  ```rust
  use corvis::Html; // no está en el prelude

  #[get]
  fn index(&self) -> Html<String> {
      let name = "Corvis";
      Html(format!("<h1>Hola, {name}</h1>"))
  }
  ```

  Si interpolas datos del usuario, escápalos: `format!` no lo hace. Los motores de plantillas
  suelen escapar por defecto.

- **Servir archivos estáticos.** `corvis::serve_static(carpeta)` o `ServeStaticModule` sirven
  HTML, CSS, JavaScript e imágenes, por ejemplo para una SPA. `examples/tertulia` sirve así su
  cliente web desde `public/`. Ver [Archivos estáticos](/es/recipes/serve-static).

- **Redirecciones.** `Redirect::to("/login")`, `Redirect::permanent(..)` o `#[redirect("url", 301)]`.
