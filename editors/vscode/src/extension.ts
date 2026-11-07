import * as vscode from 'vscode';

import { outputChannel, resetCliCache } from './cli';
import { CorvisDecorations } from './decorations';
import { SCHEMATICS, generate, newProject, runProjectTask, showInfo } from './generate';
import { WorkspaceIndex, toPosition } from './index';
import { findCorvisManifests } from './project';
import { ModulesTree, Node, RoutesTree, methodIcon } from './tree';

/** Returned by `activate` (used by the smoke test in test/smoke). */
export interface CorvisApi {
  index: WorkspaceIndex;
  modules: ModulesTree;
  routes: RoutesTree;
  ready: Promise<void>;
}

export function activate(context: vscode.ExtensionContext): CorvisApi {
  const index = new WorkspaceIndex();
  context.subscriptions.push(index);

  const modules = new ModulesTree(index);
  const routes = new RoutesTree(index);
  context.subscriptions.push(
    vscode.window.createTreeView('corvis.modules', { treeDataProvider: modules, showCollapseAll: true }),
    vscode.window.createTreeView('corvis.routes', { treeDataProvider: routes, showCollapseAll: true }),
  );

  const detect = async () => {
    const found = (await findCorvisManifests()).length > 0;
    await vscode.commands.executeCommand('setContext', 'corvis.isCorvisProject', found);
    if (found) index.enable();
  };
  const ready = detect();
  const manifests = vscode.workspace.createFileSystemWatcher('**/Cargo.toml');
  manifests.onDidCreate(() => void detect());
  manifests.onDidChange(() => void detect());
  context.subscriptions.push(manifests, vscode.workspace.onDidChangeWorkspaceFolders(() => void detect()));

  context.subscriptions.push(new CorvisDecorations(index));

  const reg = (id: string, fn: (...args: any[]) => unknown) => context.subscriptions.push(vscode.commands.registerCommand(id, fn));

  reg('corvis.generate', (uri?: vscode.Uri) => generate(undefined, uri));
  for (const s of SCHEMATICS) reg(`corvis.generate.${s.name}`, (uri?: vscode.Uri) => generate(s.name, uri));
  reg('corvis.new', () => newProject());
  reg('corvis.start', (uri?: vscode.Uri) => runProjectTask('start', uri instanceof vscode.Uri ? uri : undefined));
  reg('corvis.startWatch', (uri?: vscode.Uri) => runProjectTask('watch', uri instanceof vscode.Uri ? uri : undefined));
  reg('corvis.build', (uri?: vscode.Uri) => runProjectTask('build', uri instanceof vscode.Uri ? uri : undefined));
  reg('corvis.info', () => showInfo());
  reg('corvis.showOutput', () => outputChannel().show());
  reg('corvis.refresh', async () => {
    if (!index.isEnabled) await detect();
    await index.rebuild();
  });
  reg('corvis.revealLocation', async (uri: vscode.Uri, pos: vscode.Position) => {
    const editor = await vscode.window.showTextDocument(uri, { preview: true });
    const range = new vscode.Range(pos, pos);
    editor.selection = new vscode.Selection(pos, pos);
    editor.revealRange(range, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
  });
  reg('corvis.copyRoutePath', async (node?: Node) => {
    if (node?.kind !== 'route') return;
    await vscode.env.clipboard.writeText(node.route.fullPath);
    vscode.window.setStatusBarMessage(`Copied ${node.route.fullPath}`, 2500);
  });
  reg('corvis.goToRoute', async () => {
    if (!index.isEnabled) index.enable();
    await index.ready();
    const crates = index.getCrates();
    const multi = crates.length > 1;
    type Item = vscode.QuickPickItem & { go?: () => void };
    const items: Item[] = [];
    for (const c of crates) {
      if (!c.routes.length) continue;
      if (multi) items.push({ label: c.name, kind: vscode.QuickPickItemKind.Separator });
      for (const r of c.routes) {
        const icon = methodIcon(r.route.method);
        items.push({
          label: `${r.route.method} ${r.fullPath}`,
          iconPath: icon,
          description: `${r.owner}::${r.route.handler}`,
          detail: vscode.workspace.asRelativePath(r.file.uri),
          go: () => void vscode.commands.executeCommand('corvis.revealLocation', r.file.uri, toPosition(r.file, r.route.offset)),
        });
      }
    }
    if (!items.length) {
      vscode.window.showInformationMessage('Corvis: no routes found in this workspace.');
      return;
    }
    const pick = await vscode.window.showQuickPick(items, { placeHolder: 'Search routes by method, path or handler', matchOnDescription: true });
    pick?.go?.();
  });

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('corvis.cli') || e.affectsConfiguration('corvis.frameworkPath')) resetCliCache();
      if (e.affectsConfiguration('corvis.explorer.exclude') && index.isEnabled) void index.rebuild();
    }),
  );

  return { index, modules, routes, ready };
}

export function deactivate(): void {
  // disposables are released through context.subscriptions
}
