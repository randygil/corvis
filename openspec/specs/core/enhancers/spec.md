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
