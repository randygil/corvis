# cli/generate Specification

## Purpose
How `corvis generate` places generated files and registers them in the module of the target folder.

## Requirements

### Requirement: Registration goes to the folder's module
When `corvis generate` creates a controller, service or gateway in a folder `src/<dir>/` (the default folder or the
one given with `--path <dir>`), it SHALL register the type in the first existing file of:
`src/<dir>/<name>_module.rs`, then `src/<dir>/<dir>_module.rs`, then `src/app_module.rs`. With `--flat` it SHALL
register in `src/app_module.rs`. The CLI MUST print the path of the file it updated, and MUST NOT modify any other
module file.

#### Scenario: Controller in an existing feature folder
- **WHEN** `src/users/users_module.rs` exists and the user runs `corvis g controller profile --path users`
- **THEN** `ProfileController` is added to `controllers` of `UsersModule` in `src/users/users_module.rs`, with its
  `use` line, and `src/app_module.rs` is unchanged

#### Scenario: A module named after the element exists in the folder
- **WHEN** both `src/users/profile_module.rs` and `src/users/users_module.rs` exist and the user runs
  `corvis g service profile --path users`
- **THEN** `ProfileService` is registered in `src/users/profile_module.rs`

#### Scenario: No module in the folder
- **WHEN** `src/users/` has no `*_module.rs` and the user runs `corvis g service profile --path users`
- **THEN** `ProfileService` is registered in `src/app_module.rs`

#### Scenario: Default folder is unchanged
- **WHEN** `src/users/users_module.rs` exists and the user runs `corvis g service users` (no `--path`)
- **THEN** `UsersService` is registered in `UsersModule`, as before
