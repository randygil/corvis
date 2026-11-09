# Design

## Context

What the CLI does (after #121, verified in `crates/corvis-cli/src/main.rs`):

- `cmd_new` (lines 215-249) reads `std::env::var("CORVIS_PATH")` (line 219) and calls `corvis_dependency` (lines
  168-189). Precedence: `--corvis-path` and `--corvis-version` together is an error, then `--corvis-version`, then
  `--corvis-path`, then a non-empty `CORVIS_PATH`, then `version = "<CLI version>"`.
- The path is normalized by `normalize_path` (lines 154-156: strips `\\?\`, turns `\` into `/`) and rejected when it
  contains `"` or a control character (lines 184-187). The CLI does not check that the folder exists or has a
  `Cargo.toml`; a wrong path only shows up at `cargo build`.
- When the path came from `CORVIS_PATH`, `dependency_notes` (lines 202-213) makes `cmd_new` print
  `NOTE using CORVIS_PATH=<dir>`. When the default version was used and the checkout the CLI was compiled from exists,
  it prints `NOTE corvis = "<v>" comes from crates.io; to use the checkout this CLI was built from, pass --corvis-path
  <dir> or set CORVIS_PATH=<dir>`.
- `CORVIS_PATH` names the `crates/corvis` crate folder, not the repository root (`docs/cli/overview.md:77-90`).

What the extension does today (`editors/vscode/`):

- `newProject` (`src/generate.ts:138-171`) asks for a parent folder and a name, resolves the CLI with
  `requireCli()` (line 154) and runs `runCli(cli, ['new', name.trim()], cwd, token)` (line 159). It finds the
  generated folder from the `CREATE ... Cargo.toml` line (`parseChanges`, line 166).
- `runCli` (`src/cli.ts:123-160`) spawns the CLI with `env: { ...process.env, NO_COLOR: '1', CARGO_TERM_COLOR:
  'never' }` (line 136). It has no way to add variables per call.
- `resolveCli` (`src/cli.ts:78-99`) can run the CLI as `cargo run -q --manifest-path <repo>/crates/corvis-cli/Cargo.toml
  --` when it finds a framework checkout through `findFrameworkRoot` (`src/project.ts:83-119`: the
  `corvis.frameworkPath` setting, a workspace folder, or the `corvis = { path = ... }` dependency of the project).
  `isFrameworkRoot` (`src/project.ts:74-76`) checks `crates/corvis-cli/Cargo.toml`. `expandHome`
  (`src/project.ts:121-124`) expands a leading `~`.
- Settings (`package.json:396-442`): `corvis.cli.path` and `corvis.frameworkPath` (both `machine-overridable`, the
  latter the repository root), `corvis.generate.*`, `corvis.decorations.enabled`, `corvis.routeHints.enabled`,
  `corvis.explorer.exclude`.
- Unit tests: `npm test` compiles and runs `node --test test/parser.test.mjs`, which loads `out/parser.js` and
  `out/paths.js`. `src/paths.ts` holds helpers "without a `vscode` dependency (unit-tested in test/)".
- The extension README (`README.md:64-72` CLI lookup, `:130-141` settings table) and `CHANGELOG.md` are written in
  Spanish. The docs site mentions the extension in `docs/cli/overview.md:92-94` and `docs/devtools/overview.md:13`
  (and the Spanish pages).

## Goals / Non-Goals

**Goals:**

- A project created with **Corvis: New Project** can point at a local framework checkout, chosen once in the
  settings.
- With the setting empty, the extension behaves exactly as today.
- A bad value fails in the extension, with a message that names the setting, instead of producing a project that does
  not build.

**Non-Goals:**

- Changing the CLI or the `cli/new` rules.
- Guessing the checkout when the setting is empty.
- Passing `CORVIS_PATH` to commands other than `corvis new`.

## Decisions

1. **Setting name `corvis.new.corvisPath`.** It groups with the existing `corvis.generate.*` settings (options of a
   CLI command) and mirrors `--corvis-path` / `CORVIS_PATH`, so the docs can say "the setting is `CORVIS_PATH` for
   the New Project command".
   - Alternative: `corvis.localPath` (the issue's example). Rejected: next to `corvis.frameworkPath` it does not say
     what it is local to, or which command uses it.
   - Alternative: reuse `corvis.frameworkPath`. Rejected: that setting chooses how to *run* the CLI (with `cargo run`)
     and is empty when the CLI is installed; tying the generated dependency to it would change what existing users
     get without them asking, and the issue asks for an empty default.

2. **Value: the crate folder, with the repository root accepted.** The value is what `CORVIS_PATH` takes (the
   `crates/corvis` folder). If the expanded value is a framework root (`isFrameworkRoot(value)` is true), the
   extension uses `path.join(value, 'crates', 'corvis')`. Users who already filled `corvis.frameworkPath` with the
   root through the "Select Corvis repository…" dialog (`src/cli.ts:112-116`) will try the same folder here; accepting
   it avoids a confusing error.
   - Alternative: accept only the crate folder. Rejected for the reason above; the cost is three lines and a unit test.

3. **Validation in the extension.** After trimming and `expandHome`, the value must satisfy `path.isAbsolute`, and
   `<resolved>/Cargo.toml` must exist. Otherwise `newProject` shows `Corvis: corvis.new.corvisPath must be an absolute
   path (got "<value>")` or `Corvis: corvis.new.corvisPath points at <folder>, which has no Cargo.toml`, with an
   **Open Settings** button that opens `corvis.new.corvisPath`, and returns before asking for the parent folder.
   - Why before the dialogs: the user should not pick a folder and type a name only to be told the setting is wrong.
   - Alternative: let the CLI write whatever was configured. Rejected: the CLI does not check the folder, so the
     failure would appear at `cargo build` in the new project.
   - Alternative: resolve a relative value against the first workspace folder. Rejected: the setting is
     `machine-overridable` and usually global; a relative value would change meaning with the open workspace.

4. **Pure helper in `src/paths.ts`.** Add
   `resolveCorvisPath(value: string | undefined, home: string, exists: (p: string) => boolean):
   { path?: string; error?: string }`. It returns `{}` for an empty value, `{ path }` for a valid one (root turned into
   `crates/corvis`), `{ error }` otherwise. `home` and `exists` are parameters so the unit tests need no file system
   or `vscode`. `src/project.ts` keeps `expandHome` and `isFrameworkRoot`; the helper takes the same rules (the `~`
   expansion and the `crates/corvis-cli/Cargo.toml` check) as small local functions or imports them from a module
   without a `vscode` import, so `test/parser.test.mjs` can load it.

5. **`runCli` takes optional extra environment variables.** Signature becomes
   `runCli(cli, args, cwd, token?, extraEnv?: Record<string, string>)`, spreading `extraEnv` after `process.env` and
   before `NO_COLOR` / `CARGO_TERM_COLOR`. Only `newProject` passes `{ CORVIS_PATH: resolved }`. The output channel
   line `> corvis new my-app` gets a second line `  (CORVIS_PATH: <resolved>)` next to the existing `(cwd: ...)`, so
   the log shows why the manifest has a path.
   - Alternative: pass `--corvis-path <dir>` instead of the variable. Equivalent for the CLI, but the issue and the
     docs speak of `CORVIS_PATH`, and the variable keeps the CLI's `NOTE using CORVIS_PATH=...` line, which tells the
     user where the path came from. Either works; the variable is chosen to match the issue.

6. **Empty setting: no change to the environment.** The extension neither sets nor deletes `CORVIS_PATH`. A
   variable inherited from the shell that started VS Code keeps working, as documented in `docs/cli/overview.md`.

## Risks / Trade-offs

- [The setting is global and a checkout moves] -> the `Cargo.toml` check catches a deleted folder; a folder that
  moved but still exists elsewhere is the user's choice.
- [Users confuse `corvis.frameworkPath` and `corvis.new.corvisPath`] -> decision 2 accepts the root in both; the
  setting descriptions cross-reference each other.
- [After the first release the setting is rarely needed] -> it stays useful for framework contributors and for testing
  unreleased framework changes; the description says it is optional.

## Open questions

1. Setting name: `corvis.new.corvisPath` (recommended) or the issue's `corvis.localPath`?
2. When the setting is empty and the CLI runs through `cargo run` from a checkout (`source === 'cargo-run'`), should
   the extension default `CORVIS_PATH` to `<checkout>/crates/corvis`? Recommendation: not in this change (the issue
   asks for an empty default, and #121 rejected implicit checkout detection); revisit if contributors ask.
3. When the CLI prints the "comes from crates.io" `NOTE`, should the success message offer **Set Corvis path…**
   (opens the setting)? Recommendation: a later issue, after the first release changes what that note means.
4. The extension README and CHANGELOG are in Spanish only. This change edits them in Spanish to keep them consistent;
   confirm, or file the translation separately (listed in Out of scope).

## Review notes (2026-11-08)

1. The setting is `corvis.new.corvisPath`.
2. The path is not derived automatically from a CLI run with `cargo run`.
3. A "Set Corvis path…" button on the success message is not part of this change.
4. Public repository text is in English: the new README and CHANGELOG entries are written in English, and the extension README and CHANGELOG are translated to English in this change.

## Implementation notes

- `resolveCorvisPath` keeps its own `~` expansion and root check (`crates/corvis-cli/Cargo.toml`) instead of importing `expandHome` and `isFrameworkRoot`, because `project.ts` imports `vscode`; `paths.ts` stays testable.
- Manual checks of tasks 2.1 and 2.3 in an Extension Development Host were not run (unattended); the helper is covered by unit tests and the TypeScript build is clean.
- `cargo test --workspace` (task 4.3) was not run: the change has no Rust code. `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass.
- The README and CHANGELOG were translated to English per review note 4.

## Verification (2026-11-08)

- `npm test` in `editors/vscode`: 7/7 pass (the new `resolveCorvisPath` test covers empty, whitespace, crate folder, repository root, `~`, relative path, folder without `Cargo.toml` and a Windows-style path); `tsc` compiles clean; naming and docs checks pass.
- Not run: the manual checks in an Extension Development Host listed in tasks 2.1 and 2.3 (Settings UI entry, and the generated `Cargo.toml` for an empty setting, a crate folder, a repository root, and an invalid path). The session was unattended; the resolution logic they exercise is covered by the unit test.
