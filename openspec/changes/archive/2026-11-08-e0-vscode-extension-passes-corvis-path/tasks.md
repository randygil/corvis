# Tasks

## 1. Path helper

- [x] 1.1 Add `resolveCorvisPath(value, home, exists)` to `editors/vscode/src/paths.ts` (design.md, decisions 2-4):
  empty or whitespace -> `{}`; leading `~` expanded with `home`; not absolute -> `{ error }` naming
  `corvis.new.corvisPath`; a framework root (`<v>/crates/corvis-cli/Cargo.toml` exists) -> `<v>/crates/corvis`; no
  `<resolved>/Cargo.toml` -> `{ error }` naming the folder; otherwise `{ path }`. Keep the file free of `vscode`
  imports. Verify: `npm run compile` in `editors/vscode` has no errors
- [x] 1.2 Unit tests in `editors/vscode/test/parser.test.mjs` with a fake `exists`: empty, whitespace, crate folder,
  repository root, `~/src/corvis` with a fake home, relative path, folder without `Cargo.toml`, Windows-style
  absolute path on Windows. Verify: `npm test` in `editors/vscode`

## 2. Extension wiring

- [x] 2.1 `editors/vscode/package.json`: add `corvis.new.corvisPath` (string, default `""`, scope
  `machine-overridable`) with a `markdownDescription` that says it is passed as `CORVIS_PATH` to `corvis new`, that it
  takes the `crates/corvis` folder (or the repository root), and links `#corvis.frameworkPath#`; add a sentence to
  the `corvis.frameworkPath` description pointing back. Verify: the setting appears in the Settings UI of an
  Extension Development Host
- [x] 2.2 `editors/vscode/src/cli.ts`: add the optional `extraEnv` parameter to `runCli` (decision 5), log
  `(CORVIS_PATH: ...)` when it carries that key. Verify: `npm run compile`; existing callers compile unchanged
- [x] 2.3 `editors/vscode/src/generate.ts` `newProject`: read the setting, call `resolveCorvisPath` before the folder
  dialog, show the error with **Open Settings** (`workbench.action.openSettings`, `corvis.new.corvisPath`) and return
  on `{ error }`, pass `{ CORVIS_PATH: path }` to `runCli` on `{ path }`. Verify manually in an Extension Development
  Host: (a) setting empty -> generated `Cargo.toml` has `version = "0.1.0"`; (b) setting = `<clone>/crates/corvis`
  -> `path = "<clone>/crates/corvis"` and the output shows `NOTE using CORVIS_PATH=...`; (c) setting = `<clone>` ->
  same as (b); (d) setting = `relative/x` -> error, no dialog, no folder created; (e) a Generate command with the
  setting filled has no `CORVIS_PATH` line in the output channel

## 3. Docs

- [x] 3.1 `editors/vscode/README.md` (Spanish, like the rest of the file): add the row for `corvis.new.corvisPath`
  to the settings table (lines 130-141), and a sentence after the CLI lookup list (lines 64-72) saying that **New
  Project** passes the setting as `CORVIS_PATH` and that, until the first release, it must be filled for the project
  to build. Add an "Unreleased" entry to `editors/vscode/CHANGELOG.md`. Verify: read both rendered in VS Code's
  Markdown preview
- [x] 3.2 `docs/cli/overview.md`, section "VS Code extension" (lines 92-94): one paragraph on
  `corvis.new.corvisPath` and how it relates to `CORVIS_PATH` and the "Until the first release" warning above it.
  Verify: the paragraph matches the setting description of 2.1
- [x] 3.3 Same paragraph in `docs/es/cli/overview.md` ("Extensión para VS Code"). Verify: `npm run docs:check` in
  `docs/`

## 4. Definition of done

- [x] 4.1 No warnings: `npm run compile` in `editors/vscode` is clean; `cargo build --workspace --all-targets` with no
  warnings (no Rust code changes expected)
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes (no Rust code changed; clippy over the workspace passed)
- [x] 4.4 Regression tests: the `resolveCorvisPath` unit tests of 1.2 pass with `npm test` in `editors/vscode`
- [x] 4.5 Docs updated in English and Spanish (`docs/` and `docs/es/`) plus the extension README and CHANGELOG;
  `npm run docs:check` in `docs/` passes
- [x] 4.6 corvis-parity: not required, no framework runtime code changes (extension only); state this in the commit
  message

## Workflow follow-up

- [x] Archive the change with `/opsx:archive`
- [x] Close issue [#149](https://github.com/randygil/corvis/issues/149)
- [x] Tick #149 in its epic [#1](https://github.com/randygil/corvis/issues/1)
- [x] Move the card to Done in the Corvis Roadmap project
