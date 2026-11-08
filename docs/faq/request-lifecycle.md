---
status: documented
---

# Request lifecycle

An HTTP request goes through several layers before reaching the handler and just as many on the way back. Knowing
in which order they run helps you decide where to put each piece (does this go in a middleware or in a
guard?) and understand why an error was answered in a certain way.

## Summary

1. CORS: immediate response to `OPTIONS` preflights.
2. Body parser (its errors go straight to the global filters).
3. Global middleware (`app.use_middleware`, `app.use_fn`).
4. Module middleware (`CorvisModule::configure`).
5. Static files from `ServeStaticModule` and route resolution (`404` if there is none).
6. Guards: global → controller → method.
7. Interceptors, "before" part: global → controller → method.
8. Pipes: global → controller → method → parameter.
9. Handler.
10. Interceptors, "after" part, in reverse order.
11. Exception filters, if something failed: method → controller → global.
12. CORS headers and writing the response.

## Middleware

Global middleware runs first, in the order it was registered in the bootstrap code. Then come the middleware of the modules that implement `CorvisModule::configure` and whose
`for_routes` matches the route, starting with the root module and continuing through its imports.

A middleware receives the `Request` and a `Next`, and decides whether to continue (`next.run(req).await`) or respond
itself. What `next.run` returns is already the route's final response, with filters applied;
the only errors that can reach it as `Err` are those produced before entering the route
(for example, `404 Cannot GET /x`).

`corvis::serve_static(...)` registered with `app.use_middleware` is just another global middleware, so
it runs at its registration position. `ServeStaticModule`, on the other hand, is consulted after all
the middleware and before resolving the route.

## Guards

They run in this order and stop at the first one that returns `false` (a
`403 Forbidden resource` response) or an error:

1. Global ones registered as a provider with `Provider::app_guard`, in module order.
2. Global ones registered with `app.use_global_guards(...)`, in call order.
3. Controller ones (`#[use_guards]` below `#[controller]`), left to right.
4. Method ones.

Guards run **before** pipes: they see the unvalidated body.

## Interceptors

They wrap the rest of the chain. The entry order is the same as for guards (global
providers, `use_global_interceptors`, controller, method), and the exit order is the reverse: the first
interceptor to enter is the last to see the response. That is why an interceptor registered as a
provider (`Provider::app_interceptor`) wraps those of `use_global_interceptors`; this is verified in
`crates/corvis/tests/enhancer_order.rs`.

An interceptor can skip calling `next.handle()` and return its own response (this is what
`CacheInterceptor` does on a cache hit).

## Pipes

For each handler parameter (`#[body]`, `#[param]`, `#[query]`, ...), the following run in a chain: the global
pipes (those from `Provider::app_pipe` first), the controller's, the method's and finally
the parameter's own (`#[param("id", ParseIntPipe)]`). Each pipe's output is the next one's
input, and the final output is deserialized into the parameter's type.

All parameters are processed; if several fail, the error from the earliest pipe in its chain wins
and, on a tie, the one from the last declared parameter.

Parameters without an attribute (types that implement `FromContext`) do not go through pipes.

## Exception filters

If a guard, interceptor, pipe or the handler returns an error (or panics), Corvis looks for a
filter in this order:

1. The method's (`#[use_filters]` on the handler).
2. The controller's.
3. The global ones, last registered first: those from `app.use_global_filters(...)` (from the last call
   to the first), then those registered with `Provider::app_filter` (from the last to the first).

The **first** filter whose `catches(&exception)` returns `true` is used. If none accepts it,
`BaseExceptionFilter` responds with the standard body
(`{"message":...,"error":...,"statusCode":...}` or `{"statusCode":500,"message":"Internal server error"}`).

Errors that occur outside a route (in a middleware, when resolving the route or in the body parser)
only go through the global filters.

## Response

The default status is `201` on `POST` and `200` on the rest, unless `#[http_code(...)]` is set. It is set before the route runs, so guards and interceptors see it with
`ctx.switch_to_http().status_code()`. After
the filters, the CORS headers are applied (those set by the handler take priority), the `ETag` and
`Content-Length` on in-memory bodies, and the `304` response if the conditional request is still
valid.

## WebSockets

In a gateway, each message goes through guards → interceptors → pipes → handler, with the gateway's and the handler's
enhancers. Global enhancers are not applied and exception filters are not
supported: errors are sent to the client as an `exception` event. See
[Gateways](/websockets/gateways).
