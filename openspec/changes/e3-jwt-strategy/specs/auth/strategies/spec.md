# Spec Delta

## ADDED Requirements

### Requirement: Composite guards expose inner guards to the startup check
A guard SHALL be able to list the guards it delegates to (`CanActivate::inner_guards`); the startup check of strategy names MUST look inside them, at any depth. Listing an inner guard MUST NOT make the gateway warning fire for the wrapper.

#### Scenario: Wrapper with a typo
- **WHEN** a guard that lists an inner `AuthGuard::new("headr")` is used on a route and no strategy has that name
- **THEN** the application does not start and the error names the route and the name

#### Scenario: Wrapper with a registered name
- **WHEN** the inner guard names a registered strategy
- **THEN** the application starts
