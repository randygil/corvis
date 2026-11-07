import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';

import { outputChannel, requireCli, runCli, runCliTask } from './cli';
import { featureDirFor, parseChanges } from './paths';
import { CargoProject, findCargoProject, pickProject } from './project';

export interface Schematic {
  name: string;
  alias: string;
  label: string;
  detail: string;
  /** Suffix of the main generated file (`users_controller.rs`). */
  fileSuffix: string;
  /** The template includes a `#[cfg(test)]` module (`--no-spec` removes it). */
  hasSpec: boolean;
  /** Accepts `--flat`. */
  flat: boolean;
}

/** The schematics of `corvis generate` (crates/corvis-cli/src/main.rs). */
export const SCHEMATICS: Schematic[] = [
  { name: 'resource', alias: 'res', label: 'Resource', detail: 'CRUD: module, controller, service, DTOs, entity and test', fileSuffix: 'controller', hasSpec: false, flat: false },
  { name: 'module', alias: 'mo', label: 'Module', detail: '#[module] struct, registered in AppModule imports', fileSuffix: 'module', hasSpec: false, flat: true },
  { name: 'controller', alias: 'co', label: 'Controller', detail: '#[controller] + #[routes], registered in the feature module', fileSuffix: 'controller', hasSpec: true, flat: true },
  { name: 'service', alias: 's', label: 'Service', detail: '#[injectable] provider, registered in the feature module', fileSuffix: 'service', hasSpec: true, flat: true },
  { name: 'guard', alias: 'gu', label: 'Guard', detail: 'impl CanActivate', fileSuffix: 'guard', hasSpec: false, flat: true },
  { name: 'interceptor', alias: 'itc', label: 'Interceptor', detail: 'impl CorvisInterceptor', fileSuffix: 'interceptor', hasSpec: false, flat: true },
  { name: 'pipe', alias: 'pi', label: 'Pipe', detail: 'impl PipeTransform', fileSuffix: 'pipe', hasSpec: false, flat: true },
  { name: 'filter', alias: 'f', label: 'Exception filter', detail: 'impl ExceptionFilter', fileSuffix: 'filter', hasSpec: false, flat: true },
  { name: 'middleware', alias: 'mi', label: 'Middleware', detail: 'impl CorvisMiddleware', fileSuffix: 'middleware', hasSpec: false, flat: true },
  { name: 'gateway', alias: 'ga', label: 'WebSocket gateway', detail: '#[websocket_gateway] + #[subscribe_message]', fileSuffix: 'gateway', hasSpec: false, flat: true },
];

const NAME_RE = /^[A-Za-z][A-Za-z0-9_\- ]*$/;

function folderOf(uri?: vscode.Uri): string | undefined {
  if (!uri || uri.scheme !== 'file') return undefined;
  try {
    return fs.statSync(uri.fsPath).isDirectory() ? uri.fsPath : path.dirname(uri.fsPath);
  } catch {
    return undefined;
  }
}

export async function generate(schematicName?: string, uri?: vscode.Uri): Promise<void> {
  const folder = folderOf(uri);
  const project = folder ? findCargoProject(folder) : await pickProject();
  if (!project) {
    if (folder) vscode.window.showErrorMessage('Corvis: the selected folder is not inside a Cargo project (Cargo.toml + src/).');
    return;
  }

  let schematic = SCHEMATICS.find((s) => s.name === schematicName);
  if (!schematic) {
    const pick = await vscode.window.showQuickPick(
      SCHEMATICS.map((s) => ({ label: s.label, description: `corvis g ${s.name} (${s.alias})`, detail: s.detail, schematic: s })),
      { placeHolder: `Generate in ${project.name}`, matchOnDescription: true, matchOnDetail: true },
    );
    if (!pick) return;
    schematic = pick.schematic;
  }

  let featureDir = featureDirFor(project, folder);
  if (featureDir === null) {
    const rel = path.relative(project.root, folder!).replace(/\\/g, '/');
    const go = await vscode.window.showWarningMessage(
      `corvis g only places files in folders directly under src/ ("${rel}" is nested). Generate in src/<name>/ instead?`,
      { modal: true },
      'Generate',
    );
    if (go !== 'Generate') return;
    featureDir = undefined;
  }

  const name = await vscode.window.showInputBox({
    title: `Corvis: generate ${schematic.label.toLowerCase()}`,
    prompt: featureDir ? `Name (files go to src/${featureDir}/)` : 'Name (e.g. cats, user-profile, UserProfile)',
    value: featureDir ?? '',
    validateInput: (v) => (NAME_RE.test(v.trim()) ? undefined : 'Use letters, digits, "-" or "_", starting with a letter'),
  });
  if (!name) return;

  const flags: string[] = [];
  const config = vscode.workspace.getConfiguration('corvis');
  type Opt = vscode.QuickPickItem & { flag: string };
  const options: Opt[] = [];
  if (schematic.flat && featureDir === undefined) {
    options.push({ label: '--flat', description: `src/${'<name>'}_${schematic.fileSuffix}.rs, without a feature folder`, flag: '--flat', picked: config.get<boolean>('generate.flat') ?? false });
  }
  if (schematic.hasSpec) {
    options.push({ label: '--no-spec', description: 'Leave out the #[cfg(test)] module', flag: '--no-spec', picked: !(config.get<boolean>('generate.spec') ?? true) });
  }
  if (options.length) {
    const picked = await vscode.window.showQuickPick(options, { canPickMany: true, title: 'Options (Enter to continue)', placeHolder: 'Select flags for corvis generate' });
    if (!picked) return;
    flags.push(...picked.map((p) => p.flag));
  }

  const cli = await requireCli(project);
  if (!cli) return;

  const args = ['generate', schematic.name, name.trim()];
  if (featureDir) args.push('--path', featureDir);
  args.push(...flags);

  const result = await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: `corvis g ${schematic.name} ${name.trim()}`, cancellable: true },
    (_p, token) => runCli(cli, args, project.root, token),
  );
  reportGeneration(result.code, result.output, schematic, project);
}

function reportGeneration(code: number | null, output: string, schematic: Schematic, project: CargoProject) {
  if (code !== 0) {
    const err = /Error:\s*(.*)/.exec(output)?.[1] ?? 'see the Corvis output for details';
    vscode.window.showErrorMessage(`Corvis: ${err}`, 'Show Output').then((c) => c && outputChannel().show(true));
    return;
  }
  const changes = parseChanges(output, project.root);
  const created = changes.filter((c) => c.kind === 'CREATE');
  const updated = changes.filter((c) => c.kind === 'UPDATE');
  const skipped = changes.filter((c) => c.kind === 'SKIP');
  const main = created.find((c) => c.file.endsWith(`_${schematic.fileSuffix}.rs`)) ?? created.find((c) => c.file.endsWith('.rs') && !c.file.endsWith('mod.rs'));
  if (main && vscode.workspace.getConfiguration('corvis').get<boolean>('generate.openGeneratedFile', true)) {
    vscode.window.showTextDocument(vscode.Uri.file(main.file), { preview: false });
  }
  let msg = `Corvis: created ${created.length} file${created.length === 1 ? '' : 's'}`;
  if (updated.length) msg += `, updated ${updated.map((u) => path.basename(u.file)).join(', ')}`;
  if (skipped.length) {
    vscode.window.showWarningMessage(`${msg}. Could not update ${skipped.map((s) => path.basename(s.file)).join(', ')} automatically; register it by hand.`, 'Show Output').then((c) => c && outputChannel().show(true));
  } else {
    vscode.window.showInformationMessage(msg);
  }
}

export async function newProject(): Promise<void> {
  const defaultParent = vscode.workspace.workspaceFolders?.[0]?.uri;
  const parent = await vscode.window.showOpenDialog({
    canSelectFolders: true,
    canSelectFiles: false,
    defaultUri: defaultParent,
    openLabel: 'Create the project here',
    title: 'Corvis: parent folder of the new project',
  });
  if (!parent?.[0]) return;
  const name = await vscode.window.showInputBox({
    title: 'Corvis: new project',
    prompt: 'Project name (the folder is created in kebab-case)',
    validateInput: (v) => (NAME_RE.test(v.trim()) ? undefined : 'Use letters, digits, "-" or "_", starting with a letter'),
  });
  if (!name) return;
  const cli = await requireCli();
  if (!cli) return;
  const cwd = parent[0].fsPath;
  const result = await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: `corvis new ${name.trim()}`, cancellable: true },
    (_p, token) => runCli(cli, ['new', name.trim()], cwd, token),
  );
  if (result.code !== 0) {
    const err = /Error:\s*(.*)/.exec(result.output)?.[1] ?? 'see the Corvis output for details';
    vscode.window.showErrorMessage(`Corvis: ${err}`, 'Show Output').then((c) => c && outputChannel().show(true));
    return;
  }
  const created = parseChanges(result.output, cwd).find((c) => c.file.endsWith('Cargo.toml'));
  const dir = created ? path.dirname(created.file) : path.join(cwd, name.trim());
  const choice = await vscode.window.showInformationMessage(`Corvis: created ${path.basename(dir)}`, 'Open Folder', 'Add to Workspace');
  if (choice === 'Open Folder') await vscode.commands.executeCommand('vscode.openFolder', vscode.Uri.file(dir), { forceNewWindow: false });
  else if (choice === 'Add to Workspace') vscode.workspace.updateWorkspaceFolders(vscode.workspace.workspaceFolders?.length ?? 0, 0, { uri: vscode.Uri.file(dir) });
}

export async function runProjectTask(kind: 'start' | 'watch' | 'build', uri?: vscode.Uri): Promise<void> {
  const project = await pickProject(uri);
  if (!project) return;
  const cli = await requireCli(project);
  if (!cli) return;
  const args = kind === 'start' ? ['start'] : kind === 'watch' ? ['start', '--watch'] : ['build', '--release'];
  await runCliTask(cli, args, project.root, `${project.name}: corvis ${args.join(' ')}`);
}

export async function showInfo(uri?: vscode.Uri): Promise<void> {
  const project = await pickProject(uri);
  const cli = await requireCli(project);
  if (!cli) return;
  const out = outputChannel();
  out.show(true);
  await runCli(cli, ['info'], project?.root ?? process.cwd());
  out.appendLine(`CLI: ${cli.label} (${cli.source})`);
}
