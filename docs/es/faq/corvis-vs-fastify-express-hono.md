---
status: documented
---

# Corvis frente a Fastify, Express y Hono

Express, Fastify y Hono son frameworks HTTP minimalistas de JavaScript/TypeScript: te dan un router,
un sistema de middleware o *plugins* y la request/respuesta, y dejan la arquitectura en tus manos.
Corvis está en otra categoría y en otro lenguaje: es un framework de aplicación para Rust que define
cómo se organiza el código.

## Qué aporta Corvis

| | Express / Fastify / Hono | Corvis |
|---|---|---|
| Lenguaje y runtime | JavaScript/TypeScript sobre Node.js (Hono también sobre otros runtimes) | Rust, binario nativo sobre Tokio y hyper |
| Organización | Libre | [Módulos](/es/modules) con `imports`, `providers`, `controllers` y `exports` |
| Dependencias | Manuales (o una librería aparte) | [Inyección de dependencias](/es/providers) por tipo, resuelta al arrancar |
| Pipeline de la petición | Middleware / hooks | Middleware, [guards](/es/guards), [interceptors](/es/interceptors), [pipes](/es/pipes) y [filtros de excepción](/es/exception-filters) |
| Validación | Librería aparte | `#[derive(Validate)]` y `corvis::z` (schemas al estilo zod) |
| Errores de tipos | En tiempo de ejecución (o en el compilador de TypeScript) | El compilador de Rust comprueba handlers, DTOs y providers |
| Módulos incluidos | Ecosistema de paquetes | Configuración, JWT, eventos, tareas programadas, caché, throttler, health checks, archivos estáticos, WebSockets, testing |

Si vienes de Express, buena parte del comportamiento observable te resultará familiar: el body
parser, el router, los `ETag`, `res.cookie`, la subida de archivos y CORS siguen la semántica de
Express 5 y de sus paquetes habituales, y el repositorio tiene pruebas que comparan las respuestas con
las de una aplicación Express real (`crates/corvis/tests/express_parity.rs`, `res.rs`, `multer.rs`).

## Rendimiento medido

El directorio `bench/` contiene dos aplicaciones con los mismos endpoints: una de Corvis en
`--release` y otra de Node.js escrita con un framework modular equivalente, medida sobre Express y
sobre Fastify. Se miden con [oha](https://github.com/hatoo/oha) en `127.0.0.1`, con 100 conexiones
*keep-alive* y 15 s por escenario. Estos son los resultados publicados en el README del repositorio,
en un i9-14900KF (32 hilos) con Windows 11, Node 24 y rustc 1.99, con cliente y servidor en la misma
máquina:

| Escenario | Node.js + Express | Node.js + Fastify | Corvis (1 worker) | Corvis (todos los núcleos) |
|---|---:|---:|---:|---:|
| `GET /` texto | 713 mil req/min | 1.02 M req/min | 1.54 M req/min | 7.15 M req/min |
| `GET /json` | 680 mil | 963 mil | 1.45 M | 5.26 M |
| `GET /users/:id` + `ParseIntPipe` | 647 mil | 879 mil | 1.48 M | 7.59 M |
| `POST /users` + `ValidationPipe` | 446 mil | 547 mil | 1.42 M | 3.42 M |
| p50 `GET /` | 8.98 ms | 4.46 ms | 4.03 ms | 0.73 ms |
| RSS bajo carga | ~165–205 MB | ~163–344 MB | ~11 MB | ~9 MB |

Cómo leer estos números:

- La columna "1 worker" es la comparación por núcleo con un proceso de Node.js sin cluster.
- La columna "todos los núcleos" está limitada sobre todo por el entorno: el generador de carga y el
  loopback de Windows se saturan cerca de 110–120 mil req/s.
- La fila del `POST` tuvo más varianza entre corridas.
- No hay mediciones contra Hono.

Para repetirlo en tu máquina: `bash bench/run.sh`. Los resultados detallados se escriben en
`bench/results.md`.

## Cuándo no es la mejor opción

- Si tu equipo trabaja en TypeScript y no quiere introducir Rust.
- Si necesitas GraphQL o microservicios con transportes de mensajería: Corvis todavía no los tiene
  (ver [GraphQL](/es/graphql/quick-start) y [Microservicios](/es/microservices/basics)).
- Si vas a desplegar en entornos *edge* o *serverless* que esperan JavaScript o WebAssembly (ver
  [Cloudflare Workers](/es/faq/cloudflare-workers) y [Serverless](/es/faq/serverless)).
