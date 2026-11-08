---
status: documented
---

# Introduction

<p class="cv-lead">Corvis is a modular backend framework for Rust. It organizes an application into modules, resolves the dependencies between services with an injection container, and processes each request through an explicit chain of middleware, guards, interceptors, pipes, and exception filters.</p>

If you have worked with decorator-based Node.js frameworks written in TypeScript, the vocabulary will feel familiar: decorators are Rust attributes (`#[get(":id")]`), classes are `struct`s, and the constructor is generated from the struct's `Arc<T>` fields. If you haven't, no prior knowledge is needed: this documentation explains each piece from scratch.

## What it solves

Rust has excellent pieces for building HTTP servers (hyper, tokio, serde), but it leaves the **architecture** up to you: where each service lives, who creates it, how it is shared, how to test it in isolation, and how to apply authentication or validation to many routes at once. Corvis answers those questions with a single model:

- **Modules** that group controllers and providers and decide what they expose to other modules.
- **Dependency injection** by type: an `Arc<UsersService>` field is enough to receive the service.
- Declarative **controllers**: routes, parameters, status codes, and headers with attributes.
- A **request pipeline** made of reusable pieces that can be tested separately.
- **Built-in modules** for what almost every API needs: configuration, JWT, events, scheduled tasks, caching, rate limiting, health checks, static files, file uploads, and WebSockets.

## At a glance

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

`CatsController` does not create its service: it declares it as a field and the container hands it over when the controller is built. `ParseIntPipe` converts and validates the parameter before the method is called, and an `Err(NotFoundException)` becomes a `404` response with a JSON body.

## Where to go next

<div class="cv-cards">
  <a href="/first-steps"><strong>First steps</strong><span>Create a project with the CLI and understand its structure.</span></a>
  <a href="/providers"><strong>Providers</strong><span>Services, injection by type, and tokens.</span></a>
  <a href="/modules"><strong>Modules</strong><span>Encapsulation, exports, global and dynamic modules.</span></a>
  <a href="/websockets/gateways"><strong>WebSockets</strong><span>Gateways with rooms, guards, and acks.</span></a>
</div>

## The reference example

Many pages use **Tertulia** (`examples/tertulia` in the repository) as their example: a real-time chat with rooms, presence, JWT authentication, validation with `corvis::z`, SQLite history with `corvis-orm`, REST endpoints that read the gateway state, Server-Sent Events, and a plain WebSocket for moderators.

```bash
JWT_SECRET=a-long-secret cargo run -p tertulia   # http://127.0.0.1:5150
```

The repository also includes `examples/cats-app` (authentication, roles, versioning, caching, events, and scheduled tasks) and `examples/blog-orm` (a REST API on top of `corvis-orm`). See [Examples](/faq/examples).

## Project status

Corvis is at version `0.1.0`. Pages marked **Not available yet** describe capabilities the framework doesn't have yet (for example GraphQL or microservices); they stay in the menu so the structure of the documentation is stable, and each one points to the closest alternative that does exist.
