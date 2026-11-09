// node --test test/   (after `npm run compile`)
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { test } from 'node:test';

const require = createRequire(import.meta.url);
const p = require('../out/parser.js');
const paths = require('../out/paths.js');

const SRC = String.raw`
use corvis::prelude::*;

/// Doc with #[get("/nope")] inside a comment.
#[module(
    global,
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        // a comment, with a comma
        UsersModule,
    ],
    controllers = [CatsController],
    providers = [CatsService, Provider::use_class_as::<PgRepo, dyn Repo>("REPO", |r| r), Provider::app_guard::<RolesGuard>()],
    exports = [CatsService],
)]
pub struct CatsModule;

#[controller(path = "cats", version = "2")]
#[use_guards(AuthGuard)]
pub(crate) struct CatsController {
    #[inject("API_KEY")]
    key: Arc<String>,
}

#[routes]
impl CatsController {
    #[get]
    fn find_all(&self) -> &'static str {
        let s = "#[post(\"fake\")] ]]";
        let r = r#"#[delete("raw")]"#;
        let c = '}';
        "x"
    }

    /// GET /cats/:id
    #[get(":id")]
    #[version(VERSION_NEUTRAL)]
    async fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Value> {
        if id > 0 { Ok(json!({})) } else { Err(NotFoundException::new("no")) }
    }

    #[post(["a", "b"])]
    #[http_code(204)]
    async fn create<'a>(&'a self, #[body] dto: Value) -> HttpResult<()> { Ok(()) }

    fn helper(&self) {}
}

impl Default for CatsController {
    fn default() -> Self { todo!() }
}

#[handlers]
impl TasksService {
    #[cron(CronExpression::EVERY_MINUTE, name = "tick")]
    async fn tick(&self) {}
}

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<CatsModule>().await?;
    app.set_global_prefix("api");
    app.enable_versioning(VersioningOptions::uri().default_version("1"));
    app.listen(3000).await
}
`;

test('modules, controllers, routes and app setup', () => {
  const r = p.parseRust(SRC);
  assert.equal(r.modules.length, 1);
  const m = r.modules[0];
  assert.equal(m.name, 'CatsModule');
  assert.equal(m.global, true);
  assert.deepEqual(m.imports.map(p.displayItem), ['ConfigModule::for_root(…)', 'UsersModule']);
  assert.deepEqual(m.controllers, ['CatsController']);
  assert.equal(m.providers.length, 3);
  assert.equal(p.referencedType(m.providers[2]), 'RolesGuard');
  assert.equal(p.referencedType(m.imports[0]), 'ConfigModule');

  assert.equal(r.controllers.length, 1);
  assert.deepEqual(r.controllers[0].paths, ['cats']);
  assert.deepEqual(r.controllers[0].versions, ['2']);

  assert.equal(r.routeImpls.length, 1);
  const routes = r.routeImpls[0].routes;
  assert.deepEqual(
    routes.map((x) => [x.method, x.paths, x.handler]),
    [
      ['GET', [''], 'find_all'],
      ['GET', [':id'], 'find_one'],
      ['POST', ['a', 'b'], 'create'],
    ],
  );
  assert.equal(r.handlers.length, 1);
  assert.equal(r.handlers[0].kind, 'cron');
  assert.equal(r.app.rootModule, 'CatsModule');
  assert.equal(r.app.globalPrefix, 'api');
  assert.equal(r.app.uriVersioning, true);
  assert.equal(r.app.defaultVersion, '1');

  const ctx = r.app;
  const c = r.controllers[0];
  assert.equal(p.fullRoutePath(routes[0], c, ctx), '/api/v2/cats');
  assert.equal(p.fullRoutePath(routes[1], c, ctx), '/api/cats/:id'); // VERSION_NEUTRAL
  assert.equal(p.fullRoutePath(routes[2], c, ctx, 1), '/api/v2/cats/b');
  assert.equal(p.fullRoutePath(routes[0], undefined, {}), '/');
});

test('attributes inside strings and comments are ignored', () => {
  const r = p.parseRust(SRC);
  const names = r.attrs.map((a) => a.name);
  assert.ok(!names.includes('delete'));
  assert.equal(names.filter((n) => n === 'post').length, 1);
});

test('categories', () => {
  const r = p.parseRust(SRC);
  const opts = p.fileCategorizeOptions(SRC);
  const byName = Object.fromEntries(r.attrs.map((a) => [a.path, p.categorize(a, opts)]));
  assert.equal(byName['module'], 'di');
  assert.equal(byName['get'], 'http');
  assert.equal(byName['use_guards'], 'enhancer');
  assert.equal(byName['param'], 'param');
  assert.equal(byName['cron'], 'event');
  assert.equal(byName['corvis::main'], 'di');
  assert.equal(byName['inject'], 'di');
  const other = p.parseRust('#[derive(Debug)]\n#[serde(rename_all = "camelCase")]\n#[id]\nstruct X;');
  assert.deepEqual(other.attrs.map((a) => p.categorize(a, p.fileCategorizeOptions(''))), [undefined, undefined, undefined]);
  assert.equal(p.categorize(other.attrs[2], { validation: false, orm: true }), 'orm');
});

test('splitTopLevel and literals', () => {
  assert.deepEqual(p.splitTopLevel('A, B<C, D>, f(1, 2), [x, y], "a,b"'), ['A', 'B<C, D>', 'f(1, 2)', '[x, y]', '"a,b"']);
  assert.deepEqual(p.splitTopLevel('Vec<Arc<T>>, U'), ['Vec<Arc<T>>', 'U']);
  assert.deepEqual(p.splitTopLevel('|a| a -> b, c'), ['|a| a -> b', 'c']);
  assert.deepEqual(p.stringList('["a", "b"]'), ['a', 'b']);
  assert.equal(p.stringLiteral('r#"x"#'), 'x');
  assert.equal(p.joinRoute('/api/', '', 'cats/', ':id'), '/api/cats/:id');
});

test('positions', () => {
  const src = 'a\nbc\n\nd';
  const starts = p.lineOffsets(src);
  assert.deepEqual(p.positionOf(starts, 3), { line: 1, character: 1 });
  assert.deepEqual(p.positionOf(starts, 6), { line: 3, character: 0 });
});

test('CLI --path and output parsing', () => {
  const src = 'C:/app/src';
  assert.equal(paths.featureDirFor({ src }, undefined), undefined);
  assert.equal(paths.featureDirFor({ src }, 'C:/app/src'), undefined);
  assert.equal(paths.featureDirFor({ src }, 'C:/app'), undefined);
  assert.equal(paths.featureDirFor({ src }, 'C:/app/src/users'), 'users');
  assert.equal(paths.featureDirFor({ src }, 'C:/app/src/users/dto'), null);
  const out = 'CREATE C:/app/src/users/users_controller.rs (812 bytes)\nUPDATE C:/app/src/main.rs (40 bytes)\nSKIP C:\\app\\src\\app_module.rs (could not update automatically)\n';
  const ch = paths.parseChanges(out, 'C:/app');
  assert.deepEqual(ch.map((c) => c.kind), ['CREATE', 'UPDATE', 'SKIP']);
  assert.ok(ch[0].file.replace(/\\/g, '/').endsWith('src/users/users_controller.rs'));
});

test('resolveCorvisPath', () => {
  const win = process.platform === 'win32';
  const root = win ? 'C:/src/corvis' : '/src/corvis';
  const home = win ? 'C:/Users/me' : '/home/me';
  const norm = (s) => s.replace(/\\/g, '/');
  const files = new Set([
    `${root}/crates/corvis/Cargo.toml`,
    `${root}/crates/corvis-cli/Cargo.toml`,
    `${home}/src/corvis/crates/corvis/Cargo.toml`,
    `${home}/src/corvis/crates/corvis-cli/Cargo.toml`,
    `${root}/empty/placeholder`,
  ]);
  const exists = (f) => files.has(norm(f));
  const resolve = (v) => {
    const r = paths.resolveCorvisPath(v, home, exists);
    return r.path === undefined ? r : { ...r, path: norm(r.path) };
  };
  assert.deepEqual(resolve(undefined), {});
  assert.deepEqual(resolve(''), {});
  assert.deepEqual(resolve('   '), {});
  assert.deepEqual(resolve(`${root}/crates/corvis`), { path: `${root}/crates/corvis` });
  assert.deepEqual(resolve(`  ${root}/crates/corvis  `), { path: `${root}/crates/corvis` });
  assert.deepEqual(resolve(root), { path: `${root}/crates/corvis` });
  assert.deepEqual(resolve('~/src/corvis'), { path: `${home}/src/corvis/crates/corvis` });
  const rel = resolve('../corvis/crates/corvis');
  assert.match(rel.error, /corvis\.new\.corvisPath must be an absolute path/);
  assert.equal(rel.path, undefined);
  const none = resolve(`${root}/empty`);
  assert.match(none.error, /corvis\.new\.corvisPath points at .*empty, which has no Cargo\.toml/);
  if (win) {
    assert.deepEqual(resolve('C:\\src\\corvis\\crates\\corvis'), { path: 'C:/src/corvis/crates/corvis' });
  }
});
