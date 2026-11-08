---
status: documented
---

# Ciclo de vida de la petición

Una petición HTTP atraviesa varias capas antes de llegar al handler y otras tantas a la vuelta. Saber
en qué orden se ejecutan ayuda a decidir dónde poner cada pieza (¿esto va en un middleware o en un
guard?) y a entender por qué un error se respondió de cierta forma.

## Resumen

1. CORS: respuesta inmediata a los preflight `OPTIONS`.
2. Body parser (sus errores van directo a los filtros globales).
3. Middleware globales (`app.use_middleware`, `app.use_fn`).
4. Middleware de módulos (`CorvisModule::configure`).
5. Archivos estáticos de `ServeStaticModule` y resolución de la ruta (`404` si no hay).
6. Guards: globales → del controlador → del método.
7. Interceptors, parte "antes": globales → del controlador → del método.
8. Pipes: globales → del controlador → del método → del parámetro.
9. Handler.
10. Interceptors, parte "después", en orden inverso.
11. Filtros de excepción, si algo falló: del método → del controlador → globales.
12. Cabeceras CORS y escritura de la respuesta.

## Middleware

Los middleware globales se ejecutan primero, en el orden en que se registraron en el código de
arranque. Luego vienen los middleware de los módulos que implementan `CorvisModule::configure` y cuyo
`for_routes` coincide con la ruta, empezando por el módulo raíz y siguiendo por sus imports.

Un middleware recibe la `Request` y un `Next`, y decide si continúa (`next.run(req).await`) o responde
él mismo. Lo que devuelve `next.run` ya es la respuesta final de la ruta, con los filtros aplicados;
los únicos errores que pueden llegarle como `Err` son los que se producen antes de entrar en la ruta
(por ejemplo, `404 Cannot GET /x`).

`corvis::serve_static(...)` registrado con `app.use_middleware` es un middleware global más, así que
se ejecuta en su posición de registro. `ServeStaticModule`, en cambio, se consulta después de todos
los middleware y antes de resolver la ruta.

## Guards

Se ejecutan en este orden y se detienen en el primero que devuelve `false` (respuesta
`403 Forbidden resource`) o un error:

1. Globales registrados como provider con `Provider::app_guard`, en el orden de los módulos.
2. Globales registrados con `app.use_global_guards(...)`, en orden de llamada.
3. Del controlador (`#[use_guards]` debajo de `#[controller]`), de izquierda a derecha.
4. Del método.

Los guards se ejecutan **antes** que los pipes: ven el cuerpo sin validar.

## Interceptors

Envuelven al resto de la cadena. El orden de entrada es el mismo que el de los guards (providers
globales, `use_global_interceptors`, controlador, método), y el de salida es el inverso: el primer
interceptor en entrar es el último en ver la respuesta. Por eso un interceptor registrado como
provider (`Provider::app_interceptor`) envuelve a los de `use_global_interceptors`; está comprobado en
`crates/corvis/tests/enhancer_order.rs`.

Un interceptor puede no llamar a `next.handle()` y devolver su propia respuesta (es lo que hace
`CacheInterceptor` con un acierto de caché).

## Pipes

Para cada parámetro del handler (`#[body]`, `#[param]`, `#[query]`, ...) se ejecutan, en cadena, los
pipes globales (los de `Provider::app_pipe` primero), los del controlador, los del método y por
último los del propio parámetro (`#[param("id", ParseIntPipe)]`). La salida de cada pipe es la
entrada del siguiente, y la salida final se deserializa en el tipo del parámetro.

Todos los parámetros se procesan; si fallan varios, gana el error del pipe más temprano en su cadena
y, si empatan, el del último parámetro declarado.

Los parámetros sin atributo (tipos que implementan `FromContext`) no pasan por pipes.

## Filtros de excepción

Si un guard, interceptor, pipe o el handler devuelve un error (o entra en `panic`), Corvis busca un
filtro en este orden:

1. Los del método (`#[use_filters]` en el handler).
2. Los del controlador.
3. Los globales, el último registrado primero: los de `app.use_global_filters(...)` (de la última
   llamada a la primera), luego los registrados con `Provider::app_filter` (del último al primero).

Se usa el **primer** filtro cuyo `catches(&exception)` devuelve `true`. Si ninguno lo acepta,
`BaseExceptionFilter` responde con el cuerpo estándar
(`{"message":...,"error":...,"statusCode":...}` o `{"statusCode":500,"message":"Internal server error"}`).

Los errores que ocurren fuera de una ruta (en un middleware, al resolver la ruta o en el body parser)
solo pasan por los filtros globales.

## Respuesta

El estado por defecto es `201` en `POST` y `200` en el resto, salvo `#[http_code(...)]`. Se fija antes de correr la ruta, así que los guards y los interceptors lo ven con
`ctx.switch_to_http().status_code()`. Después de
los filtros se aplican las cabeceras CORS (las que fijó el handler tienen prioridad), el `ETag` y
`Content-Length` en los cuerpos en memoria, y la respuesta `304` si la petición condicional sigue
vigente.

## WebSockets

En un gateway, cada mensaje pasa por guards → interceptors → pipes → handler, con los enhancers globales, los del
gateway y los del handler. Los filtros de excepción globales no se aplican; los errores pasan por los
`#[use_filters]` del handler y del gateway y, si ninguno los atrapa, se envían al cliente como un evento `exception`. Ver
[Gateways](/es/websockets/gateways).
