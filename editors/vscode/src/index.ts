import * as path from 'path';
import * as vscode from 'vscode';

import { AppSetup, ControllerDef, ModuleDef, ParsedFile, ProviderDef, RouteContext, RouteDef, fullRoutePath, lineOffsets, parseRust, positionOf } from './parser';
import { CargoProject, findCargoProject } from './project';

export interface IndexedFile {
  uri: vscode.Uri;
  parsed: ParsedFile;
  starts: number[];
  crate?: CargoProject;
}

export interface Located<T> {
  def: T;
  file: IndexedFile;
}

export interface IndexedRoute {
  route: RouteDef;
  owner: string;
  controller?: Located<ControllerDef>;
  file: IndexedFile;
  fullPath: string;
}

export interface CrateIndex {
  key: string;
  name: string;
  root?: string;
  modules: Located<ModuleDef>[];
  controllers: Located<ControllerDef>[];
  providers: Located<ProviderDef>[];
  routes: IndexedRoute[];
  app: RouteContext & { rootModule?: string };
}

const INTERESTING = /#\s*\[\s*(?:corvis\s*::\s*)?(?:module|controller|routes|injectable|handlers|websocket_gateway)\b|CorvisFactory|set_global_prefix/;

export function isInteresting(text: string): boolean {
  return INTERESTING.test(text);
}

export function toPosition(file: { starts: number[] }, offset: number): vscode.Position {
  const p = positionOf(file.starts, offset);
  return new vscode.Position(p.line, p.character);
}

/** Index of the Corvis items declared in the workspace's `.rs` files. */
export class WorkspaceIndex implements vscode.Disposable {
  private files = new Map<string, IndexedFile>();
  private crateCache = new Map<string, CargoProject | undefined>();
  private crates: CrateIndex[] | undefined;
  private readonly emitter = new vscode.EventEmitter<void>();
  readonly onDidChange = this.emitter.event;
  private watcher: vscode.FileSystemWatcher | undefined;
  private pending = new Map<string, vscode.Uri | null>();
  private flushTimer: NodeJS.Timeout | undefined;
  private building: Promise<void> | undefined;
  private enabled = false;

  enable(): void {
    if (this.enabled) return;
    this.enabled = true;
    this.watcher = vscode.workspace.createFileSystemWatcher('**/*.rs');
    this.watcher.onDidCreate((u) => this.queue(u, u));
    this.watcher.onDidChange((u) => this.queue(u, u));
    this.watcher.onDidDelete((u) => this.queue(u, null));
    void this.rebuild();
  }

  get isEnabled(): boolean {
    return this.enabled;
  }

  rebuild(): Promise<void> {
    this.building = (async () => {
      const exclude = vscode.workspace.getConfiguration('corvis').get<string>('explorer.exclude') || '**/{target,node_modules,.git}/**';
      const uris = await vscode.workspace.findFiles('**/*.rs', exclude);
      this.files.clear();
      this.crateCache.clear();
      const batch = 32;
      for (let i = 0; i < uris.length; i += batch) {
        await Promise.all(uris.slice(i, i + batch).map((u) => this.load(u)));
      }
      this.invalidate();
    })();
    return this.building;
  }

  async ready(): Promise<void> {
    if (!this.building) await this.rebuild();
    else await this.building;
  }

  private excluded(uri: vscode.Uri): boolean {
    return /[\\/](target|node_modules|\.git)[\\/]/.test(uri.fsPath);
  }

  private queue(uri: vscode.Uri, value: vscode.Uri | null) {
    if (this.excluded(uri)) return;
    this.pending.set(uri.toString(), value);
    if (this.flushTimer) clearTimeout(this.flushTimer);
    this.flushTimer = setTimeout(() => void this.flush(), 300);
  }

  private async flush() {
    const items = [...this.pending.entries()];
    this.pending.clear();
    for (const [key, uri] of items) {
      if (uri) await this.load(uri);
      else this.files.delete(key);
    }
    this.invalidate();
  }

  private async load(uri: vscode.Uri) {
    let text: string;
    try {
      text = Buffer.from(await vscode.workspace.fs.readFile(uri)).toString('utf8');
    } catch {
      this.files.delete(uri.toString());
      return;
    }
    this.setText(uri, text);
  }

  private setText(uri: vscode.Uri, text: string) {
    if (!isInteresting(text)) {
      this.files.delete(uri.toString());
      return;
    }
    const parsed = parseRust(text);
    this.files.set(uri.toString(), { uri, parsed, starts: lineOffsets(text), crate: this.crateOf(uri) });
  }

  private crateOf(uri: vscode.Uri): CargoProject | undefined {
    const dir = path.dirname(uri.fsPath);
    if (!this.crateCache.has(dir)) this.crateCache.set(dir, findCargoProject(dir));
    return this.crateCache.get(dir);
  }

  private invalidate() {
    this.crates = undefined;
    this.emitter.fire();
  }

  /** Items grouped per Cargo package. */
  getCrates(): CrateIndex[] {
    if (this.crates) return this.crates;
    const map = new Map<string, CrateIndex>();
    const get = (f: IndexedFile) => {
      const key = f.crate?.root ?? '';
      let c = map.get(key);
      if (!c) {
        c = { key, name: f.crate?.name ?? 'workspace', root: f.crate?.root, modules: [], controllers: [], providers: [], routes: [], app: {} };
        map.set(key, c);
      }
      return c;
    };
    const apps = new Map<string, { setup: AppSetup; file: IndexedFile }[]>();
    for (const f of this.files.values()) {
      const c = get(f);
      c.modules.push(...f.parsed.modules.map((def) => ({ def, file: f })));
      c.controllers.push(...f.parsed.controllers.map((def) => ({ def, file: f })));
      c.providers.push(...f.parsed.providers.map((def) => ({ def, file: f })));
      if (f.parsed.app) {
        const list = apps.get(c.key) ?? [];
        list.push({ setup: f.parsed.app, file: f });
        apps.set(c.key, list);
      }
    }
    for (const c of map.values()) {
      // Prefer src/main.rs over binaries in src/bin when merging the app setup.
      const list = (apps.get(c.key) ?? []).sort((a, b) => Number(/[\\/]bin[\\/]/.test(a.file.uri.fsPath)) - Number(/[\\/]bin[\\/]/.test(b.file.uri.fsPath)));
      for (const { setup } of list) {
        c.app.rootModule ??= setup.rootModule;
        c.app.globalPrefix ??= setup.globalPrefix;
        if (setup.uriVersioning) {
          c.app.uriVersioning = true;
          c.app.defaultVersion ??= setup.defaultVersion;
        }
      }
    }
    for (const f of this.files.values()) {
      const c = get(f);
      for (const impl of f.parsed.routeImpls) {
        const controller = c.controllers.find((x) => x.def.name === impl.owner);
        for (const route of impl.routes) {
          c.routes.push({ route, owner: impl.owner, controller, file: f, fullPath: fullRoutePath(route, controller?.def, c.app) });
        }
      }
    }
    this.crates = [...map.values()]
      .filter((c) => c.modules.length || c.controllers.length || c.routes.length || c.providers.length)
      .sort((a, b) => a.name.localeCompare(b.name));
    return this.crates;
  }

  crateFor(uri: vscode.Uri): CrateIndex | undefined {
    const root = this.crateOf(uri)?.root ?? '';
    return this.getCrates().find((c) => c.key === root);
  }

  handlersOf(c: CrateIndex, owner: string) {
    const out: { kind: string; args: string; handler: string; offset: number; file: IndexedFile }[] = [];
    for (const f of this.files.values()) {
      if ((f.crate?.root ?? '') !== c.key) continue;
      for (const h of f.parsed.handlers) if (h.owner === owner) out.push({ ...h, file: f });
    }
    return out;
  }

  dispose(): void {
    this.watcher?.dispose();
    this.emitter.dispose();
    if (this.flushTimer) clearTimeout(this.flushTimer);
  }
}
