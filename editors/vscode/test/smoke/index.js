// Runs inside the VS Code extension host (see scripts/smoke.mjs). Opens a Corvis app created
// with `corvis new` + `corvis g resource cats` and checks activation, the index, the trees
// and the generate command (input boxes and quick picks are stubbed).
const assert = require('assert');
const fs = require('fs');
const path = require('path');
const vscode = require('vscode');

async function waitFor(fn, ms = 20000) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() > end) throw new Error('timeout waiting for condition');
    await new Promise((r) => setTimeout(r, 200));
  }
}

async function labels(provider, node) {
  const kids = await provider.getChildren(node);
  return kids.map((k) => {
    const item = provider.getTreeItem(k);
    return { node: k, label: typeof item.label === 'string' ? item.label : item.label.label, description: item.description };
  });
}

async function run() {
  const log = [];
  const ext = vscode.extensions.getExtension('randygil.corvis-vscode');
  assert.ok(ext, 'extension is installed in the development host');
  const api = await ext.activate();
  await api.ready;
  log.push('activated');

  const commands = await vscode.commands.getCommands(true);
  for (const c of ['corvis.generate', 'corvis.generate.resource', 'corvis.new', 'corvis.goToRoute', 'corvis.start']) {
    assert.ok(commands.includes(c), `command ${c} registered`);
  }

  await api.index.ready();
  const crates = api.index.getCrates();
  assert.equal(crates.length, 1, 'one crate');
  const crate = crates[0];
  const routes = crate.routes.map((r) => `${r.route.method} ${r.fullPath}`).sort();
  log.push('routes: ' + routes.join(', '));
  assert.ok(routes.includes('GET /cats/:id'), 'resource routes indexed');
  assert.ok(routes.includes('GET /'), 'app controller route indexed');
  assert.equal(crate.app.rootModule, 'AppModule');

  // Modules tree: AppModule first, with its imports/controllers/providers groups.
  const roots = await labels(api.modules);
  log.push('modules: ' + roots.map((r) => `${r.label}${r.description ? ` (${r.description})` : ''}`).join(', '));
  assert.equal(roots[0].label, 'AppModule');
  assert.equal(roots[0].description, 'root');
  const groups = await labels(api.modules, roots[0].node);
  assert.deepEqual(groups.map((g) => g.label.replace(/ \(\d+\)/, '')), ['Imports', 'Controllers', 'Providers']);
  const imports = await labels(api.modules, groups[0].node);
  assert.ok(imports.some((i) => i.label === 'CatsModule'), 'CatsModule linked from AppModule imports');

  const routeRoots = await labels(api.routes);
  const cats = routeRoots.find((r) => r.label === 'CatsController');
  assert.ok(cats, 'CatsController in routes view');
  const catRoutes = await labels(api.routes, cats.node);
  assert.equal(catRoutes.length, 5);

  // Decorations / grammar must not break when opening a controller.
  const folder = vscode.workspace.workspaceFolders[0].uri.fsPath;
  const doc = await vscode.workspace.openTextDocument(path.join(folder, 'src', 'cats', 'cats_controller.rs'));
  await vscode.window.showTextDocument(doc);
  await new Promise((r) => setTimeout(r, 1500));
  try {
    const tokens = await vscode.commands.executeCommand('_workbench.captureSyntaxTokens', doc.uri);
    const corvis = (tokens || []).filter((t) => /corvis/.test(t.t));
    const pick = (re) => corvis.find((t) => re.test(t.c));
    assert.ok(pick(/^controller$/) && /keyword\.control\.decorator\.di\.corvis/.test(pick(/^controller$/).t), 'controller attribute scoped by the injection');
    assert.ok(pick(/^get$/) && /support\.type\.decorator\.http\.corvis/.test(pick(/^get$/).t), 'route attribute scoped');
    log.push(`grammar: ${corvis.length} tokens with corvis scopes`);
  } catch (e) {
    if (e instanceof assert.AssertionError) throw e;
    log.push('grammar: _workbench.captureSyntaxTokens unavailable (' + e.message + ')');
  }
  if (process.platform === 'win32' && process.env.CORVIS_SMOKE_DIR) {
    const out = path.join(process.env.CORVIS_SMOKE_DIR, 'controller.png');
    const r = require('child_process').spawnSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', path.join(__dirname, 'capture.ps1'), '-Title', 'cats_controller.rs', '-Out', out], { encoding: 'utf8' });
    log.push('screenshot: ' + (r.stdout || r.stderr || '').trim());
  }

  // Generate: right-click on src/cats -> Corvis -> Generate Guard, name "cats".
  const origInput = vscode.window.showInputBox;
  const origPick = vscode.window.showQuickPick;
  vscode.window.showInputBox = async (opts) => opts.value || 'cats';
  vscode.window.showQuickPick = async (items, opts) => (opts && opts.canPickMany ? [] : (await items)[0]);
  try {
    await vscode.commands.executeCommand('corvis.generate.guard', vscode.Uri.file(path.join(folder, 'src', 'cats')));
    const guard = path.join(folder, 'src', 'cats', 'cats_guard.rs');
    await waitFor(() => fs.existsSync(guard), 240000);
    assert.ok(fs.readFileSync(path.join(folder, 'src', 'cats', 'mod.rs'), 'utf8').includes('pub mod cats_guard;'));
    log.push('generated ' + path.relative(folder, guard));

    // Palette flow with a schematic quick pick: "Corvis: Generate…" -> first item (resource) "dogs".
    vscode.window.showInputBox = async () => 'dogs';
    await vscode.commands.executeCommand('corvis.generate', vscode.Uri.file(path.join(folder, 'src')));
    await waitFor(() => fs.existsSync(path.join(folder, 'src', 'dogs', 'dogs_controller.rs')), 240000);
    assert.ok(fs.readFileSync(path.join(folder, 'src', 'app_module.rs'), 'utf8').includes('DogsModule'));
    log.push('generated resource dogs');
    await waitFor(() => api.index.getCrates()[0].routes.some((r) => r.fullPath === '/dogs/:id'), 10000);
    log.push('index refreshed with /dogs/:id');
  } finally {
    vscode.window.showInputBox = origInput;
    vscode.window.showQuickPick = origPick;
  }
  fs.writeFileSync(process.env.CORVIS_SMOKE_LOG, log.join('\n') + '\nOK\n');
}

module.exports = { run };
