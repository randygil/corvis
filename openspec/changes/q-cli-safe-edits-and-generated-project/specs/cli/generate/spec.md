# Spec Delta

## ADDED Requirements

### Requirement: Registration edits leave source that parses
When `corvis generate` registers an element, it SHALL locate the `use` and `mod` items and the `#[module(...)]` lists by
parsing the file, insert only the new text, and keep comments, formatting and line endings. An element already
imported or registered MUST NOT be added again. If the file does not parse before or after the edit, the CLI MUST leave
it unchanged and print a `SKIP` line with the registration to add by hand.

#### Scenario: Import group split by rustfmt
- **WHEN** `src/app_module.rs` has a `use crate::{` group spread over several lines and the user runs
  `corvis g mo users`
- **THEN** the new `use` line follows the whole group, and the file parses and lists `UsersModule` once in `imports`

#### Scenario: Attribute text in a comment
- **WHEN** a comment above the module struct contains `#[module(providers = [Old])]`
- **THEN** the registration goes into the real attribute and the comment is unchanged

#### Scenario: Type already imported in a group
- **WHEN** `UsersService` is already imported through `use crate::users::{UsersController, UsersService};`
- **THEN** no second import of `UsersService` is added

#### Scenario: CRLF file
- **WHEN** the module file uses CRLF line endings
- **THEN** the inserted lines end with CRLF and every other byte of the file is unchanged

#### Scenario: File that does not parse
- **WHEN** the module file has a syntax error
- **THEN** it is not changed and the output has a `SKIP` line naming the file and the line to add

### Requirement: Files are written safely
`corvis new` and `corvis generate` SHALL create files only if they do not exist, replace edited files through a
temporary file in the same folder and a rename, and, when a step fails, remove the files and folders the command created
and restore the files it edited before reporting the error.

#### Scenario: Failure halfway through a resource
- **WHEN** `corvis g res cats` fails while writing its third file
- **THEN** the project's files are byte for byte what they were before the command, and the error is printed

#### Scenario: Existing file
- **WHEN** a file the command would create already exists
- **THEN** the command fails without changing that file

### Requirement: Generated routes under route posture
Generated controllers, resources and gateways SHALL have no guard and no `#[public]` by default. When the project's
crate root enables route posture, the CLI MUST print a `NOTE` listing the generated routes and handlers and how to accept
them. With `--public`, each generated route and handler SHALL carry `#[public]` and a `TODO` comment.

#### Scenario: Resource in an enforcing project
- **WHEN** `corvis g res cats` runs in a project created by `corvis new`
- **THEN** the output has a `NOTE` listing the five routes of `CatsController`, and `corvis posture` lists them as
  unprotected

#### Scenario: Public resource
- **WHEN** `corvis g res cats --public` runs in a project created by `corvis new`
- **THEN** the project builds, starts, and `corvis posture` exits with status 0

### Requirement: Entity names of a resource are singular
`corvis g res <name>` SHALL name the entity, its DTOs and messages after the singular of `<name>`, from ordered rules
and a short exception list, so that words ending in `ss`, `us` or `is` stay as they are; `--entity <Name>` MUST
override the rule and be validated like a name.

#### Scenario: Words the old rule broke
- **WHEN** the user runs `corvis g res statuses`, `corvis g res addresses` and `corvis g res houses`
- **THEN** the entities are `Status`, `Address` and `House`

#### Scenario: Explicit entity
- **WHEN** the user runs `corvis g res people --entity Person`
- **THEN** the entity is `Person` and the DTOs are `CreatePersonDto` and `UpdatePersonDto`
