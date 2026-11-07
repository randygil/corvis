// Launches the installed VS Code with this extension in development mode against a fresh
// Corvis app and runs test/smoke/index.js inside the extension host.
//
//   node scripts/smoke.mjs <work-dir>
//
// <work-dir> receives the generated app, an isolated user-data dir and the log.
// Needs `corvis` (or the Corvis repository for `cargo run -p corvis-cli`).
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const ext = resolve(here, '..');
const repo = resolve(ext, '../..');
const work = resolve(process.argv[2] ?? join(ext, '.smoke'));
const app = join(work, 'smoke-app');
const log = join(work, 'smoke.log');

mkdirSync(work, { recursive: true });
rmSync(app, { recursive: true, force: true });
rmSync(log, { force: true });

const cli = ['run', '-q', '--manifest-path', join(repo, 'crates/corvis-cli/Cargo.toml'), '--'];
execFileSync('cargo', [...cli, 'new', 'smoke-app', '--skip-git'], { cwd: work, stdio: 'inherit' });
execFileSync('cargo', [...cli, 'generate', 'resource', 'cats'], { cwd: app, stdio: 'inherit' });

const code =
  process.env.VSCODE_EXE ??
  [join(process.env.LOCALAPPDATA ?? '', 'Programs/Microsoft VS Code/Code.exe'), '/usr/share/code/code', '/Applications/Visual Studio Code.app/Contents/MacOS/Electron'].find(existsSync);
if (!code) throw new Error('VS Code executable not found; set VSCODE_EXE');

const r = spawnSync(
  code,
  [
    app,
    `--extensionDevelopmentPath=${ext}`,
    `--extensionTestsPath=${join(ext, 'test/smoke/index.js')}`,
    `--user-data-dir=${join(work, 'user-data')}`,
    `--extensions-dir=${join(work, 'extensions')}`,
    '--disable-workspace-trust',
    '--skip-welcome',
    '--skip-release-notes',
    '--disable-telemetry',
    '--new-window',
  ],
  { stdio: 'inherit', env: { ...process.env, CORVIS_SMOKE_LOG: log, CORVIS_SMOKE_DIR: work }, timeout: 15 * 60 * 1000 },
);
console.log(existsSync(log) ? readFileSync(log, 'utf8') : '(no log written)');
console.log(`VS Code exited with ${r.status}${r.error ? ' ' + r.error.message : ''}`);
process.exit(r.status ?? 1);
