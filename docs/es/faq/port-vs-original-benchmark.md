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

Resultados pendientes de la ejecución publicable.

<!-- bench:port-vs-original:end -->

## Cómo leerlos

- Todo corre en una máquina: el generador de carga, los dos servidores y PostgreSQL (en un contenedor de Docker, que
  añade latencia a cada consulta por igual para ambos lados). Los números absolutos dependen de la máquina; lo que se
  conserva son las proporciones.
- Una proporción de rendimiento mayor que 1 significa que Corvis sirvió más peticiones por segundo que el original con
  la misma carga.
- La latencia se midió con una tasa fija por escenario. Si la tasa alcanzada por un objetivo es menor que esa tasa, el objetivo se saturó y sus percentiles incluyen tiempo en cola; léelos así.
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
