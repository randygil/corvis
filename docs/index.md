---
status: documentado
---

# Introducción

<p class="cv-lead">Corvis es un framework backend modular para Rust. Organiza una aplicación en módulos, resuelve las dependencias entre servicios con un contenedor de inyección y procesa cada petición a través de una cadena explícita de middleware, guards, interceptors, pipes y filtros de excepción.</p>

Si ya trabajaste con frameworks de Node.js con TypeScript basados en decoradores, el vocabulario te resultará familiar: los decoradores son atributos de Rust (`#[get(":id")]`), las clases son `struct`s y el constructor se genera solo a partir de los campos `Arc<T>` del struct. Si no, no hace falta ningún conocimiento previo: esta documentación explica cada pieza desde cero.

## Qué resuelve

Rust tiene muy buenas piezas para construir servidores HTTP (hyper, tokio, serde), pero deja en tus manos la **arquitectura**: dónde vive cada servicio, quién lo crea, cómo se comparte, cómo se prueba de forma aislada y cómo se aplican la autenticación o la validación a muchas rutas a la vez. Corvis responde a esas preguntas con un modelo único:

- **Módulos** que agrupan controladores y providers y deciden qué exponen a otros módulos.
- **Inyección de dependencias** por tipo: un campo `Arc<UsersService>` basta para recibir el servicio.
- **Controladores** declarativos: rutas, parámetros, códigos de estado y cabeceras con atributos.
- **Pipeline de la petición** con piezas reutilizables y comprobables por separado.
- **Módulos incluidos** para lo que casi toda API necesita: configuración, JWT, eventos, tareas programadas, caché, límite de peticiones, health checks, archivos estáticos, subida de archivos y WebSockets.

## Un vistazo

```rust
use corvis::prelude::*;

#[injectable]
pub struct CatsService;

impl CatsService {
    pub fn find_all(&self) -> Vec<&'static str> {
        vec!["Tom", "Garfield"]
    }
}

#[controller("cats")]
pub struct CatsController {
    cats_service: Arc<CatsService>,
}

#[routes]
impl CatsController {
    #[get]
    fn find_all(&self) -> Vec<&'static str> {
        self.cats_service.find_all()
    }

    #[get(":id")]
    fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Value> {
        if id == 0 {
            return Err(NotFoundException::new(format!("Cat #{id} not found")));
        }
        Ok(json!({ "id": id }))
    }
}

#[module(controllers = [CatsController], providers = [CatsService])]
pub struct AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let app = CorvisFactory::create::<AppModule>().await?;
    app.listen("127.0.0.1:3000").await
}
```

`CatsController` no crea su servicio: lo declara como campo y el contenedor lo entrega al construir el controlador. `ParseIntPipe` convierte y valida el parámetro antes de llamar al método, y un `Err(NotFoundException)` se convierte en una respuesta `404` con cuerpo JSON.

## Por dónde seguir

<div class="cv-cards">
  <a href="/first-steps"><strong>Primeros pasos</strong><span>Crea un proyecto con la CLI y entiende su estructura.</span></a>
  <a href="/providers"><strong>Providers</strong><span>Servicios, inyección por tipo y tokens.</span></a>
  <a href="/modules"><strong>Módulos</strong><span>Encapsulación, exports, módulos globales y dinámicos.</span></a>
  <a href="/websockets/gateways"><strong>WebSockets</strong><span>Gateways con salas, guards y acks.</span></a>
</div>

## El ejemplo de referencia

Muchas páginas usan como ejemplo **Tertulia** (`examples/tertulia` en el repositorio): un chat en tiempo real con salas, presencia, autenticación JWT, validación con `corvis::z`, historial en SQLite con `corvis-orm`, endpoints REST que leen el estado del gateway, Server-Sent Events y un WebSocket plano para moderadores.

```bash
JWT_SECRET=un-secreto-largo cargo run -p tertulia   # http://127.0.0.1:5150
```

El repositorio incluye además `examples/cats-app` (autenticación, roles, versionado, caché, eventos y tareas programadas) y `examples/blog-orm` (API REST sobre `corvis-orm`). Ver [Ejemplos](/faq/examples).

## Estado del proyecto

Corvis está en la versión `0.1.0`. Las páginas marcadas como **No disponible todavía** corresponden a capacidades que el framework aún no tiene (por ejemplo GraphQL o microservicios); se mantienen en el menú para que la estructura de la documentación sea estable y cada una indica la alternativa más cercana que sí existe.
