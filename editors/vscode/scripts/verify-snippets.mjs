// Expands every snippet with its default values, writes each one into its own module of a
// scratch Cargo crate that depends on corvis by path, and runs `cargo check --all-targets`.
//
//   node scripts/verify-snippets.mjs [--crate <dir>] [--no-check]
//
// The crate defaults to ../../../corvis-tests/tmp/vscode (outside the Corvis workspace).
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const crateDir = resolve(args.includes('--crate') ? args[args.indexOf('--crate') + 1] : join(here, '../../../../corvis-tests/tmp/vscode'));
const corvisRoot = resolve(here, '../../..');

// ------------------------------------------------------------------ snippet expansion

/** Parses VS Code snippet syntax into nodes. */
function parse(src) {
  let i = 0;
  function readUntil(stop) {
    const nodes = [];
    let text = '';
    const flush = () => {
      if (text) nodes.push({ t: 'text', v: text });
      text = '';
    };
    while (i < src.length) {
      const c = src[i];
      if (c === '\\' && i + 1 < src.length && '$}\\,|'.includes(src[i + 1])) {
        text += src[i + 1];
        i += 2;
        continue;
      }
      if (stop && stop.includes(c)) break;
      if (c === '$') {
        const simple = /^\$(\d+|[A-Z_]+)/.exec(src.slice(i));
        if (simple) {
          flush();
          nodes.push(/^\d/.test(simple[1]) ? { t: 'tab', n: +simple[1] } : { t: 'var', name: simple[1] });
          i += simple[0].length;
          continue;
        }
        const head = /^\$\{(\d+|[A-Z_]+)/.exec(src.slice(i));
        if (head) {
          flush();
          i += head[0].length;
          const id = head[1];
          const numeric = /^\d/.test(id);
          const next = src[i];
          if (next === '}') {
            i++;
            nodes.push(numeric ? { t: 'tab', n: +id } : { t: 'var', name: id });
          } else if (next === ':') {
            i++;
            const children = readUntil('}');
            i++;
            nodes.push(numeric ? { t: 'ph', n: +id, children } : { t: 'var', name: id, children });
          } else if (next === '|') {
            i++;
            const opts = [];
            let cur = '';
            while (i < src.length && !(src[i] === '|' && src[i + 1] === '}')) {
              if (src[i] === '\\') {
                cur += src[i + 1];
                i += 2;
                continue;
              }
              if (src[i] === ',') {
                opts.push(cur);
                cur = '';
              } else cur += src[i];
              i++;
            }
            opts.push(cur);
            i += 2;
            nodes.push({ t: 'choice', n: +id, opts });
          } else if (next === '/') {
            i++;
            const part = () => {
              let s = '';
              while (i < src.length && src[i] !== '/') {
                if (src[i] === '\\') {
                  s += src[i] + src[i + 1];
                  i += 2;
                  continue;
                }
                s += src[i++];
              }
              i++;
              return s;
            };
            const regex = part();
            // format may contain ${1:/downcase}: read up to the next unescaped '/'
            let format = '';
            let depth = 0;
            while (i < src.length && !(src[i] === '/' && depth === 0)) {
              if (src[i] === '{') depth++;
              if (src[i] === '}') depth--;
              format += src[i++];
            }
            i++;
            let flags = '';
            while (src[i] !== '}') flags += src[i++];
            i++;
            nodes.push({ t: 'transform', n: +id, regex, format, flags });
          } else throw new Error(`bad snippet syntax near ${src.slice(i - 10, i + 10)}`);
          continue;
        }
      }
      text += c;
      i++;
    }
    flush();
    return nodes;
  }
  return readUntil(null);
}

function expand(body) {
  const nodes = parse(body);
  const values = new Map();
  // first pass: collect defaults (first definition wins)
  (function collect(list) {
    for (const n of list) {
      if (n.t === 'ph' && !values.has(n.n)) {
        collect(n.children);
        values.set(n.n, render(n.children));
      } else if (n.t === 'ph') collect(n.children);
      else if (n.t === 'choice' && !values.has(n.n)) values.set(n.n, n.opts[0]);
    }
  })(nodes);
  function applyFormat(format, m) {
    return format.replace(/\$\{(\d+):\/(downcase|upcase|capitalize)\}|\$(\d+)/g, (_, g, op, plain) => {
      const v = m[+(g ?? plain)] ?? '';
      if (op === 'downcase') return v.toLowerCase();
      if (op === 'upcase') return v.toUpperCase();
      if (op === 'capitalize') return v.charAt(0).toUpperCase() + v.slice(1);
      return v;
    });
  }
  function render(list) {
    let out = '';
    for (const n of list) {
      if (n.t === 'text') out += n.v;
      else if (n.t === 'tab') out += values.get(n.n) ?? '';
      else if (n.t === 'ph') out += values.get(n.n) ?? render(n.children);
      else if (n.t === 'choice') out += values.get(n.n) ?? n.opts[0];
      else if (n.t === 'var') out += n.children ? render(n.children) : '';
      else if (n.t === 'transform') {
        const v = values.get(n.n) ?? '';
        const re = new RegExp(n.regex, n.flags);
        const m = re.exec(v);
        out += m ? v.slice(0, m.index) + applyFormat(n.format, m) + v.slice(m.index + m[0].length) : v;
      }
    }
    return out;
  }
  return render(nodes);
}

// ------------------------------------------------------------------ contexts

const STUBS = {
  CatsService: '#[injectable]\npub struct CatsService;',
  CatsController: '#[controller("cats")]\npub struct CatsController {}\n#[routes]\nimpl CatsController {}',
  SharedService: '#[injectable]\npub struct SharedService;',
  AppModule: '#[module]\npub struct AppModule;',
  LoggerMiddleware:
    '#[injectable]\npub struct LoggerMiddleware;\n#[async_trait]\nimpl CorvisMiddleware for LoggerMiddleware {\n    async fn use_(&self, req: Request, next: Next) -> HandlerResult {\n        next.run(req).await\n    }\n}',
  CreateCatDto: '#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]\npub struct CreateCatDto {\n    pub name: String,\n}',
  ListQuery: '#[derive(Debug, Clone, serde::Deserialize)]\npub struct ListQuery {\n    pub page: Option<u32>,\n    pub limit: Option<u32>,\n}',
  RolesGuard:
    'pub const ROLES_KEY: &str = "roles";\n#[injectable]\npub struct RolesGuard;\n#[async_trait]\nimpl CanActivate for RolesGuard {\n    async fn can_activate(&self, _c: &ExecutionContext) -> Result<bool, HttpException> {\n        Ok(true)\n    }\n}',
  create_cat_schema: 'pub fn create_cat_schema() -> z::ZObject {\n    z::object! { name: z::string() }\n}',
  CatDto:
    '#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]\npub struct CatDto {\n    pub id: u32,\n    pub name: String,\n}\ncorvis::zod_dto!(CatDto, z::object! { id: z::int(), name: z::string() });',
};

const CORVIS = 'use corvis::prelude::*;\n';
const SWAGGER = 'use corvis::prelude::*;\nuse corvis_swagger::*;\n';
const ORM_CLIENT = 'use corvis::prelude::*;\nuse crate::models::{PostDelegate, User, UserDelegate};\n';

const ctx = {
  header: (s) => s,
  item: (s) => CORVIS + s,
  routes: (s) => `${CORVIS}#[controller("x")]\npub struct XController {}\n\n#[routes]\nimpl XController {\n${indent(s)}\n}`,
  param: (s) => `${CORVIS}#[controller("x")]\npub struct XController {}\n\n#[routes]\nimpl XController {\n    #[post("p")]\n    async fn handler(&self, ${s}) -> HttpResult<Value> {\n        todo!()\n    }\n}`,
  field: (s) => `${CORVIS}#[injectable]\npub struct Holder {\n${indent(s)}\n}`,
  provider: (s) => `${CORVIS}#[module(providers = [\n${indent(s)}\n])]\npub struct ProvidersModule;`,
  expr: (s) => `${CORVIS}pub fn build() {\n    let _value = ${s.replace(/\n/g, '\n    ')};\n}`,
  main: (s) => `${CORVIS}pub async fn bootstrap() -> corvis::Result<()> {\n    let mut app = CorvisFactory::create::<AppModule>().await?;\n${indent(s)}\n    app.listen(3000).await\n}`,
  model: (s) => `use corvis_orm::prelude::*;\n${s}`,
  orm: (s) =>
    `${ORM_CLIENT}#[injectable]\npub struct Repo {\n    db: Arc<corvis_orm::OrmService>,\n}\n\nimpl Repo {\n    pub async fn run(&self, id: i64, email: String, name: String) -> Result<(), corvis_orm::OrmError> {\n        let _ = (&id, &email, &name);\n        let _result = ${s.replace(/\n/g, '\n        ')};\n        Ok(())\n    }\n}`,
  'orm-item': (s) => ORM_CLIENT + s,
  'swagger-item': (s) => SWAGGER + s,
  'swagger-routes': (s) => `${SWAGGER}#[controller("x")]\npub struct XController {}\n\n#[routes]\nimpl XController {\n${indent(s)}\n}`,
};

function indent(s) {
  return s
    .split('\n')
    .map((l) => (l ? '    ' + l : l))
    .join('\n');
}

/** snippet name -> [context, stubs] (default: item, no stubs). */
const MANIFEST = {
  'Corvis: prelude': ['header'],
  'Corvis: main (bootstrap)': ['item', ['AppModule']],
  'Corvis: module': ['item', ['CatsController', 'CatsService']],
  'Corvis: global module': ['item', ['SharedService']],
  'Corvis: CorvisModule middleware configuration': ['item', ['AppModule', 'LoggerMiddleware']],
  'Corvis: controller (CRUD)': ['item', ['CatsService']],
  'Corvis: GET route': ['routes'],
  'Corvis: GET :id route': ['routes'],
  'Corvis: POST route': ['routes'],
  'Corvis: PUT route': ['routes'],
  'Corvis: PATCH route': ['routes'],
  'Corvis: DELETE route': ['routes'],
  'Corvis: GET route with query DTO': ['routes', ['ListQuery']],
  'Corvis: guarded route with roles': ['routes', ['RolesGuard']],
  'Corvis: #[body] parameter': ['param', ['CreateCatDto']],
  'Corvis: #[param] parameter': ['param'],
  'Corvis: #[query] parameter': ['param', ['ListQuery']],
  'Corvis: #[headers] parameter': ['param'],
  'Corvis: #[body] with ZodValidationPipe': ['param', ['CreateCatDto', 'create_cat_schema']],
  'Corvis: injected dependency field': ['field', ['CatsService']],
  'Corvis: #[inject("TOKEN")] field': ['field'],
  'Corvis: Provider::use_value': ['provider'],
  'Corvis: Provider::use_factory': ['provider'],
  'Corvis: lifecycle hook (OnModuleInit)': ['item', ['CatsService']],
  'Corvis: z::object!': ['expr'],
  'Corvis: zod query schema in a route': ['routes', ['ListQuery']],
  'Corvis ORM: model': ['model'],
  'Corvis ORM: service with OrmService': ['orm-item'],
  'Corvis ORM: find_many': ['orm'],
  'Corvis ORM: find_unique': ['orm'],
  'Corvis ORM: create': ['orm'],
  'Corvis ORM: update': ['orm'],
  'Corvis ORM: upsert': ['orm'],
  'Corvis ORM: delete': ['orm'],
  'Corvis ORM: count': ['orm'],
  'Corvis ORM: interactive transaction': ['orm'],
  'Corvis Swagger: documented controller': ['swagger-item', ['CatDto']],
  'Corvis Swagger: documented route': ['swagger-routes', ['CatDto']],
  'Corvis Swagger: setup in main': ['main', ['AppModule']],
  'Corvis: test module': ['item', ['CatsController', 'CatsService']],
};

/** Choice placeholders: every option is checked, not only the first one. */
function variants(body) {
  const choices = [...body.matchAll(/\$\{(\d+)\|([^}]*)\|\}/g)];
  const out = [body];
  for (const c of choices) {
    const opts = c[2].split(',');
    for (const o of opts.slice(1)) out.push(body.replace(c[0], `\${${c[1]}:${o}}`));
  }
  return out;
}

const MODELS = `use corvis_orm::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
#[model(table = "users")]
pub struct User {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    #[unique]
    pub email: String,
    pub name: Option<String>,
    #[relation(has_many)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posts: Option<Vec<Post>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    #[id]
    #[default(autoincrement)]
    pub id: i64,
    pub title: String,
    pub author_id: i64,
    #[relation(belongs_to, fields = "author_id", on_delete = "Cascade")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Box<User>>,
}
`;

// ------------------------------------------------------------------ main

const snippets = JSON.parse(readFileSync(join(here, '../snippets/corvis.code-snippets'), 'utf8'));
const srcDir = join(crateDir, 'src');
if (!existsSync(join(crateDir, 'Cargo.toml'))) {
  mkdirSync(srcDir, { recursive: true });
  const dep = (c) => `${c} = { path = "${join(corvisRoot, 'crates', c).replace(/\\/g, '/')}" }`;
  writeFileSync(
    join(crateDir, 'Cargo.toml'),
    `[package]\nname = "corvis-vscode-snippets"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n\n[workspace]\n\n[dependencies]\n${dep('corvis')}\n${dep('corvis-orm')}\n${dep('corvis-swagger')}\nserde = { version = "1", features = ["derive"] }\nserde_json = { version = "1", features = ["preserve_order"] }\n`,
  );
}
const genDir = join(srcDir, 'snippets');
rmSync(genDir, { recursive: true, force: true });
mkdirSync(genDir, { recursive: true });

const mods = [];
let k = 0;
for (const [name, snippet] of Object.entries(snippets)) {
  const body = Array.isArray(snippet.body) ? snippet.body.join('\n') : snippet.body;
  const [context = 'item', stubs = []] = MANIFEST[name] ?? [];
  for (const variant of variants(body)) {
    const code = expand(variant);
    const file = `s${String(k++).padStart(2, '0')}`;
    const wrapped = ctx[context](code) + '\n' + stubs.map((s) => STUBS[s]).join('\n\n') + '\n';
    writeFileSync(join(genDir, `${file}.rs`), `// ${name}\n${wrapped}`);
    mods.push({ file, name });
  }
}
writeFileSync(join(genDir, 'mod.rs'), mods.map((m) => `mod ${m.file}; // ${m.name}`).join('\n') + '\n');
writeFileSync(join(srcDir, 'models.rs'), MODELS);
writeFileSync(join(srcDir, 'main.rs'), '#![allow(dead_code, unused)]\n\nmod models;\nmod snippets;\n\nfn main() {}\n');
console.log(`expanded ${Object.keys(snippets).length} snippets into ${mods.length} modules in ${genDir}`);

const unknown = Object.keys(MANIFEST).filter((n) => !snippets[n]);
if (unknown.length) {
  console.error('MANIFEST entries without a snippet:', unknown);
  process.exit(1);
}

if (!args.includes('--no-check')) {
  try {
    execFileSync('cargo', ['check', '--all-targets', '--message-format', 'short'], { cwd: crateDir, stdio: 'inherit' });
    console.log('OK: all snippets compile');
  } catch {
    console.error('FAILED: see the errors above (src/snippets/sNN.rs; the first line names the snippet)');
    process.exit(1);
  }
}
