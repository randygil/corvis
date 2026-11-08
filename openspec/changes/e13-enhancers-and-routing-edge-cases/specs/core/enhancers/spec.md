# Spec Delta

## Purpose

Order in which global enhancers registered as providers (`Provider::app_guard`, `app_interceptor`, `app_pipe`,
`app_filter`) take part in a request.

## ADDED Requirements

### Requirement: Global filters registered as providers keep registration order
When several exception filters are registered with `Provider::app_filter`, the framework SHALL try them in the order
they were registered (module scan order, then order inside a module's `providers`), after the route's and the
controller's filters and before the filters added with `use_global_filters`. The first filter whose `catches` returns
`true` MUST be the one that answers.

#### Scenario: Two provider filters catch the same exception
- **WHEN** a module registers `Provider::app_filter::<A>()` and then `Provider::app_filter::<B>()`, both catch every
  exception, and a handler fails
- **THEN** filter `A` answers the request

#### Scenario: Provider filters come before bootstrap filters
- **WHEN** a module registers `Provider::app_filter::<A>()`, the bootstrap code calls `use_global_filters(C)`, both
  catch every exception, and a handler fails
- **THEN** filter `A` answers the request

### Requirement: Global enhancers of every kind share the same ordering rule
Global guards, interceptors, pipes and filters registered as providers SHALL be applied in registration order and
ahead of the ones registered with the matching `use_global_*` method.

#### Scenario: Two provider guards
- **WHEN** a module registers `Provider::app_guard::<A>()` and then `Provider::app_guard::<B>()`
- **THEN** guard `A` runs before guard `B` on every route
