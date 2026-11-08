// Checks the public naming rule on the published surfaces of the repository: the words "Nest", "NestJS" and
// "Prisma", identifiers built on them and the original packages' names and links do not appear, apart from the
// nominative uses in ALLOW below. The documentation site has its own check as well (docs/scripts/check.mjs).
//
//   node scripts/check-naming.mjs              scan the tree (tracked files plus untracked, non-ignored ones)
//   node scripts/check-naming.mjs --self-test  check the matcher itself on built-in samples
//
// Prints every violation as `path:line` and exits with status 1 when there is at least one, when an allowlist entry
// has no reason, or when an entry no longer allows anything (remove it).
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

/** Published surfaces (globs relative to the repository root). */
const SURFACES = [
  'Cargo.toml',
  'README.md',
  'crates/**',
  'examples/**',
  'docs/**',
  'bench/README.md',
  'bench/report.js',
  'bench/results.md',
  'bench/run.sh',
  'bench/corvis-app/**',
]

/** Banned patterns. Case-sensitive, so "nested", "nesting" and "schema.prisma" do not match. */
const PATTERNS = [
  { name: 'brand', re: /\b(?:Nest|NestJS)\b|Nest[A-Z]\w*|\bPrisma\w*/g },
  { name: 'package', re: /@nestjs\/|\bnestjs\b|\bnestjs-|\bnest-[a-z]\w*|\bNEST_[A-Z]\w*/g },
  // A lowercase `prisma` that is not a file extension (`schema.prisma`, `*.prisma`): folders, commands, the client.
  { name: 'identifier', re: /(?<![\w.])prisma\w*|_prisma_migrations/g },
  { name: 'link', re: /pris\.ly|prisma\.io|nestjs\.com/g },
  { name: 'tagline', re: /efficient and scalable server-side/g },
]

/**
 * Nominative uses. Each entry: `path` (glob), optional `pattern` (only the text it matches is allowed; without it the
 * whole file is) and `reason`. Every entry needs a reason; keep the list short.
 */
const COMPAT = 'compatibility identifier of existing projects'
const ALLOW = [
  // Compatibility identifiers (corvis-orm reads and writes what the original tooling does).
  { path: '**', pattern: /_prisma_migrations/, reason: `${COMPAT}: migration control table` },
  { path: 'crates/corvis-orm/src/bin/corvis-orm.rs', pattern: /prisma\/(?:schema|migrations|\.env)/, reason: `${COMPAT}: default schema, migrations and .env paths` },
  { path: 'crates/corvis-orm/src/migrate.rs', pattern: /prisma\/migrations/, reason: `${COMPAT}: migrations folder layout` },
  { path: 'crates/corvis-orm/src/psl.rs', pattern: /e == "prisma"|prismaSchemaFolder|"prisma-client-js"/, reason: 'schema file format: the `.prisma` extension, a preview feature and the generator provider' },

  // Error texts copied verbatim for parity (design Decision 6).
  { path: 'crates/corvis-orm/src/error.rs', pattern: /http:\/\/pris\.ly\/d\/connection-pool/, reason: 'P2024 message copied verbatim for parity' },
  { path: 'crates/corvis-orm/src/migrate.rs', pattern: /https:\/\/pris\.ly\/d\/migrate-resolve/, reason: 'P3009 and P3018 messages copied verbatim for parity' },

  // The legally required notices of the replicated projects; the root file is not a scanned surface.
  { path: 'crates/*/THIRD_PARTY_NOTICES.md', reason: 'copy of the root third-party notices: the names of the projects whose licenses must travel with the crate' },

  // Tests and fixtures that run against or record the output of an original Node library.
  { path: 'crates/corvis/tests/fixtures/**', reason: 'oracle scripts and recorded output of the original Node libraries' },
  { path: 'crates/corvis-orm/tests/fixtures/**', reason: 'oracle schema and recorded output of the original client' },
  { path: 'crates/corvis-orm/tests/pg_oracle.rs', reason: 'maps errors to the original client error class names of the recorded oracle' },
  { path: 'crates/corvis-orm/tests/generated/*.rs', reason: 'generated from the reference apps\' schemas; header paths and copied schema comments' },
  { path: 'crates/corvis-orm/tests/pg_houseplan.rs', pattern: /houseplan\/api\/prisma\/migrations/, reason: 'migrations folder of the reference app the test runs' },

  // Out of scope here, with their own tasks.
  { path: 'crates/corvis-orm/README.md', pattern: /prisma\/(?:migrations|schema\.prisma)/, reason: `${COMPAT}: folder layout in the corvis-orm README` },

  // Files that state or enforce the rule.
  { path: 'docs/README.md', pattern: /"Nest", "NestJS" or "Prisma"/, reason: 'states the naming rule' },
  { path: 'docs/scripts/check.mjs', pattern: /"Nest", "NestJS" or "Prisma"|\(Nest\|NestJS\|Prisma\)/, reason: 'implements the naming rule on the docs site' },
]

// ---------- matcher ----------

function globToRegExp(glob) {
  let re = ''
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i]
    if (c === '*' && glob[i + 1] === '*') {
      re += '.*'
      i++
      if (glob[i + 1] === '/') i++
    } else if (c === '*') re += '[^/]*'
    else if (c === '?') re += '[^/]'
    else re += c.replace(/[.+^${}()|[\]\\]/g, '\\$&')
  }
  return new RegExp(`^${re}$`)
}

function prepare(allow) {
  return allow.map((a, i) => ({
    ...a,
    index: i,
    pathRe: globToRegExp(a.path),
    patternRe: a.pattern ? new RegExp(a.pattern.source, a.pattern.flags.includes('g') ? a.pattern.flags : a.pattern.flags + 'g') : null,
    used: false,
  }))
}

/** Problems with the allowlist itself. */
function validate(allow) {
  return allow
    .filter((a) => typeof a.reason !== 'string' || a.reason.trim() === '' || !a.path)
    .map((a) => `allowlist entry ${JSON.stringify(a.path ?? null)}${a.pattern ? ' ' + a.pattern : ''}: needs a path and a non-empty reason`)
}

/**
 * Returns the violations of one file: `{ line, text, match, kind }`. Entries with a pattern blank the text they match;
 * an entry without one allows the rest of every line. Marks the entries that allowed a banned word.
 */
function scan(file, content, entries) {
  const applicable = entries.filter((a) => a.pathRe.test(file))
  const partial = applicable.filter((a) => a.patternRe)
  const whole = applicable.find((a) => !a.patternRe)
  const banned = (s) => PATTERNS.some((p) => new RegExp(p.re.source).test(s))
  const out = []
  content.split(/\r?\n/).forEach((text, i) => {
    let rest = text
    for (const a of partial) {
      rest = rest.replace(a.patternRe, (m) => {
        if (banned(m)) a.used = true
        return ' '.repeat(m.length)
      })
    }
    if (whole && banned(rest)) {
      whole.used = true
      return
    }
    for (const p of PATTERNS) {
      for (const m of rest.matchAll(new RegExp(p.re.source, 'g'))) out.push({ line: i + 1, text, match: m[0], kind: p.name })
    }
  })
  return out
}

// ---------- self-test ----------

function selfTest() {
  const results = []
  const expect = (label, ok) => results.push(`${ok ? 'ok  ' : 'FAIL'} ${label}`)
  const run = (file, content, allow = ALLOW) => scan(file, content, prepare(allow))

  expect('a comment citing the original package is reported', run('crates/corvis/src/jwt.rs', '/// As in `@nestjs/jwt`, a secret wins.').length === 1)
  expect('the framework name is reported', run('crates/corvis/src/lib.rs', '// SSE framing like Nest 11').length === 1)
  expect('an identifier built on the name is reported', run('examples/x/src/main.rs', 'let app = NestFactory::create();').length === 1)
  expect('the client name is reported', run('crates/corvis-orm/src/lib.rs', '// Same as PrismaClient.').length === 1)
  expect('a lowercase folder is reported outside the allowlist', run('crates/corvis/src/lib.rs', '// see prisma/migrations').length === 1)
  expect('a vendor link is reported', run('crates/corvis/src/lib.rs', '// https://pris.ly/d/x').length === 1)
  expect('the copied tagline is reported', run('crates/corvis/Cargo.toml', 'description = "for building efficient and scalable server-side apps"').length === 1)
  expect('"nested" and "nesting" pass', run('crates/corvis/src/lib.rs', '// nested routes, nesting depth, the soonest').length === 0)
  expect('schema.prisma passes', run('crates/corvis-orm/src/lib.rs', '//! The `schema.prisma` equivalent; reads *.prisma files').length === 0)
  expect('_prisma_migrations passes', run('crates/corvis-orm/src/client.rs', '// writes to "_prisma_migrations"').length === 0)
  expect('an allowlisted fixture passes', run('crates/corvis-orm/tests/pg_oracle.rs', '"PrismaClientKnownRequestError",').length === 0)
  expect('an allowlist pattern only hides what it matches',
    run('crates/corvis-orm/src/migrate.rs', '// https://pris.ly/d/migrate-resolve, as in Nest').length === 1)
  expect('an entry with an empty reason fails', validate([{ path: 'x', reason: '' }]).length === 1)
  expect('an entry with a reason passes', validate([{ path: 'x', reason: 'why' }]).length === 0)

  for (const r of results) console.log(r)
  const failed = results.filter((r) => r.startsWith('FAIL')).length
  console.log(failed ? `\n${failed} self-test(s) failed` : '\nself-test passed')
  process.exit(failed ? 1 : 0)
}

// ---------- main ----------

if (process.argv.includes('--self-test')) selfTest()

const problems = validate(ALLOW)
const surfaces = SURFACES.map(globToRegExp)
const files = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'], { cwd: root, encoding: 'utf8', maxBuffer: 64 << 20 })
  .split('\0')
  .filter((f) => f && surfaces.some((re) => re.test(f)))
const entries = prepare(ALLOW)
const violations = []

for (const file of [...new Set(files)].sort()) {
  const abs = path.join(root, file)
  if (!fs.existsSync(abs) || !fs.statSync(abs).isFile()) continue
  const buf = fs.readFileSync(abs)
  if (buf.subarray(0, 8000).includes(0)) continue // binary
  for (const v of scan(file, buf.toString('utf8'), entries)) violations.push(`${file}:${v.line}: [${v.kind}] ${v.match}: ${v.text.trim().slice(0, 120)}`)
}

for (const a of entries) {
  if (!a.used) problems.push(`allowlist entry ${a.path}${a.pattern ? ' ' + a.pattern : ''} allows nothing; remove it`)
}

for (const v of violations) console.log(v)
for (const p of problems) console.log(p)
if (violations.length || problems.length) {
  console.log(`\nnaming check failed: ${violations.length} violation(s), ${problems.length} allowlist problem(s)`)
  process.exit(1)
}
console.log(`naming check passed (${files.length} files)`)
