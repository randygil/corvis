// Checks the packaging of the published crates (see openspec/specs/infra/packaging). No compilation needed.
//
// For every workspace package without `publish = false`:
// - files:    LICENSE and THIRD_PARTY_NOTICES.md exist in the crate folder and are byte-identical to the root files
// - metadata: description, license, repository, homepage, rust-version, readme, 1-5 keywords and at least one category
// - inherit:  license, authors, repository, homepage, edition and rust-version come from [workspace.package]
// - deps:     every dependency on another workspace crate has a version requirement (not `*`), and one on a
//             proc-macro crate is pinned with `=` to the version of both crates
// - shared:   a crates.io dependency that two or more published crates declare (in any dependency section) comes from
//             [workspace.dependencies] in every one of them (`name = { workspace = true }`, extra features allowed)
//
// Run from anywhere: `node scripts/check-crate-files.mjs`. Prints one line per problem and exits 1 if there is any.
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const SHARED_FILES = ['LICENSE', 'THIRD_PARTY_NOTICES.md']
const INHERITED = ['license', 'authors', 'repository', 'homepage', 'edition', 'rust-version']

const meta = JSON.parse(
  execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], {
    cwd: root,
    maxBuffer: 64 * 1024 * 1024,
    encoding: 'utf8',
  }),
)

const problems = []
const bad = (crate, msg) => problems.push(`${crate}: ${msg}`)

const workspaceIds = new Set(meta.workspace_members)
const members = meta.packages.filter((p) => workspaceIds.has(p.id))
const byName = new Map(members.map((p) => [p.name, p]))
// `publish = false` is reported as an empty list; no `publish` key means any registry.
const published = members.filter((p) => !(Array.isArray(p.publish) && p.publish.length === 0))

for (const pkg of published) {
  const dir = path.dirname(pkg.manifest_path)
  const name = pkg.name

  for (const file of SHARED_FILES) {
    const source = path.join(root, file)
    const copy = path.join(dir, file)
    if (!fs.existsSync(copy)) {
      bad(name, `${file} is missing (copy it from the repository root)`)
    } else if (!fs.readFileSync(copy).equals(fs.readFileSync(source))) {
      bad(name, `${file} differs from the root file (refresh it: copy ${file} to ${path.relative(root, copy)})`)
    }
  }

  for (const field of ['description', 'license', 'repository', 'homepage', 'rust_version', 'readme']) {
    if (!pkg[field]) bad(name, `missing \`${field.replace('_', '-')}\``)
  }
  if (pkg.readme && !fs.existsSync(path.resolve(dir, pkg.readme))) bad(name, `readme \`${pkg.readme}\` does not exist`)
  if (!pkg.authors || pkg.authors.length === 0) bad(name, 'missing `authors`')
  if (pkg.keywords.length < 1 || pkg.keywords.length > 5) bad(name, `\`keywords\` must have 1-5 entries, has ${pkg.keywords.length}`)
  if (pkg.categories.length < 1) bad(name, 'missing `categories`')

  const manifest = fs.readFileSync(pkg.manifest_path, 'utf8')
  const pkgSection = manifest.split(/^\[(?!package\])/m)[0]
  for (const field of INHERITED) {
    if (!new RegExp(`^${field}\\.workspace\\s*=\\s*true\\s*$`, 'm').test(pkgSection)) {
      bad(name, `\`${field}\` must be inherited (\`${field}.workspace = true\`)`)
    }
  }

  for (const dep of pkg.dependencies) {
    const target = byName.get(dep.name)
    if (!target || dep.kind === 'dev') continue
    if (dep.req === '*') {
      bad(name, `dependency \`${dep.name}\` has no version (add it to [workspace.dependencies])`)
      continue
    }
    if (target.targets.some((t) => t.kind.includes('proc-macro'))) {
      if (dep.req !== `=${target.version}` || pkg.version !== target.version) {
        bad(name, `proc-macro dependency \`${dep.name}\` must be pinned \`=${target.version}\` (found \`${dep.req}\`) and share the version of ${name}`)
      }
    }
  }
  for (const dep of pkg.dependencies) {
    // A dev-dependency on a workspace crate also needs a version to package.
    const target = byName.get(dep.name)
    if (target && dep.kind === 'dev' && dep.req === '*') bad(name, `dev-dependency \`${dep.name}\` has no version`)
  }
}

// Shared dependencies. Cargo metadata resolves `workspace = true`, so the manifests are read as text: one entry per
// `name = ...` line of a dependency section, or per `[dependencies.name]` table.
const declarations = new Map() // dependency -> [{ crate, inherited }]
for (const pkg of published) {
  for (const { name, inherited } of dependencyDeclarations(fs.readFileSync(pkg.manifest_path, 'utf8'))) {
    if (byName.has(name)) continue // workspace crates are checked above
    if (!declarations.has(name)) declarations.set(name, [])
    declarations.get(name).push({ crate: pkg.name, inherited })
  }
}
for (const [dep, decls] of [...declarations].sort(([a], [b]) => a.localeCompare(b))) {
  const crates = [...new Set(decls.map((d) => d.crate))]
  if (crates.length < 2) continue
  const own = [...new Set(decls.filter((d) => !d.inherited).map((d) => d.crate))]
  if (own.length) {
    bad(own.join(', '), `dependency \`${dep}\` is declared by ${crates.join(', ')}; declare it once in [workspace.dependencies] and use \`${dep} = { workspace = true }\``)
  }
}

/** The dependencies a manifest declares, each with whether it comes from [workspace.dependencies]. */
function dependencyDeclarations(manifest) {
  const out = []
  let section = ''
  let table = null // the entry of a `[dependencies.name]` table
  for (const line of manifest.split(/\r?\n/)) {
    const header = /^\s*\[([^\]]+)\]\s*(#.*)?$/.exec(line)
    if (header) {
      section = header[1].trim()
      table = null
      const named = /^(?:target\..+\.)?(?:dev-|build-)?dependencies\.([A-Za-z0-9_-]+)$/.exec(section)
      if (named) {
        table = { name: named[1], inherited: false }
        out.push(table)
      }
      continue
    }
    if (table) {
      if (/^\s*workspace\s*=\s*true\b/.test(line)) table.inherited = true
      continue
    }
    if (!/^(?:target\..+\.)?(?:dev-|build-)?dependencies$/.test(section)) continue
    const entry = /^\s*([A-Za-z0-9_-]+)(\.workspace)?\s*=\s*(.*)$/.exec(line)
    if (entry) out.push({ name: entry[1], inherited: Boolean(entry[2]) || /\bworkspace\s*=\s*true\b/.test(entry[3]) })
  }
  return out
}

if (problems.length) {
  console.error(problems.join('\n'))
  console.error(`\ncrate packaging check failed: ${problems.length} problem(s)`)
  process.exit(1)
}
console.log(`crate packaging check passed (${published.length} published crates: ${published.map((p) => p.name).join(', ')})`)
