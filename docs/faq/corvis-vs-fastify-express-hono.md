---
status: documented
---

# Corvis vs Fastify, Express and Hono

Express, Fastify and Hono are minimalist JavaScript/TypeScript HTTP frameworks: they give you a router,
a middleware or *plugin* system and the request/response, and leave the architecture in your hands.
Corvis is in a different category and a different language: it is an application framework for Rust that defines
how code is organized.

## What Corvis brings

| | Express / Fastify / Hono | Corvis |
|---|---|---|
| Language and runtime | JavaScript/TypeScript on Node.js (Hono also on other runtimes) | Rust, native binary on Tokio and hyper |
| Organization | Free | [Modules](/modules) with `imports`, `providers`, `controllers` and `exports` |
| Dependencies | Manual (or a separate library) | [Dependency injection](/providers) by type, resolved at startup |
| Request pipeline | Middleware / hooks | Middleware, [guards](/guards), [interceptors](/interceptors), [pipes](/pipes) and [exception filters](/exception-filters) |
| Validation | Separate library | `#[derive(Validate)]` and `corvis::z` (zod-style schemas) |
| Type errors | At runtime (or in the TypeScript compiler) | The Rust compiler checks handlers, DTOs and providers |
| Included modules | Package ecosystem | Configuration, JWT, events, scheduled tasks, cache, throttler, health checks, static files, WebSockets, testing |

If you come from Express, a good part of the observable behavior will feel familiar: the body
parser, the router, `ETag`s, `res.cookie`, file upload and CORS follow the semantics of
Express 5 and its usual packages, and the repository has tests that compare the responses with
those of a real Express application (`crates/corvis/tests/express_parity.rs`, `res.rs`, `multer.rs`).

## Measured performance

The `bench/` directory contains two applications with the same endpoints: a Corvis one in
`--release` and a Node.js one written with an equivalent modular framework, measured on Express and
on Fastify. They are measured with [oha](https://github.com/hatoo/oha) on `127.0.0.1`, with 100
*keep-alive* connections and 15 s per scenario. These are the results published in the repository README,
on an i9-14900KF (32 threads) with Windows 11, Node 24 and rustc 1.99, with client and server on the same
machine:

| Scenario | Node.js + Express | Node.js + Fastify | Corvis (1 worker) | Corvis (all cores) |
|---|---:|---:|---:|---:|
| `GET /` text | 713k req/min | 1.02M req/min | 1.54M req/min | 7.15M req/min |
| `GET /json` | 680k | 963k | 1.45M | 5.26M |
| `GET /users/:id` + `ParseIntPipe` | 647k | 879k | 1.48M | 7.59M |
| `POST /users` + `ValidationPipe` | 446k | 547k | 1.42M | 3.42M |
| p50 `GET /` | 8.98 ms | 4.46 ms | 4.03 ms | 0.73 ms |
| RSS under load | ~165–205 MB | ~163–344 MB | ~11 MB | ~9 MB |

How to read these numbers:

- The "1 worker" column is the per-core comparison with a Node.js process without cluster.
- The "all cores" column is limited mostly by the environment: the load generator and the Windows
  loopback saturate near 110–120k req/s.
- The `POST` row had more variance between runs.
- There are no measurements against Hono.

To repeat it on your machine: `bash bench/run.sh`. The detailed results are written to
`bench/results.md`.

## When it is not the best option

- If your team works in TypeScript and does not want to introduce Rust.
- If you need GraphQL or microservices with messaging transports: Corvis does not have them yet
  (see [GraphQL](/graphql/quick-start) and [Microservices](/microservices/basics)).
- If you are going to deploy to *edge* or *serverless* environments that expect JavaScript or WebAssembly (see
  [Cloudflare Workers](/faq/cloudflare-workers) and [Serverless](/faq/serverless)).
