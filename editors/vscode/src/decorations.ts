import * as vscode from 'vscode';

import { WorkspaceIndex } from './index';
import { AttrCategory, categorize, fileCategorizeOptions, fullRoutePath, parseRust } from './parser';

const CATEGORIES: AttrCategory[] = ['di', 'http', 'enhancer', 'param', 'event', 'swagger', 'validation', 'orm'];
const MAX_SIZE = 1_500_000;

/**
 * Colors Corvis attribute names and shows the full path of each route next to its
 * `#[get]`/`#[post]`/... attribute. Decorations are drawn on top of semantic tokens, so
 * they stay visible when rust-analyzer's semantic highlighting replaces TextMate colors.
 */
export class CorvisDecorations implements vscode.Disposable {
  private types = new Map<AttrCategory, vscode.TextEditorDecorationType>();
  private hint: vscode.TextEditorDecorationType;
  private timers = new Map<string, NodeJS.Timeout>();
  private disposables: vscode.Disposable[] = [];

  constructor(private readonly index: WorkspaceIndex) {
    for (const c of CATEGORIES) {
      this.types.set(
        c,
        vscode.window.createTextEditorDecorationType({
          color: new vscode.ThemeColor(`corvis.attribute.${c}`),
          fontWeight: c === 'http' || c === 'di' ? '600' : undefined,
        }),
      );
    }
    this.hint = vscode.window.createTextEditorDecorationType({
      after: { color: new vscode.ThemeColor('corvis.routeHint'), margin: '0 0 0 1.5em', fontStyle: 'italic' },
    });
    this.disposables.push(
      vscode.window.onDidChangeVisibleTextEditors((eds) => eds.forEach((e) => this.schedule(e, 0))),
      vscode.workspace.onDidChangeTextDocument((e) => {
        for (const ed of vscode.window.visibleTextEditors) if (ed.document === e.document) this.schedule(ed, 250);
      }),
      vscode.workspace.onDidChangeConfiguration((e) => {
        if (e.affectsConfiguration('corvis.decorations') || e.affectsConfiguration('corvis.routeHints')) this.refreshAll();
      }),
      index.onDidChange(() => this.refreshAll()),
    );
    this.refreshAll();
  }

  refreshAll(): void {
    for (const ed of vscode.window.visibleTextEditors) this.schedule(ed, 0);
  }

  private schedule(editor: vscode.TextEditor, delay: number) {
    if (editor.document.languageId !== 'rust') return;
    const key = editor.document.uri.toString();
    const t = this.timers.get(key);
    if (t) clearTimeout(t);
    this.timers.set(
      key,
      setTimeout(() => {
        this.timers.delete(key);
        this.apply(editor);
      }, delay),
    );
  }

  private apply(editor: vscode.TextEditor) {
    const config = vscode.workspace.getConfiguration('corvis');
    const colors = config.get<boolean>('decorations.enabled', true);
    const hints = config.get<boolean>('routeHints.enabled', true);
    const doc = editor.document;
    const text = doc.getText();
    const byCategory = new Map<AttrCategory, vscode.Range[]>();
    const hintOptions: vscode.DecorationOptions[] = [];

    // Only in Corvis workspaces: other frameworks (actix, rocket) also have #[get] attributes.
    if (this.index.isEnabled && (colors || hints) && text.length <= MAX_SIZE && text.includes('#')) {
      const parsed = parseRust(text);
      if (colors) {
        const opts = fileCategorizeOptions(text);
        for (const a of parsed.attrs) {
          const cat = categorize(a, opts);
          if (!cat || a.nameEnd <= a.nameStart) continue;
          const list = byCategory.get(cat) ?? [];
          list.push(new vscode.Range(doc.positionAt(a.nameStart), doc.positionAt(a.nameEnd)));
          byCategory.set(cat, list);
        }
      }
      if (hints && parsed.routeImpls.length) {
        const crate = this.index.crateFor(doc.uri);
        const ctx = crate?.app ?? {};
        for (const impl of parsed.routeImpls) {
          const controller = parsed.controllers.find((c) => c.name === impl.owner) ?? crate?.controllers.find((c) => c.def.name === impl.owner)?.def;
          for (const r of impl.routes) {
            const pos = doc.positionAt(r.attrEnd);
            const full = fullRoutePath(r, controller, ctx);
            const more = r.paths.length > 1 ? ` (+${r.paths.length - 1})` : '';
            hintOptions.push({
              range: new vscode.Range(pos, pos),
              renderOptions: { after: { contentText: `${r.method} ${full}${more}${r.attrName === 'sse' ? ' · SSE' : ''}` } },
            });
          }
        }
      }
    }
    for (const [cat, type] of this.types) editor.setDecorations(type, byCategory.get(cat) ?? []);
    editor.setDecorations(this.hint, hintOptions);
  }

  dispose(): void {
    for (const t of this.timers.values()) clearTimeout(t);
    for (const t of this.types.values()) t.dispose();
    this.hint.dispose();
    this.disposables.forEach((d) => d.dispose());
  }
}
