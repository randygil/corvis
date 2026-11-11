# Spec Delta

## ADDED Requirements

### Requirement: Resolving an existing request-scoped instance builds nothing
When a request-scoped provider already has an instance for the request being dispatched, resolving it again (through
`Scoped<T>`, `ModuleRef::resolve_for` or a dependency) SHALL return that instance without building a resolution context,
calling the provider's factory or creating its future. The checks that apply before a resolution (outside a request,
cycles, depth) MUST still apply.

#### Scenario: Two Scoped parameters
- **WHEN** a handler declares two `Scoped<RequestContext>` parameters of a request-scoped provider
- **THEN** the factory runs once, both parameters are the same instance, and the second resolution allocates nothing
  beyond the returned handle

#### Scenario: Outside a request
- **WHEN** `ModuleRef::resolve_for` is called for a request that was never dispatched
- **THEN** it fails with the outside-request error, as before
