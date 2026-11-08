# Spec Delta

## ADDED Requirements

### Requirement: New Project passes the configured framework path
The VS Code extension SHALL contribute the setting `corvis.new.corvisPath` (string, default empty). When the setting
is not empty and valid, the **Corvis: New Project** command MUST run `corvis new <name>` with the environment variable
`CORVIS_PATH` set to the resolved folder, so the generated `Cargo.toml` declares `corvis = { path = "<folder>" }`.
The variable MUST be set only for that child process and only for `corvis new`.

#### Scenario: Setting points at the crate folder
- **WHEN** `corvis.new.corvisPath` is `C:/src/corvis/crates/corvis` and the user runs **Corvis: New Project** with the
  name `my-app`
- **THEN** the CLI runs with `CORVIS_PATH=C:/src/corvis/crates/corvis`, `my-app/Cargo.toml` contains
  `corvis = { path = "C:/src/corvis/crates/corvis" }`, and the "Corvis" output channel shows the CLI's
  `NOTE using CORVIS_PATH=...` line

#### Scenario: Setting points at the repository root
- **WHEN** `corvis.new.corvisPath` is `~/src/corvis`, a folder that contains `crates/corvis-cli/Cargo.toml`
- **THEN** the CLI runs with `CORVIS_PATH` set to the absolute path of `~/src/corvis/crates/corvis`

#### Scenario: Other commands are unchanged
- **WHEN** `corvis.new.corvisPath` is set and the user runs a **Corvis: Generate…** command or **Start Application**
- **THEN** the extension does not add `CORVIS_PATH` to that CLI process

### Requirement: An empty setting keeps the CLI default
When `corvis.new.corvisPath` is empty or only whitespace, the extension MUST NOT add or remove `CORVIS_PATH`: the CLI
receives the extension host's environment unchanged, and its own rules (`cli/new`) pick the dependency.

#### Scenario: Default setting
- **WHEN** `corvis.new.corvisPath` is empty, VS Code was started without `CORVIS_PATH`, and the user runs **Corvis: New
  Project**
- **THEN** the generated `Cargo.toml` contains the CLI's version requirement, as when `corvis new` runs in a terminal

#### Scenario: Variable inherited from the shell
- **WHEN** `corvis.new.corvisPath` is empty and VS Code was started from a shell that exports `CORVIS_PATH`
- **THEN** the CLI receives that inherited value

### Requirement: Invalid framework paths are rejected before running the CLI
The extension MUST reject a non-empty `corvis.new.corvisPath` that is not absolute after `~` expansion, or whose
resolved folder has no `Cargo.toml`. It SHALL then show an error message that names `corvis.new.corvisPath` and offers
to open the settings, and it MUST NOT run `corvis new`.

#### Scenario: Relative path
- **WHEN** `corvis.new.corvisPath` is `../corvis/crates/corvis` and the user runs **Corvis: New Project**
- **THEN** an error naming `corvis.new.corvisPath` and saying the path must be absolute is shown, and no project folder
  is created

#### Scenario: Folder without a manifest
- **WHEN** `corvis.new.corvisPath` points at an existing folder with no `Cargo.toml`
- **THEN** an error naming `corvis.new.corvisPath` and the folder is shown, and the CLI is not run
