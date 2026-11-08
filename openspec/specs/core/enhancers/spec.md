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
