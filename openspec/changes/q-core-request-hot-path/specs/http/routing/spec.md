# Spec Delta

## ADDED Requirements

### Requirement: Compiled middleware patterns match exactly as the pattern rules say
The patterns given to `for_routes` and `exclude` of module middleware SHALL be compiled once, when the application
initializes, and a compiled pattern MUST match a request path exactly when the pattern rules do: literals compared
ignoring ASCII case, one optional trailing slash, `:param` and `{param}` for one segment, whole-segment wildcards for zero
or more segments and `*` inside a segment spanning any characters. Matching a request MUST NOT depend on how many
patterns were declared.

#### Scenario: Equivalence with the pattern rules
- **WHEN** the property test generates 10,000 patterns and request paths from the pattern grammar, in mixed case and with
  empty segments and trailing slashes
- **THEN** every compiled pattern gives the same answer as the reference matcher

#### Scenario: Case and trailing slash cannot skip a middleware
- **WHEN** a middleware is applied with `for_routes("admin/*")` and a request is sent to `/ADMIN/users/`
- **THEN** the middleware runs

#### Scenario: A path no module middleware matches
- **WHEN** a module middleware is applied with `for_routes(CatsController)` and a request is sent to `/dogs`
- **THEN** the global middleware runs and the module middleware does not
