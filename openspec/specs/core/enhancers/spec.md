# core/enhancers Specification

## Purpose
Order in which global enhancers registered as providers (`Provider::app_guard`, `app_interceptor`, `app_pipe`,
`app_filter`) take part in a request.

## Requirements

### Requirement: Global exception filters are tried last registered first
Global exception filters SHALL be tried in reverse registration order, like the original framework (verified against
it): the filters added with `use_global_filters` from the last call to the first, then the filters registered with
`Provider::app_filter` from the last (module scan order, then order inside a module's `providers`) to the first. They
come after the route's and the controller's filters. The first filter whose `catches` returns `true` MUST be the one
that answers.

#### Scenario: Two provider filters catch the same exception
- **WHEN** a module registers `Provider::app_filter::<A>()` and then `Provider::app_filter::<B>()`, both catch every
  exception, and a handler fails
- **THEN** filter `B` answers the request

#### Scenario: Bootstrap filters come before provider filters
- **WHEN** a module registers `Provider::app_filter::<A>()`, the bootstrap code calls `use_global_filters(C)`, both
  catch every exception, and a handler fails
- **THEN** filter `C` answers the request

#### Scenario: Several bootstrap filters
- **WHEN** the bootstrap code calls `use_global_filters(C)` and then `use_global_filters(D)`, both catch every
  exception, and a handler fails
- **THEN** filter `D` answers the request

### Requirement: Global guards, interceptors and pipes registered as providers keep registration order
Global guards, interceptors and pipes registered as providers SHALL be applied in registration order and ahead of the
ones registered with the matching `use_global_*` method.

#### Scenario: Two provider guards
- **WHEN** a module registers `Provider::app_guard::<A>()` and then `Provider::app_guard::<B>()`
- **THEN** guard `A` runs before guard `B` on every route

### Requirement: Route attributes on a method without a route attribute are a compile error
In a `#[routes]` block, when a method carries an attribute that `#[routes]` consumes (any `#[use_*]` enhancer,
`#[set_metadata]`, `#[http_code]`, `#[header]`, `#[redirect]`, `#[version]`, `#[apply_decorators]`, `#[api_*]`, and
attributes added to that set later) but no route attribute (an HTTP method attribute such as `#[get]`, or `#[sse]`),
the macro SHALL fail compilation with an error spanned on the first such
attribute.

#### Scenario: Guard on a helper method
- **WHEN** a `#[routes]` block contains `#[use_guards(AdminGuard)] async fn purge(&self) -> HttpResult<()>` with no
  route attribute
- **THEN** compilation fails with
  ``` `purge` has #[use_guards] but no route attribute (#[get], #[post], #[put], #[patch], #[delete], #[options], #[head], #[search], #[all] or #[sse]); add one or remove #[use_guards] ```

#### Scenario: Non-enhancer route attribute on a helper method
- **WHEN** a `#[routes]` block contains `#[http_code(204)] fn reset(&self)` with no route attribute
- **THEN** compilation fails with a message that starts with `` `reset` has #[http_code] but no route attribute ``

#### Scenario: Plain helper method
- **WHEN** a `#[routes]` block contains `fn normalize(&self, s: &str) -> String` with no attributes, or only
  attributes `#[routes]` does not consume (such as `#[allow(dead_code)]` or `#[inline]`)
- **THEN** the block compiles and the method is not registered as a route

#### Scenario: Route method unchanged
- **WHEN** the method from the first scenario also carries `#[delete("cache")]`
- **THEN** the block compiles and `DELETE /<controller>/cache` runs `AdminGuard` before the handler, as before this
  change

### Requirement: The routes error names the method and the attribute
The message MUST name the method and the attribute and MUST say that a route attribute is missing:
``` `<method>` has #[<attribute>] but no route attribute (#[get], #[post], #[put], #[patch], #[delete], #[options], #[head], #[search], #[all] or #[sse]); add one or remove #[<attribute>] ```.
Methods without any of these attributes MUST still be accepted as plain helper methods.

#### Scenario: Message of a guarded helper
- **WHEN** a `#[routes]` method `audit` has `#[use_guards(AuthGuard)]` and no route attribute
- **THEN** compilation fails with a message that starts with "`audit` has #[use_guards] but no route attribute"

### Requirement: Enhancers on a method without a handler attribute in `#[handlers]` are a compile error
In a `#[handlers]` block, when a method carries one or more attributes that `#[handlers]` consumes as enhancers or
metadata (`#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]`, `#[set_metadata]`, and any attribute added to that
set later) and carries no `#[subscribe_message]`, the macro SHALL fail compilation with an error spanned on the first such
attribute. A method with `#[on_event]`, `#[cron]`, `#[interval]` or `#[timeout]` plus an enhancer is rejected by the
same rule, with the existing message below; a method with none of those attributes gets the new message.

#### Scenario: Guard on a gateway helper method
- **WHEN** a `#[handlers]` block contains `#[use_guards(WsAuthGuard)] fn kick_idle(&self)` with no handler attribute
- **THEN** compilation fails with
  ``` `kick_idle` has #[use_guards] but no #[subscribe_message] attribute; enhancers and metadata in `#[handlers]` only apply to message handlers ```

#### Scenario: Enhancer on another handler kind unchanged
- **WHEN** a `#[handlers]` block contains `#[use_guards(G)] #[on_event("x")] fn on_x(&self)`
- **THEN** compilation fails with the existing message
  ``` enhancers in `#[handlers]` are only supported on #[subscribe_message] methods ```

### Requirement: The handlers error names the method and the attribute
The message MUST
name the method and the attribute and MUST say that `#[subscribe_message]` is missing:
``` `<method>` has #[<attribute>] but no #[subscribe_message] attribute; enhancers and metadata in `#[handlers]` only apply to message handlers ```.

#### Scenario: Message of a guarded helper in a gateway
- **WHEN** a `#[handlers]` method `cleanup` has `#[use_guards(WsGuard)]` and no handler attribute
- **THEN** compilation fails with a message that starts with "`cleanup` has #[use_guards] but no #[subscribe_message]
  attribute"

### Requirement: All discarded-attribute errors of a block are reported together
When several methods of one `#[routes]` or `#[handlers]` block trigger the errors above, the macro SHALL report one
error per such method in the same compilation, instead of stopping at the first one.

#### Scenario: Two helper methods with enhancers
- **WHEN** a `#[routes]` block has `#[use_guards(A)] fn a(&self)` and `#[use_pipes(P)] fn b(&self)`, neither with a
  route attribute
- **THEN** one compilation reports both errors, one naming `a` and one naming `b`

### Requirement: Route and controller filters are tried in the original framework's order
For an HTTP route, the framework SHALL try exception filters in this order and use the first one whose `catches`
returns `true`: the method's filters, then the controller's filters, then the global filters (in the order of the
requirement "Global exception filters are tried last registered first"). Within one level, the filters of one
`#[use_filters(...)]` attribute MUST be tried from the last listed to the first, and when several `#[use_filters]`
attributes are stacked on the same method or controller, the filters of the upper attribute MUST be tried before
those of the attributes below it. The array form `#[use_filters([A, B])]` MUST behave as `#[use_filters(A, B)]`. This
is the order of the original framework, verified with a differential fixture.

#### Scenario: Two filters in one method attribute
- **WHEN** a route declares `#[use_filters(First, Second)]`, both filters catch every exception, and the handler fails
- **THEN** `Second` answers the request and `First` is not called

#### Scenario: Two filters in one controller attribute
- **WHEN** a controller declares `#[use_filters(First, Second)]` below `#[controller]`, its route declares no filter,
  both filters catch every exception, and the handler fails
- **THEN** `Second` answers the request

#### Scenario: Stacked attributes
- **WHEN** a route declares `#[use_filters(A, B)]` and, on the next line, `#[use_filters(C)]`, and the three filters
  catch the exception
- **THEN** the filters are tried in the order `B`, `A`, `C`, so `B` answers

#### Scenario: Method level before controller level
- **WHEN** a controller declares `#[use_filters(C1, C2)]`, its route declares `#[use_filters(M1, M2)]`, and only `M1`
  and `C2` catch the raised exception
- **THEN** `M1` answers the request

#### Scenario: Array form
- **WHEN** a route declares `#[use_filters([First, Second])]`, both filters catch every exception, and the handler
  fails
- **THEN** `Second` answers the request

### Requirement: Enhancers can set response headers through the execution context
The framework SHALL provide `HttpArgumentsHost::set_response_header(name, value) -> bool`, reached through
`ctx.switch_to_http()`, for guards, interceptors and pipes that receive the route's execution context. It MUST record
the header for the current request and return `true`; a later call with the same name MUST replace the recorded value.

#### Scenario: Guard sets a header on a successful request
- **WHEN** a guard calls `set_response_header("X-Trace", "abc")` and returns `true`, and the handler returns a JSON body
- **THEN** the response carries `X-Trace: abc`

### Requirement: Invalid headers are rejected
`set_response_header` SHALL reject a name or value that is not a valid HTTP header, including values containing CR or
LF: it MUST return `false` and record nothing.

#### Scenario: Header injection attempt
- **WHEN** a guard calls `set_response_header("X-Trace", "a\r\nSet-Cookie: x=1")`
- **THEN** the call returns `false` and the response carries neither `X-Trace` nor the injected `Set-Cookie`

### Requirement: Setting a response header in a WebSocket context does nothing
In a WebSocket context `set_response_header` SHALL record nothing and return `false`.

#### Scenario: Gateway message
- **WHEN** a guard running on a gateway message calls `set_response_header("X-Trace", "abc")`
- **THEN** the call returns `false` and the message is handled normally

### Requirement: Enhancer headers never replace headers the response already has
The framework SHALL add each recorded header to the final response only when the response has no header with that
name. Headers set by the handler, by a returned `Response`, by `#[header]` on the route, by an exception filter, or on
`Res` by a `#[res]` handler MUST win over enhancer headers.

#### Scenario: Route header wins
- **WHEN** a route declares `#[header("X-Trace", "route")]` and a guard sets `X-Trace: guard`
- **THEN** the response carries `X-Trace: route`

#### Scenario: Handler header wins
- **WHEN** a guard sets `X-Trace: guard` and the handler returns `Response::json(..).with_header("X-Trace", "handler")`
- **THEN** the response carries `X-Trace: handler`

### Requirement: Enhancer headers are kept on successful and error responses
The framework SHALL merge the recorded headers into the route's response on success, on an error handled by the
built-in filter, and on an error handled by an exception filter (route, controller or global), including headers
recorded by an interceptor after the handler ran.

#### Scenario: Error without filter
- **WHEN** a guard sets `X-Trace: abc` and the handler returns `Err(NotFoundException::new("x"))`
- **THEN** the 404 response carries `X-Trace: abc`

#### Scenario: Error handled by a filter
- **WHEN** a guard sets `X-Trace: abc` and `X-Other: 1`, the handler fails, and a filter returns a response with
  `X-Trace: filter`
- **THEN** the response carries `X-Trace: filter` and `X-Other: 1`

#### Scenario: Interceptor sets a header after the handler
- **WHEN** an interceptor calls `set_response_header("X-Elapsed", "5")` after `next.handle().await`
- **THEN** the response carries `X-Elapsed: 5`

### Requirement: Enhancer headers reach `#[res]` routes
For routes with a `#[res]` parameter, headers recorded before the `Res` is created SHALL be copied into it after the
route's `#[header]` values, and headers recorded afterwards MUST be written to the `Res` when it has no header with
that name and its headers were not sent yet. Headers recorded after the response was sent MUST be ignored.

#### Scenario: `#[res]` passthrough
- **WHEN** a route with a `#[res(passthrough)] res: Res` parameter returns a value and a guard set `X-Trace: abc`
- **THEN** the response carries `X-Trace: abc`

#### Scenario: `#[res]` handled by the handler
- **WHEN** a route with a `#[res] res: Res` parameter calls `res.send("ok")` and a guard set `X-Trace: abc`
- **THEN** the response carries `X-Trace: abc`, and a handler that called `res.set("X-Trace", "mine")` before sending
  gets `X-Trace: mine`
