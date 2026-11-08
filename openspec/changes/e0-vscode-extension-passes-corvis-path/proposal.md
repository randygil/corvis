# Proposal

Implements [randygil/corvis#149](https://github.com/randygil/corvis/issues/149) (epic
[#1](https://github.com/randygil/corvis/issues/1)). Follow-up of #121 (archived change
`2026-11-08-e0-corvis-new-from-a-crates-io-install-points-at-a-local-path`).

## Why

Since #121, `corvis new` writes `corvis = { version = "<CLI version>" }` unless `--corvis-path` or a non-empty
`CORVIS_PATH` asks for a local checkout (`crates/corvis-cli/src/main.rs:168-189`). The VS Code extension's
**Corvis: New Project** runs `corvis new <name>` with neither (`editors/vscode/src/generate.ts:159`). Until the
framework is on crates.io, a project created from the extension cannot resolve `corvis` and does not build. The only
workaround is to launch VS Code from a shell that already exports `CORVIS_PATH` (the extension passes
`process.env` through, `editors/vscode/src/cli.ts:136`), which nobody can discover from the extension.

## What Changes

- New extension setting `corvis.new.corvisPath` (string, empty by default, scope `machine-overridable`). When it is
  not empty, **Corvis: New Project** runs `corvis new <name>` with `CORVIS_PATH` set to that folder in the child
  process environment. When it is empty, the command behaves as today (an inherited `CORVIS_PATH` still passes
  through).
- The value means the same as `CORVIS_PATH`: the `crates/corvis` folder of a framework checkout. As a convenience, a
  value that points at the repository root (the folder `corvis.frameworkPath` holds) is turned into
  `<root>/crates/corvis`. `~` is expanded like the other path settings.
- Before running the CLI, the extension checks the value: it must be absolute and contain a `Cargo.toml`. Otherwise
  it shows an error that names the setting, offers **Open Settings**, and does not run the CLI.
- Only `corvis new` gets the variable. `generate`, `start`, `build` and `info` are unchanged.
- The path logic lives in a helper without a `vscode` dependency, unit-tested in `editors/vscode/test/`.
- Docs: the extension README (settings table and the **New Project** row), the extension CHANGELOG, and the "VS Code
  extension" section of `docs/cli/overview.md` and `docs/es/cli/overview.md`.

## Capabilities

### New Capabilities

- `cli/vscode-extension`: how the VS Code extension runs `corvis new` and which framework dependency the generated
  project gets.

### Modified Capabilities

None. `cli/new` is unchanged: the extension uses the existing `CORVIS_PATH` contract.

## Impact

- Code: `editors/vscode/package.json` (configuration), `editors/vscode/src/generate.ts` (`newProject`),
  `editors/vscode/src/cli.ts` (`runCli` accepts extra environment variables), `editors/vscode/src/paths.ts` (new
  helper), `editors/vscode/test/parser.test.mjs` (unit tests).
- No Rust code changes. No framework runtime change, so the corvis-parity check does not apply.
- Docs: `editors/vscode/README.md` and `editors/vscode/CHANGELOG.md` (both written in Spanish today),
  `docs/cli/overview.md`, `docs/es/cli/overview.md`.
- Users who set the variable in their shell keep working; the setting wins over the inherited variable when set.

## Out of scope

- Deriving the path automatically from `corvis.frameworkPath` or from the checkout used to run `cargo run -p
  corvis-cli` when the setting is empty (design.md, open question 2).
- Offering to fill the setting when the CLI prints its "comes from crates.io" `NOTE` line (open question 3).
- A `--corvis-version` setting for the extension (no request for it; the CLI default already covers releases).
- `editors/vscode/scripts/smoke.mjs` generates its app without `CORVIS_PATH`, so the smoke app has a version
  requirement; the smoke test does not build it, so it keeps working (possible follow-up if it ever builds the app).
- Translating the extension README into English (it is Spanish only today; separate docs task).
