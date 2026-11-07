import * as vscode from 'vscode';

import { CrateIndex, IndexedFile, IndexedRoute, Located, WorkspaceIndex, toPosition } from './index';
import { ControllerDef, ModuleDef, displayItem, referencedType } from './parser';

export type Node =
  | { kind: 'crate'; crate: CrateIndex; view: 'modules' | 'routes' }
  | { kind: 'module'; crate: CrateIndex; module: Located<ModuleDef> }
  | { kind: 'group'; crate: CrateIndex; label: string; icon: string; children: () => Node[]; expanded: boolean }
  | { kind: 'controller'; crate: CrateIndex; name: string; controller?: Located<ControllerDef>; routes: IndexedRoute[] }
  | { kind: 'route'; crate: CrateIndex; route: IndexedRoute }
  | { kind: 'provider'; crate: CrateIndex; text: string; name?: string; file?: IndexedFile; offset?: number; children: Node[] }
  | { kind: 'ref'; text: string; icon: string; file?: IndexedFile; offset?: number; tooltip?: string };

const METHOD_COLORS: Record<string, string> = {
  GET: 'charts.green',
  POST: 'charts.yellow',
  PUT: 'charts.blue',
  PATCH: 'charts.purple',
  DELETE: 'charts.red',
};

export function methodIcon(method: string): vscode.ThemeIcon {
  const color = METHOD_COLORS[method];
  return new vscode.ThemeIcon('symbol-method', color ? new vscode.ThemeColor(color) : undefined);
}

function providerIcon(name: string): string {
  if (/Guard$/.test(name)) return 'shield';
  if (/Interceptor$/.test(name)) return 'arrow-swap';
  if (/Pipe$/.test(name)) return 'filter';
  if (/Filter$/.test(name)) return 'warning';
  if (/Middleware$/.test(name)) return 'layers';
  if (/Gateway$/.test(name)) return 'plug';
  if (/^Provider::/.test(name)) return 'symbol-constant';
  return 'symbol-class';
}

function revealCommand(file: IndexedFile | undefined, offset: number | undefined): vscode.Command | undefined {
  if (!file || offset === undefined) return undefined;
  return { command: 'corvis.revealLocation', title: 'Open', arguments: [file.uri, toPosition(file, offset)] };
}

function controllerPath(c?: ControllerDef): string {
  if (!c) return '';
  const p = c.paths[0] ?? '';
  return '/' + p.replace(/^\/+/, '');
}

function controllerNode(crate: CrateIndex, name: string): Node {
  const controller = crate.controllers.find((c) => c.def.name === name);
  const routes = crate.routes.filter((r) => r.owner === name);
  return { kind: 'controller', crate, name, controller, routes };
}

abstract class BaseProvider implements vscode.TreeDataProvider<Node> {
  protected readonly emitter = new vscode.EventEmitter<Node | undefined>();
  readonly onDidChangeTreeData = this.emitter.event;

  constructor(protected readonly index: WorkspaceIndex) {
    index.onDidChange(() => this.emitter.fire(undefined));
  }

  refresh(): void {
    this.emitter.fire(undefined);
  }

  abstract getChildren(node?: Node): Promise<Node[]>;

  protected multiCrate(): boolean {
    return this.index.getCrates().length > 1;
  }

  getTreeItem(node: Node): vscode.TreeItem {
    switch (node.kind) {
      case 'crate': {
        const item = new vscode.TreeItem(node.crate.name, vscode.TreeItemCollapsibleState.Expanded);
        item.iconPath = new vscode.ThemeIcon('package');
        item.description = node.crate.root ? vscode.workspace.asRelativePath(node.crate.root) : undefined;
        item.contextValue = 'corvis.crate';
        return item;
      }
      case 'module': {
        const m = node.module.def;
        const item = new vscode.TreeItem(m.name, vscode.TreeItemCollapsibleState.Collapsed);
        item.iconPath = new vscode.ThemeIcon('symbol-module', m.name === node.crate.app.rootModule ? new vscode.ThemeColor('charts.orange') : undefined);
        const tags = [];
        if (m.name === node.crate.app.rootModule) tags.push('root');
        if (m.global) tags.push('global');
        item.description = tags.join(', ') || undefined;
        item.tooltip = new vscode.MarkdownString(
          [`**${m.name}**`, m.imports.length ? `imports: ${m.imports.map(displayItem).join(', ')}` : '', m.controllers.length ? `controllers: ${m.controllers.join(', ')}` : '', m.providers.length ? `providers: ${m.providers.map(displayItem).join(', ')}` : '', m.exports.length ? `exports: ${m.exports.map(displayItem).join(', ')}` : '']
            .filter(Boolean)
            .join('\n\n'),
        );
        item.command = revealCommand(node.module.file, m.offset);
        item.contextValue = 'corvis.module';
        return item;
      }
      case 'group': {
        const item = new vscode.TreeItem(node.label, node.expanded ? vscode.TreeItemCollapsibleState.Expanded : vscode.TreeItemCollapsibleState.Collapsed);
        item.iconPath = new vscode.ThemeIcon(node.icon);
        item.contextValue = 'corvis.group';
        return item;
      }
      case 'controller': {
        const item = new vscode.TreeItem(node.name, node.routes.length ? vscode.TreeItemCollapsibleState.Expanded : vscode.TreeItemCollapsibleState.None);
        item.iconPath = new vscode.ThemeIcon('symbol-class', new vscode.ThemeColor('charts.blue'));
        const c = node.controller?.def;
        const extra = [];
        if (c?.versions.length) extra.push(`v${c.versions.join(', v')}`);
        if (c?.host) extra.push(`host ${c.host}`);
        item.description = [controllerPath(c), ...extra].join('  ');
        item.tooltip = c ? `${node.name}  ${c.paths.map((p) => '/' + p.replace(/^\/+/, '')).join(', ') || '/'}  (${node.routes.length} routes)` : `${node.name} (no #[controller] found)`;
        item.command = revealCommand(node.controller?.file, c?.offset);
        item.contextValue = 'corvis.controller';
        return item;
      }
      case 'route': {
        const r = node.route;
        const label = `${r.route.method} ${r.fullPath}`;
        const item = new vscode.TreeItem({ label, highlights: [[0, r.route.method.length]] }, vscode.TreeItemCollapsibleState.None);
        item.iconPath = methodIcon(r.route.method);
        item.description = r.route.handler + (r.route.attrName === 'sse' ? ' (SSE)' : '');
        const md = new vscode.MarkdownString();
        md.appendMarkdown(`**${r.route.method}** \`${r.fullPath}\`\n\n`);
        md.appendMarkdown(`\`${r.owner}::${r.route.handler}\`${r.route.isAsync ? ' (async)' : ''}\n\n`);
        if (r.route.paths.length > 1) md.appendMarkdown(`paths: ${r.route.paths.map((p) => `\`${p}\``).join(', ')}\n\n`);
        if (r.route.versions.length) md.appendMarkdown(`version: ${r.route.versions.join(', ')}\n\n`);
        md.appendMarkdown(vscode.workspace.asRelativePath(r.file.uri));
        item.tooltip = md;
        item.command = revealCommand(r.file, r.route.offset);
        item.contextValue = 'corvis.route';
        return item;
      }
      case 'provider': {
        const item = new vscode.TreeItem(node.text, node.children.length ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None);
        item.iconPath = new vscode.ThemeIcon(providerIcon(node.name ?? node.text));
        item.command = revealCommand(node.file, node.offset);
        item.contextValue = 'corvis.provider';
        return item;
      }
      case 'ref': {
        const item = new vscode.TreeItem(node.text, vscode.TreeItemCollapsibleState.None);
        item.iconPath = new vscode.ThemeIcon(node.icon);
        item.tooltip = node.tooltip;
        item.command = revealCommand(node.file, node.offset);
        return item;
      }
    }
  }
}

export class ModulesTree extends BaseProvider {
  async getChildren(node?: Node): Promise<Node[]> {
    if (!node) {
      await this.index.ready();
      const crates = this.index.getCrates().filter((c) => c.modules.length);
      if (crates.length === 1) return this.modulesOf(crates[0]);
      return crates.map((crate) => ({ kind: 'crate', crate, view: 'modules' }));
    }
    switch (node.kind) {
      case 'crate':
        return this.modulesOf(node.crate);
      case 'module':
        return this.moduleChildren(node.crate, node.module);
      case 'group':
        return node.children();
      case 'controller':
        return node.routes.map((route) => ({ kind: 'route', crate: node.crate, route }));
      case 'provider':
        return node.children;
      default:
        return [];
    }
  }

  private modulesOf(crate: CrateIndex): Node[] {
    const root = crate.app.rootModule;
    return [...crate.modules]
      .sort((a, b) => Number(b.def.name === root) - Number(a.def.name === root) || a.def.name.localeCompare(b.def.name))
      .map((module) => ({ kind: 'module', crate, module }));
  }

  private moduleChildren(crate: CrateIndex, module: Located<ModuleDef>): Node[] {
    const m = module.def;
    const groups: Node[] = [];
    if (m.imports.length) {
      groups.push({
        kind: 'group',
        crate,
        label: `Imports (${m.imports.length})`,
        icon: 'references',
        expanded: false,
        children: () =>
          m.imports.map((expr): Node => {
            const ref = referencedType(expr);
            const target = crate.modules.find((x) => x.def.name === ref);
            if (target && displayItem(expr) === target.def.name) return { kind: 'module', crate, module: target };
            return { kind: 'ref', text: displayItem(expr), icon: target ? 'symbol-module' : 'library', file: target?.file, offset: target?.def.offset, tooltip: expr.replace(/\s+/g, ' ') };
          }),
      });
    }
    if (m.controllers.length) {
      groups.push({
        kind: 'group',
        crate,
        label: `Controllers (${m.controllers.length})`,
        icon: 'symbol-class',
        expanded: true,
        children: () => m.controllers.map((expr) => controllerNode(crate, referencedType(expr) ?? expr)),
      });
    }
    if (m.providers.length) {
      groups.push({
        kind: 'group',
        crate,
        label: `Providers (${m.providers.length})`,
        icon: 'symbol-interface',
        expanded: true,
        children: () => m.providers.map((expr) => this.providerNode(crate, expr)),
      });
    }
    if (m.exports.length) {
      groups.push({
        kind: 'group',
        crate,
        label: `Exports (${m.exports.length})`,
        icon: 'export',
        expanded: false,
        children: () =>
          m.exports.map((expr): Node => {
            const ref = referencedType(expr);
            const p = crate.providers.find((x) => x.def.name === ref) ?? crate.modules.find((x) => x.def.name === ref);
            return { kind: 'ref', text: displayItem(expr), icon: providerIcon(ref ?? expr), file: p?.file, offset: p?.def.offset };
          }),
      });
    }
    return groups;
  }

  private providerNode(crate: CrateIndex, expr: string): Node {
    const ref = referencedType(expr);
    const def = crate.providers.find((x) => x.def.name === ref);
    const children: Node[] = ref
      ? this.index.handlersOf(crate, ref).map((h) => ({
          kind: 'ref',
          text: `${h.handler}`,
          icon: h.kind === 'subscribe_message' ? 'radio-tower' : h.kind === 'on_event' ? 'zap' : 'watch',
          file: h.file,
          offset: h.offset,
          tooltip: `#[${h.kind}(${h.args})]`,
        }))
      : [];
    for (const c of children) if (c.kind === 'ref') c.text = `${c.text}  ${c.tooltip}`;
    return { kind: 'provider', crate, text: displayItem(expr), name: ref, file: def?.file, offset: def?.def.offset, children };
  }
}

export class RoutesTree extends BaseProvider {
  async getChildren(node?: Node): Promise<Node[]> {
    if (!node) {
      await this.index.ready();
      const crates = this.index.getCrates().filter((c) => c.routes.length || c.controllers.length);
      if (crates.length === 1) return this.controllersOf(crates[0]);
      return crates.map((crate) => ({ kind: 'crate', crate, view: 'routes' }));
    }
    if (node.kind === 'crate') return this.controllersOf(node.crate);
    if (node.kind === 'controller') return node.routes.map((route) => ({ kind: 'route', crate: node.crate, route }));
    return [];
  }

  private controllersOf(crate: CrateIndex): Node[] {
    const names = new Set([...crate.controllers.map((c) => c.def.name), ...crate.routes.map((r) => r.owner)]);
    return [...names]
      .map((name) => controllerNode(crate, name))
      .sort((a, b) => {
        const pa = a.kind === 'controller' ? controllerPath(a.controller?.def) : '';
        const pb = b.kind === 'controller' ? controllerPath(b.controller?.def) : '';
        return pa.localeCompare(pb);
      });
  }
}
