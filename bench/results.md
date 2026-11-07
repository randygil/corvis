# Benchmark de throughput: NestJS vs Corvis

Generado: 2026-11-07 02:54:48 -0400

## Entorno

- SO: Microsoft Windows 11 Pro 10.0.26300.0
- CPU: Intel(R) Core(TM) i9-14900KF (32 hilos lógicos)
- Node: v24.13.0 · rustc: 1.99.0
- Generador de carga: oha 1.16.0 (8 hilos)
- Parámetros: 100 conexiones, 15s por escenario, warmup de 5s, HTTP/1.1 keep-alive, 127.0.0.1
- Cliente y servidor corren en la misma máquina (compiten por CPU).
- Corvis: build `--release`, runtime Tokio multi-hilo (un worker por núcleo salvo que se indique `TOKIO_WORKER_THREADS`).
- NestJS: `node dist/main.js` (compilado con tsc), un solo proceso (sin cluster).

## Resultados

| Escenario | Framework | req/s | req/min | p50 (ms) | p99 (ms) | Errores / non-2xx | RSS bajo carga (MB) |
|---|---|---:|---:|---:|---:|---:|---:|
| GET / (texto) | Corvis (Tokio, todos los núcleos) | 119,240 | 7,154,419 | 0.73 | 2.59 | 0 / 0 | 8.6 |
| GET / (texto) | Corvis (Tokio, 1 worker) | 25,617 | 1,537,028 | 4.03 | 10.15 | 0 / 0 | 10.6 |
| GET / (texto) | NestJS + Express | 11,886 | 713,141 | 8.98 | 20.11 | 0 / 0 | 165.7 |
| GET / (texto) | NestJS + Fastify | 16,967 | 1,017,999 | 4.46 | 16.81 | 0 / 0 | 163.1 |
| GET /json | Corvis (Tokio, todos los núcleos) | 87,644 | 5,258,627 | 0.81 | 6.27 | 0 / 0 | 8.6 |
| GET /json | Corvis (Tokio, 1 worker) | 24,156 | 1,449,350 | 4.30 | 10.63 | 0 / 0 | 10.6 |
| GET /json | NestJS + Express | 11,335 | 680,115 | 6.77 | 25.87 | 0 / 0 | 168.1 |
| GET /json | NestJS + Fastify | 16,048 | 962,865 | 4.44 | 19.35 | 0 / 0 | 162.7 |
| GET /users/:id (ParseIntPipe) | Corvis (Tokio, todos los núcleos) | 126,470 | 7,588,227 | 0.72 | 2.00 | 0 / 0 | 8.6 |
| GET /users/:id (ParseIntPipe) | Corvis (Tokio, 1 worker) | 24,688 | 1,481,296 | 4.28 | 10.95 | 0 / 0 | 11.4 |
| GET /users/:id (ParseIntPipe) | NestJS + Express | 10,790 | 647,383 | 6.70 | 27.16 | 0 / 0 | 200.6 |
| GET /users/:id (ParseIntPipe) | NestJS + Fastify | 14,645 | 878,727 | 5.29 | 20.35 | 0 / 0 | 194.1 |
| POST /users (ValidationPipe) | Corvis (Tokio, todos los núcleos) | 56,972 | 3,418,338 | 1.09 | 12.97 | 0 / 0 | 8.6 |
| POST /users (ValidationPipe) | Corvis (Tokio, 1 worker) | 23,692 | 1,421,493 | 4.02 | 8.42 | 0 / 0 | 10.7 |
| POST /users (ValidationPipe) | NestJS + Express | 7,425 | 445,530 | 15.20 | 25.43 | 0 / 0 | 205.3 |
| POST /users (ValidationPipe) | NestJS + Fastify | 9,113 | 546,805 | 9.11 | 31.89 | 0 / 0 | 344.1 |

## Relación de throughput (respecto a NestJS + Express)

| Escenario | Corvis (Tokio, todos los núcleos) | Corvis (Tokio, 1 worker) | NestJS + Express | NestJS + Fastify |
|---|---:|---:|---:|---:|
| GET / (texto) | 10.03x | 2.16x | 1.00x | 1.43x |
| GET /json | 7.73x | 2.13x | 1.00x | 1.42x |
| GET /users/:id (ParseIntPipe) | 11.72x | 2.29x | 1.00x | 1.36x |
| POST /users (ValidationPipe) | 7.67x | 3.19x | 1.00x | 1.23x |

## Memoria RSS en reposo (tras arrancar, antes de la carga)

| Framework | RSS en reposo (MB) | RSS máximo observado bajo carga (MB) |
|---|---:|---:|
| Corvis (Tokio, todos los núcleos) | 8.6 | 8.6 |
| Corvis (Tokio, 1 worker) | 8.0 | 11.4 |
| NestJS + Express | 139.7 | 205.3 |
| NestJS + Fastify | 138.0 | 344.1 |

_RSS = working set de Windows (`tasklist`), muestreado varias veces durante cada medición; es aproximado._

## Sanidad

- Todas las mediciones terminaron con 0 errores de conexión y 0 respuestas non-2xx.
- Antes de medir, run.sh comprueba con curl que cada endpoint devuelve exactamente el cuerpo y el código esperados en cada framework.

## Cómo interpretar

- **Corvis (todos los núcleos)** frente a NestJS es la comparación "tal cual sale de la caja": Tokio usa todos los núcleos y Node ejecuta el JS en un solo hilo. En esta máquina, con cliente y servidor en el mismo equipo, el techo de oha + loopback de Windows ronda los ~100-110k req/s (un servidor hyper sin framework se queda en esa misma cifra), así que esa fila mide sobre todo el techo del entorno, no el límite de Corvis.
- **Corvis (1 worker)** (`TOKIO_WORKER_THREADS=1`) es la comparación por núcleo, más justa frente a un proceso Node sin cluster.
- req/min = req/s × 60 (extrapolado de la ventana medida).

## Cómo reejecutar

```bash
# Desde la raíz del workspace, en Git Bash
bash bench/run.sh

# Parámetros opcionales (variables de entorno)
CONNS=100 DURATION=15 WARMUP=5 OHA_THREADS=8 FRAMEWORKS="corvis corvis-1t nest-express nest-fastify" bash bench/run.sh
```
