## ADDED Requirements

### Requirement: The exception factory accepts closures
`ValidationPipe::exception_factory` and `ValidationPipeOptions::exception_factory` SHALL accept any
`Fn(Vec<ValidationError>) -> HttpException + Send + Sync + 'static`, including a closure that captures values, and the
pipe SHALL call it with the validation errors instead of building its default exception.

#### Scenario: A factory that captures a status
- **WHEN** a `ValidationPipe` is built with a closure that captures a status code chosen at run time and a request fails
  validation
- **THEN** the response has that status and the body the closure built

#### Scenario: A function item
- **WHEN** a `ValidationPipe` is built with `.exception_factory(my_factory)` where `my_factory` is a plain function
- **THEN** it compiles and the pipe answers with what `my_factory` returns
