# Spec Delta

## ADDED Requirements

### Requirement: Handler results are recognized by type, not by name
The code generated for route, event, scheduled and gateway handlers SHALL treat a return value as a result with a value
and an error exactly when its type is `std::result::Result<T, E>`, whatever name or alias the signature uses. Any other
type SHALL be answered as a value, also when its name ends in `Result`.

#### Scenario: A DTO named like a result
- **WHEN** a `#[get]` handler returns a struct `SearchResult { total: 2 }` deriving `Serialize`
- **THEN** the controller compiles and the response is 200 with the body `{"total":2}`

#### Scenario: A result alias without the suffix
- **WHEN** a handler returns `type Out = Result<CatDto, NotFound>` and the value is `Err(NotFound)`, where both types
  derive `Serialize` and `NotFound` converts to a 404 exception
- **THEN** the response is 404 with the exception's body, not 200 with `{"Err":...}`

#### Scenario: Event handler returning a struct named like a result
- **WHEN** an `#[on_event]` handler returns a struct `TaskResult`
- **THEN** the handler compiles and runs on every emit
