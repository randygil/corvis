import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';

/** A Cargo package: the folder with `Cargo.toml` and `src/` (what `corvis g` looks for). */
export interface CargoProject {
  root: string;
  src: string;
  name: string;
}

function exists(p: string): boolean {
  try {
    fs.accessSync(p);
    return true;
  } catch {
    return false;
  }
}

function isDir(p: string): boolean {
  try {
    return fs.statSync(p).isDirectory();
  } catch {
    return false;
  }
}

function packageName(cargoToml: string): string | undefined {
  try {
    const text = fs.readFileSync(cargoToml, 'utf8');
    const pkg = /\[package\][^[]*?\bname\s*=\s*"([^"]+)"/.exec(text);
    return pkg?.[1];
  } catch {
    return undefined;
  }
}

/** Same lookup as the CLI: walk up until a folder has both `Cargo.toml` and `src/`. */
export function findCargoProject(start: string): CargoProject | undefined {
  let dir = isDir(start) ? start : path.dirname(start);
  for (;;) {
    const toml = path.join(dir, 'Cargo.toml');
    if (exists(toml) && isDir(path.join(dir, 'src'))) {
      return { root: dir, src: path.join(dir, 'src'), name: packageName(toml) ?? path.basename(dir) };
    }
    const parent = path.dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
}

/** `true` when the manifest depends on corvis (or is part of the corvis repository). */
export function dependsOnCorvis(cargoTomlText: string): boolean {
  return /^\s*corvis(?:-orm|-swagger)?\s*=/m.test(cargoTomlText) || /^\s*name\s*=\s*"corvis(?:-[a-z]+)?"/m.test(cargoTomlText);
}

export async function findCorvisManifests(token?: vscode.CancellationToken): Promise<vscode.Uri[]> {
  const exclude = vscode.workspace.getConfiguration('corvis').get<string>('explorer.exclude') || '**/{target,node_modules,.git}/**';
  const manifests = await vscode.workspace.findFiles('**/Cargo.toml', exclude, 200, token);
  const out: vscode.Uri[] = [];
  for (const m of manifests) {
    try {
      const text = Buffer.from(await vscode.workspace.fs.readFile(m)).toString('utf8');
      if (dependsOnCorvis(text)) out.push(m);
    } catch {
      // unreadable manifest: ignore
    }
  }
  return out;
}

/** Is `dir` the root of the Corvis repository (has `crates/corvis-cli`)? */
export function isFrameworkRoot(dir: string): boolean {
  return exists(path.join(dir, 'crates', 'corvis-cli', 'Cargo.toml'));
}

/**
 * Locates the Corvis repository, used to run the CLI through `cargo run`:
 * the `corvis.frameworkPath` setting, a workspace folder that is the repository,
 * or the `corvis = { path = "..." }` dependency of the project.
 */
export function findFrameworkRoot(project?: CargoProject): string | undefined {
  const configured = vscode.workspace.getConfiguration('corvis').get<string>('frameworkPath')?.trim();
  if (configured) {
    const resolved = expandHome(configured);
    if (isFrameworkRoot(resolved)) return resolved;
  }
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    let dir = folder.uri.fsPath;
    for (let i = 0; i < 4; i++) {
      if (isFrameworkRoot(dir)) return dir;
      const parent = path.dirname(dir);
      if (parent === dir) break;
      dir = parent;
    }
  }
  if (project) {
    try {
      const text = fs.readFileSync(path.join(project.root, 'Cargo.toml'), 'utf8');
      const dep = /^\s*corvis\s*=\s*\{[^}]*\bpath\s*=\s*"([^"]+)"/m.exec(text);
      if (dep) {
        const crateDir = path.resolve(project.root, dep[1]);
        const repo = path.resolve(crateDir, '..', '..');
        if (isFrameworkRoot(repo)) return repo;
      }
    } catch {
      // no manifest
    }
    let dir = project.root;
    for (let i = 0; i < 4; i++) {
      if (isFrameworkRoot(dir)) return dir;
      const parent = path.dirname(dir);
      if (parent === dir) break;
      dir = parent;
    }
  }
  return undefined;
}

export function expandHome(p: string): string {
  if (p.startsWith('~')) return path.join(process.env.USERPROFILE || process.env.HOME || '', p.slice(1));
  return p;
}

/** The folder the user is working on: explorer selection, active editor, or the only workspace folder. */
export function contextFolder(uri?: vscode.Uri): string | undefined {
  if (uri && uri.scheme === 'file') return uri.fsPath;
  const active = vscode.window.activeTextEditor?.document.uri;
  if (active && active.scheme === 'file') return path.dirname(active.fsPath);
  const folders = vscode.workspace.workspaceFolders;
  if (folders && folders.length === 1) return folders[0].uri.fsPath;
  return undefined;
}

/** Resolves the Cargo project for a command; asks when it cannot be inferred. */
export async function pickProject(uri?: vscode.Uri): Promise<CargoProject | undefined> {
  const folder = contextFolder(uri);
  if (folder) {
    const p = findCargoProject(folder);
    if (p) return p;
  }
  const manifests = await findCorvisManifests();
  // Applications only: a package with `src/main.rs` (libraries such as the corvis crates are skipped).
  const unique = manifests
    .map((m) => path.dirname(m.fsPath))
    .filter((dir) => exists(path.join(dir, 'src', 'main.rs')))
    .map((dir) => ({ root: dir, src: path.join(dir, 'src'), name: packageName(path.join(dir, 'Cargo.toml')) ?? path.basename(dir) }));
  if (unique.length === 1) return unique[0];
  if (unique.length === 0) {
    vscode.window.showErrorMessage('Corvis: no Cargo project (Cargo.toml + src/) found. Open a Corvis application or create one with "Corvis: New Project…".');
    return undefined;
  }
  const pick = await vscode.window.showQuickPick(
    unique.map((p) => ({ label: p.name, description: vscode.workspace.asRelativePath(p.root), project: p })),
    { placeHolder: 'Select the Corvis application' },
  );
  return pick?.project;
}
