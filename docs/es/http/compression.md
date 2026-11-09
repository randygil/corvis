---
status: documented
---

# Compresión

Corvis comprime las respuestas con `br`, `gzip` o `deflate` cuando lo activas, con las mismas reglas que el middleware
`compression` (1.8.2): las mismas cabeceras, las mismas decisiones y los mismos cuerpos decodificados. Viene
**desactivado** dos veces: el código está detrás de la feature de cargo `compression` y nada se comprime hasta que la
aplicación llama a `enable_compression`. Sin ambas cosas, las respuestas son exactamente las de siempre.

Corvis no descomprime las peticiones: un cuerpo con `Content-Encoding: gzip`, `deflate` o `br` se sigue rechazando con
`415 unsupported content encoding` (ver [Cuerpo en bruto](/es/faq/raw-body)).

## Activarla

```toml
[dependencies]
corvis = { version = "0.1", features = ["compression"] }
```

```rust
use corvis::prelude::*;

let mut app = CorvisFactory::create::<AppModule>().await?;
app.enable_compression(CompressionOptions::new());
app.listen("127.0.0.1:3000").await?;
```

`enable_compression` hace el papel de `app.use(compression())`. El cliente de pruebas (`corvis::testing`) pasa por el
mismo código que el servidor, así que una aplicación de test comprime igual que la real.

## Opciones

| Método | Por defecto | Opción de `compression` |
|---|---|---|
| `.threshold(bytes)` | 1024 | `threshold` |
| `.level(0..=9)` | 6 | `level` (gzip y deflate) |
| `.brotli_quality(0..=11)` | 4 | `brotli.params[BROTLI_PARAM_QUALITY]` |
| `.filter(\|ctx\| ...)` | `should_compress` | `filter` |
| `.max_concurrent(n)` | 8 veces los núcleos disponibles | ninguna (ver [el límite](#compresiones-concurrentes)) |

Los valores fuera de rango se ajustan al límite (`level(20)` es 9, `brotli_quality(50)` es 11, `max_concurrent(0)` es 1).

`filter` recibe un `CompressionContext` con el método, la ruta y las cabeceras de la petición y el estado y las
cabeceras de la respuesta. `corvis::should_compress` es el filtro por defecto y es público, como `compression.filter`,
así que un filtro propio puede recurrir a él:

```rust
app.enable_compression(
    CompressionOptions::new()
        .threshold(2048)
        .filter(|ctx| !ctx.request_headers.contains_key("x-no-compression") && should_compress(ctx)),
);
```

Las cabeceras de la petición se copian para el filtro solo cuando defines uno.

## Cuándo se comprime una respuesta

Las comprobaciones se hacen en este orden, como en el middleware:

1. **Filtro.** El de por defecto acepta un `Content-Type` que mime-db 1.54.0 marca como comprimible (JSON, HTML, texto,
   JavaScript, CSS, SVG, XML, `application/octet-stream` y cientos más) y, para los tipos sin esa marca, `text/*` y los
   sufijos `+json`, `+text` y `+xml`. Las imágenes, los archivos comprimidos, las fuentes y el vídeo se dejan como
   están. Una respuesta sin `Content-Type` no se comprime.
2. **`Cache-Control: no-transform`** (como elemento separado por comas, sin distinguir mayúsculas).
3. **`Vary: Accept-Encoding`** se añade a toda respuesta que pasó las dos primeras comprobaciones, termine o no
   comprimida, conservando los valores existentes (`Vary: Origin` pasa a `Origin, Accept-Encoding`) y dejando `Vary: *`.
4. **Umbral.** Un `Content-Length` menor que el umbral, o sin esa cabecera un cuerpo de tamaño conocido y menor, no se
   comprime. Un cuerpo en streaming de longitud desconocida se comprime sea cual sea su tamaño.
5. **`Content-Encoding` existente** distinto de `identity`.
6. **`HEAD`.** Una respuesta `HEAD` conserva el `Content-Length` que habría tenido el `GET`.
7. **Rangos y estados sin cuerpo** (ver [diferencias](#diferencias-con-el-middleware-original)).
8. **Negociación.** La codificación sale de `Accept-Encoding` como lo hace negotiator 0.6.4: valores `q`, `*`,
   `identity` aceptable con la calidad más baja listada, `q=0` excluido y, en empates, `br`, luego `gzip` y luego
   `deflate`. Sin la cabecera, o cuando no hay nada más aceptable, la respuesta se envía tal cual (nunca un `406`).

Una respuesta comprimida recibe `Content-Encoding` y pierde `Content-Length`. Su estado, `ETag`, `Accept-Ranges` y el
resto de cabeceras se conservan, como en el middleware.

`deflate` es el formato zlib (RFC 1950), el que esperan los navegadores y el que produce `createDeflate` de Node.

La compresión se aplica a la respuesta final, después de `X-Powered-By`, el `ETag`, el `304` y las cabeceras puestas
por middleware, interceptors, filtros de excepciones y `#[header]`.

## Streams y Server-Sent Events

Los cuerpos en streaming (archivos estáticos, `StreamableFile`, `Response::stream`) se comprimen a medida que se leen.
Como máximo 64 KiB pasan por el codificador en cada paso y la tarea cede el turno al runtime entre pasos, así que un
cuerpo grande no retiene un hilo de trabajo. Cuando la fuente no tiene nada listo, el codificador vacía su búfer
(flush): un stream lento nunca queda retenido por los búferes del compresor. No necesitas nada parecido a
`res.flush()`. El coste son unos pocos bytes por flush en streams muy lentos.

`text/event-stream` nunca se comprime. Ver [Server-Sent Events](/es/http/server-sent-events).

## Diferencias con el middleware original

- **`text/event-stream` nunca se comprime.** El middleware lo comprime salvo que haya `Cache-Control: no-transform` y la
  aplicación llame a `res.flush()`; el SSE de la implementación original envía `no-transform`, así que en la práctica
  allí tampoco se comprime. El `Sse` de Corvis no envía `no-transform`, por lo que el filtro excluye el tipo. No se
  añade `Vary`, que es también lo que recibe el SSE original.
- **Las respuestas con `Content-Range` (`206`, `416`) y los estados sin cuerpo (1xx, `204`, `304`) nunca se comprimen.**
  El middleware comprime un `206` y conserva un `Content-Range` que se refiere a los bytes sin comprimir, lo que
  corrompe las descargas reanudadas. `Vary: Accept-Encoding` se sigue añadiendo, así que la única diferencia visible es
  la falta de `Content-Encoding` en un `206`.
- No disponibles: `enforceEncoding`, `chunkSize`, `memLevel`, `windowBits`, `strategy` ni `params` propios de brotli;
  `res.flush()` no hace falta.
- **La ventana de brotli es como máximo de 512 KiB (2^19)**, donde Node usa 4 MiB fijos (2^22). Por debajo de eso se
  elige por respuesta: la más pequeña desde 64 KiB que cubra el cuerpo. La salida decodificada es la misma; la tasa de
  compresión es algo menor en cuerpos muy grandes, y un codificador cuesta unos 5 MiB en lugar de unos 15 MiB.
- **Un límite de compresiones en curso** (ver [más abajo](#compresiones-concurrentes)); el middleware no tiene ninguno.
- Los bytes comprimidos no son los que produce el zlib de Node, solo equivalentes: los clientes decodifican ambos igual.

## Seguridad

**BREACH.** Una respuesta comprimida que mezcla un secreto (un token CSRF, un valor ligado a la sesión) con texto que el
atacante puede influir permite que un atacante que vea los tamaños cifrados recupere el secreto byte a byte. Por eso la
compresión es opcional. Para reducir el riesgo:

- no reflejes datos de la petición junto a secretos en la misma respuesta;
- enmascara los tokens con un valor aleatorio nuevo en cada respuesta;
- excluye esas rutas con `filter` o enviando `Cache-Control: no-transform`;
- o comprime solo los activos estáticos, en un proxy.

**CPU.** Cualquier cliente puede pedir respuestas comprimidas de los recursos comprimibles más grandes. Los valores por
defecto son los del middleware (gzip 6, brotli 4); el umbral y la lista de tipos dejan fuera los cuerpos pequeños, las
imágenes y los archivos comprimidos; y un paso de compresión procesa como máximo 64 KiB. Velocidad aproximada de los
codificadores sobre JSON, medida en un núcleo de una máquina de desarrollo con una compilación release:

| Codificación | Velocidad | Tamaño comprimido |
|---|---|---|
| gzip nivel 1 | unos 640 MiB/s | 17% |
| gzip nivel 6 (por defecto) | unos 85 MiB/s | 12,6% |
| gzip nivel 9 | unos 45 MiB/s | 12,6% |
| brotli calidad 4 (por defecto) | unos 130 MiB/s | 13,6% |
| brotli calidad 11 | alrededor de 1 MiB/s | 5,1% |

No subas `brotli_quality` por encima de 6 para respuestas dinámicas.

`application/octet-stream` es comprimible según mime-db, como en el middleware. Si sirves descargas binarias, dales un
tipo preciso o exclúyelas con `filter`.

**Memoria.** Cada respuesta comprimida en curso mantiene un codificador. Pico de memoria del heap por encima del
servidor en reposo, medido con `crates/corvis/tests/compression_memory.rs`:

| Cuerpo | gzip / deflate | brotli |
|---|---|---|
| 1 KiB | unos 350 KiB | unos 560 KiB |
| Stream de 5 MiB de longitud desconocida | unos 410 KiB | unos 4,5 MiB (ventana de 512 KiB) |
| Archivo de 5 MiB de longitud conocida | unos 2,4 MiB* | unos 4,0 MiB (ventana de 512 KiB) |

\* Incluye la salida comprimida que recoge el cliente de pruebas; el servidor la envía en streaming.

**Clientes que dejan de leer.** Un cliente que abre una conexión, pide una respuesta comprimida y no la lee mantiene su
codificador, y su memoria, hasta que la conexión se cierra: Corvis aún no tiene un timeout de envío (está en
[#157](https://github.com/randygil/corvis/issues/157)). Con `br` y un archivo de 11,6 MiB, 500 conexiones HTTP/1 así
ocuparon unos 440 MiB con el límite de abajo (unos 0,85 MiB cada una), frente a 127 MiB sin compresión. Define
`ServerOptions::max_connections` cuando la compresión esté activada y calcúlalo con las cifras anteriores.

### Compresiones concurrentes

Como máximo `max_concurrent` respuestas se comprimen a la vez (por defecto, 8 veces los núcleos disponibles, mínimo 1).
Una respuesta toma un hueco al empezar y lo conserva hasta que su cuerpo termina, falla o se descarta. El límite acota
la memoria de arriba y la CPU que los clientes pueden pedir.

- `br` puede usar como máximo la mitad de los huecos (al menos uno), así que no deja sin huecos a `gzip`.
- Cuando no hay hueco para `br`, la respuesta usa `gzip` (o `deflate`) si el cliente lo acepta y hay un hueco libre para
  él. Si no, se envía sin comprimir, con estado 200 y sus cabeceras normales.
- Las respuestas que eran elegibles para compresión llevan siempre `Vary: Accept-Encoding`, también cuando el límite
  hizo que salieran sin comprimir, para que las cachés sigan siendo correctas.

**ETag fuertes.** Con `app.set_etag(EtagMode::Strong)` el `ETag` del cuerpo sin comprimir se conserva en el comprimido,
como en el middleware. El `ETag` débil por defecto es correcto para ambos.

## Cuándo sigue siendo mejor un proxy

Si ya tienes nginx, Caddy, Traefik, un balanceador o una CDN delante de la aplicación, comprimir allí no consume CPU de
los hilos que atienden peticiones y se configura una sola vez para todos los servicios:

```nginx
gzip on;
gzip_types application/json text/plain text/css application/javascript;
gzip_min_length 1024;
```

Tampoco comprimas [Server-Sent Events](/es/http/server-sent-events) en el proxy: podría acumular eventos en su búfer.
