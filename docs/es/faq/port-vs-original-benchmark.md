---
status: documented
---

# Benchmark: una aplicación migrada frente a su original

El [benchmark sintético](/es/faq/corvis-vs-fastify-express-hono) mide el costo del framework en cuatro
endpoints triviales. Esta página mide lo que obtiene un equipo cuando migra un backend real: la misma
aplicación, escrita una vez en Node.js y otra en Corvis, que recibe las mismas peticiones contra la misma
base de datos en la misma máquina.

## Qué se midió

La aplicación es una API real de facturación (facturación, inventario y contabilidad) con 74 rutas sobre PostgreSQL, migrada
desde Node.js. La versión de Corvis pasa la suite de pruebas diferenciales contra el original (las mismas peticiones dan
el mismo status, los mismos headers y los mismos cuerpos, y las dos dejan la base de datos en el mismo estado), así que
ambos lados hacen el mismo trabajo. La aplicación es privada: esta página publica los números y el método, no su nombre
ni su código.

Cuatro escenarios, todos de solo lectura:

- **Comprobación de salud**: el camino del framework con sus guards e interceptores globales; sin autenticación ni base de datos.
- **Lista de productos**: autenticación y guards de roles, una consulta con el ORM y una respuesta JSON grande (100 productos).
- **Un producto por id**: validación del id, una búsqueda con el ORM y una respuesta JSON pequeña.
- **Vista previa de factura**: validación del cuerpo, varias lecturas de la base de datos y aritmética fiscal decimal para
  5 ítems y un pago; no se persiste nada.

El inicio de sesión queda fuera del conjunto por defecto porque mide el costo del hash de la contraseña, no el framework.

## Cómo se midió

- **Mismos datos, mismas peticiones.** Las dos bases de datos se crean desde un mismo template y se llenan a través de la
  API HTTP con un solo escenario. Cada petición medida se envía una vez a los dos servidores y se comparan las respuestas
  antes de medir nada; una diferencia detiene la ejecución.
- **Un servidor a la vez, un proceso nuevo por escenario**, con calentamiento. El generador de carga es
  [oha](https://github.com/hatoo/oha), limitado a 8 hilos.
- **Rendimiento**: bucle cerrado, 100 conexiones, 15 s, tres ejecuciones; se reporta la mediana.
- **Latencia**: bucle abierto con una tasa fija y corrección de latencia. La tasa es la mitad de lo que sostiene el original
  en la fase de rendimiento y es la misma para todos los objetivos, así que los dos lados trabajan por debajo de la
  saturación y los percentiles comparan el tiempo de servicio, no la longitud de la cola. Tres ejecuciones de 30 s; se
  reporta la mediana de cada percentil.
- **Memoria**: el conjunto residente del proceso del servidor en reposo, su pico durante el escenario (el contador de pico
  del sistema operativo más una serie de muestras cada 500 ms, para no perder un pico entre muestras) y después de la
  carga. La base de datos es compartida y no se cuenta.
- **Mismo pool de base de datos.** Los números principales dan a los dos lados un pool de 10 conexiones, de modo que la
  comparación no sea una comparación de tamaños de pool. La segunda tabla deja que cada lado conserve el tamaño de pool
  con el que viene (el ORM del original abre `2 x núcleos + 1` conexiones por defecto).
- Corvis se mide dos veces: con todos los núcleos (el runtime por defecto) y con un solo hilo de trabajo, que es la
  comparación por núcleo con un proceso de Node.js.

## Resultados

Las tablas se generan desde el `summary.json` confirmado de cada ejecución (`bench/port-vs-original/results/`) con
`report.mjs --docs`; no las edites a mano.

<!-- bench:port-vs-original:start -->

Ejecución del 2026-11-08 en Intel(R) Core(TM) i9-14900KF (32 hilos), Windows_NT 10.0.26300 (win32 x64). Commit de Corvis 1eb6156784c8. 3 ejecuciones por medición, 100 conexiones.

### Principal: el mismo tamaño de pool de base de datos en ambos lados (10 conexiones)

**Rendimiento (peticiones por segundo, mediana de las ejecuciones)**

| Escenario | Original (Node.js) | Corvis (1 hilo de trabajo) | Corvis (todos los núcleos) | Corvis (todos los núcleos) / original |
| --- | ---: | ---: | ---: | ---: |
| Comprobación de salud: sin autenticación ni base de datos | 24221 | 38121 | 120479 | 5,0x |
| Lista de productos (100 productos): autenticación, consulta con el ORM, JSON grande | 1512 | 1136 | 7592 | 5,0x |
| Un producto por id: validación del id, búsqueda con el ORM, JSON pequeño | 4467 | 6149 | 21140 | 4,7x |
| Vista previa de factura (5 ítems, 1 pago): validación del cuerpo, varias lecturas, aritmética decimal, sin persistir | 1075 | 1450 | 4445 | 4,1x |

**Latencia con una tasa fija (ms, mediana de las ejecuciones)**

| Escenario | Tasa (req/s) | Objetivo | Alcanzada (req/s) | p50 | p90 | p99 | max |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| Comprobación de salud: sin autenticación ni base de datos | 12111 | Original (Node.js) | 11087 | 4,57 | 807 | 2322 | 2537 |
|  |  | Corvis (1 hilo de trabajo) | 12110 | 1,76 | 3,28 | 5,36 | 15,9 |
|  |  | Corvis (todos los núcleos) | 12105 | 0,98 | 1,44 | 1,85 | 9,73 |
| Lista de productos (100 productos): autenticación, consulta con el ORM, JSON grande | 756 | Original (Node.js) | 756 | 2,27 | 3,19 | 6,63 | 29,4 |
|  |  | Corvis (1 hilo de trabajo) | 756 | 1,52 | 8,30 | 211 | 285 |
|  |  | Corvis (todos los núcleos) | 756 | 1,93 | 3,00 | 3,95 | 21,6 |
| Un producto por id: validación del id, búsqueda con el ORM, JSON pequeño | 2233 | Original (Node.js) | 2232 | 2,88 | 4,74 | 40,6 | 58,2 |
|  |  | Corvis (1 hilo de trabajo) | 2233 | 2,76 | 5,12 | 10,4 | 21,1 |
|  |  | Corvis (todos los núcleos) | 2233 | 1,77 | 2,26 | 2,89 | 28,2 |
| Vista previa de factura (5 ítems, 1 pago): validación del cuerpo, varias lecturas, aritmética decimal, sin persistir | 538 | Original (Node.js) | 538 | 2,40 | 4,36 | 6,36 | 29,1 |
|  |  | Corvis (1 hilo de trabajo) | 538 | 2,37 | 2,77 | 4,11 | 22,4 |
|  |  | Corvis (todos los núcleos) | 538 | 2,93 | 3,59 | 5,16 | 23,2 |

**Memoria del proceso del servidor (MB)**

| Escenario | Objetivo | En reposo | Pico | Después de la carga |
| --- | --- | ---: | ---: | ---: |
| Comprobación de salud: sin autenticación ni base de datos | Original (Node.js) | 174,1 | 234,7 | 234,7 |
|  | Corvis (1 hilo de trabajo) | 20,0 | 24,2 | 20,5 |
|  | Corvis (todos los núcleos) | 20,7 | 31,3 | 28,0 |
| Lista de productos (100 productos): autenticación, consulta con el ORM, JSON grande | Original (Node.js) | 176,3 | 456,3 | 449,6 |
|  | Corvis (1 hilo de trabajo) | 21,7 | 30,3 | 25,1 |
|  | Corvis (todos los núcleos) | 21,9 | 60,0 | 45,4 |
| Un producto por id: validación del id, búsqueda con el ORM, JSON pequeño | Original (Node.js) | 176,5 | 388,0 | 387,9 |
|  | Corvis (1 hilo de trabajo) | 20,8 | 29,1 | 23,8 |
|  | Corvis (todos los núcleos) | 21,9 | 46,3 | 37,4 |
| Vista previa de factura (5 ítems, 1 pago): validación del cuerpo, varias lecturas, aritmética decimal, sin persistir | Original (Node.js) | 178,5 | 427,6 | 426,8 |
|  | Corvis (1 hilo de trabajo) | 21,5 | 31,8 | 25,4 |
|  | Corvis (todos los núcleos) | 22,3 | 52,1 | 42,1 |

**Errores y estados inesperados**

Ninguno en ninguna ejecución.

### Secundaria: cada lado con el tamaño de pool que trae por defecto

Ejecución del 2026-11-08 en Intel(R) Core(TM) i9-14900KF (32 hilos), Windows_NT 10.0.26300 (win32 x64). Commit de Corvis 1eb6156784c8. 3 ejecuciones por medición, 100 conexiones.

**Rendimiento (peticiones por segundo, mediana de las ejecuciones)**

| Escenario | Original (Node.js) | Corvis (1 hilo de trabajo) | Corvis (todos los núcleos) | Corvis (todos los núcleos) / original |
| --- | ---: | ---: | ---: | ---: |
| Comprobación de salud: sin autenticación ni base de datos | 22090 | 40674 | 131837 | 6,0x |
| Lista de productos (100 productos): autenticación, consulta con el ORM, JSON grande | 1556 | 1192 | 7656 | 4,9x |
| Un producto por id: validación del id, búsqueda con el ORM, JSON pequeño | 4511 | 7503 | 21296 | 4,7x |
| Vista previa de factura (5 ítems, 1 pago): validación del cuerpo, varias lecturas, aritmética decimal, sin persistir | 2042 | 1633 | 4600 | 2,3x |

**Latencia con una tasa fija (ms, mediana de las ejecuciones)**

| Escenario | Tasa (req/s) | Objetivo | Alcanzada (req/s) | p50 | p90 | p99 | max |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| Comprobación de salud: sin autenticación ni base de datos | 11045 | Original (Node.js) | 11041 | 2,33 | 4,23 | 6,31 | 14,2 |
|  |  | Corvis (1 hilo de trabajo) | 11041 | 1,71 | 3,10 | 4,22 | 11,9 |
|  |  | Corvis (todos los núcleos) | 11039 | 0,99 | 1,50 | 1,92 | 7,67 |
| Lista de productos (100 productos): autenticación, consulta con el ORM, JSON grande | 778 | Original (Node.js) | 778 | 2,20 | 3,89 | 7,06 | 17,8 |
|  |  | Corvis (1 hilo de trabajo) | 778 | 1,96 | 6,41 | 27,4 | 55,7 |
|  |  | Corvis (todos los núcleos) | 778 | 1,96 | 3,04 | 4,18 | 19,1 |
| Un producto por id: validación del id, búsqueda con el ORM, JSON pequeño | 2256 | Original (Node.js) | 2255 | 2,84 | 4,69 | 7,27 | 18,1 |
|  |  | Corvis (1 hilo de trabajo) | 2256 | 2,57 | 3,61 | 4,54 | 16,2 |
|  |  | Corvis (todos los núcleos) | 2255 | 1,67 | 2,13 | 2,60 | 13,2 |
| Vista previa de factura (5 ítems, 1 pago): validación del cuerpo, varias lecturas, aritmética decimal, sin persistir | 1021 | Original (Node.js) | 1021 | 1,73 | 2,26 | 3,55 | 11,2 |
|  |  | Corvis (1 hilo de trabajo) | 1021 | 2,61 | 3,10 | 4,07 | 33,3 |
|  |  | Corvis (todos los núcleos) | 1021 | 2,58 | 2,98 | 3,60 | 30,5 |

**Memoria del proceso del servidor (MB)**

| Escenario | Objetivo | En reposo | Pico | Después de la carga |
| --- | --- | ---: | ---: | ---: |
| Comprobación de salud: sin autenticación ni base de datos | Original (Node.js) | 174,4 | 233,6 | 233,6 |
|  | Corvis (1 hilo de trabajo) | 19,8 | 24,1 | 20,3 |
|  | Corvis (todos los núcleos) | 21,0 | 32,3 | 29,0 |
| Lista de productos (100 productos): autenticación, consulta con el ORM, JSON grande | Original (Node.js) | 176,5 | 476,6 | 470,7 |
|  | Corvis (1 hilo de trabajo) | 21,3 | 30,4 | 24,5 |
|  | Corvis (todos los núcleos) | 21,8 | 59,1 | 43,2 |
| Un producto por id: validación del id, búsqueda con el ORM, JSON pequeño | Original (Node.js) | 177,2 | 390,3 | 389,4 |
|  | Corvis (1 hilo de trabajo) | 20,9 | 28,9 | 24,2 |
|  | Corvis (todos los núcleos) | 21,9 | 46,3 | 38,2 |
| Vista previa de factura (5 ítems, 1 pago): validación del cuerpo, varias lecturas, aritmética decimal, sin persistir | Original (Node.js) | 177,4 | 432,6 | 432,0 |
|  | Corvis (1 hilo de trabajo) | 21,2 | 31,5 | 25,1 |
|  | Corvis (todos los núcleos) | 22,2 | 52,7 | 42,4 |

**Errores y estados inesperados**

Ninguno en ninguna ejecución.

<!-- bench:port-vs-original:end -->

## Cómo leerlos

- Todo corre en una máquina: el generador de carga, los dos servidores y PostgreSQL (en un contenedor de Docker, que
  añade latencia a cada consulta por igual para ambos lados). Los números absolutos dependen de la máquina; lo que se
  conserva son las proporciones.
- Una proporción de rendimiento mayor que 1 significa que Corvis sirvió más peticiones por segundo que el original con
  la misma carga.
- La latencia se midió con una tasa fija por escenario. Si la tasa alcanzada por un objetivo es menor que esa tasa, el objetivo se saturó y sus percentiles incluyen tiempo en cola; léelos así.
- En esta ejecución el original no sostuvo la tasa fija del escenario de comprobación de salud (la tasa alcanzada es
  menor que la tasa en ambas tablas), así que su p90 y su p99 ahí incluyen tiempo en cola y dicen poco del tiempo de
  servicio. La lista de productos de Corvis con un solo hilo muestra una cola larga (p99) con la misma tasa en la que
  su mediana no la muestra.
- Los picos de memoria incluyen el arranque del proceso y el calentamiento, porque el pico es el contador del sistema
  operativo para toda la vida de un proceso iniciado para ese escenario.
- Cualquier error o estado inesperado aparece junto a la celda afectada y en la lista de errores; "Ninguno" significa que
  no hubo ninguno en ninguna ejecución.

## Límites

- Una aplicación, una máquina, un sistema operativo (Windows 11). Los números en Linux pueden ser distintos.
- La aplicación es privada, así que no puedes repetir exactamente este benchmark. El método, el escenario y el
  `summary.json` sin procesar de cada ejecución publicada están en el repositorio, en `bench/port-vs-original/`, y el
  script recibe la aplicación como parámetro.
- Escenarios de solo lectura: todavía no se miden las escrituras (emitir facturas).
- Un cambio en el framework entre ejecuciones cambia los números; cada tabla muestra la fecha y el commit de Corvis de su
  ejecución.

## Repetirlo

```bash
node bench/port-vs-original/run.mjs --pool equal
node bench/port-vs-original/run.mjs --pool default
node bench/port-vs-original/report.mjs --docs <equal summary.json> --secondary <default summary.json>
```

La ejecución completa tarda unos 35 minutos por modo de pool; `--quick` corre una versión corta para comprobar la
configuración y su salida queda marcada como no publicable. Los requisitos y las opciones están en
`bench/port-vs-original/README.md`.
