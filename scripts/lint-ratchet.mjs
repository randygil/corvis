// Lint ratchet: the findings of the lints below may only go down (see openspec/specs/infra/ci, "Lint findings only go
// down"). The lints with no finding at all live in `[workspace.lints]` of the root Cargo.toml instead, where every CI
// job denies them; the ones here still have findings, so they are counted rather than denied.
//
//   node scripts/lint-ratchet.mjs                     count and compare with lint-baseline.json
//   node scripts/lint-ratchet.mjs --update            rewrite lint-baseline.json with the current counts (refuses to
//                                                     raise a count unless --allow-increase is given too)
//   node scripts/lint-ratchet.mjs --self-test         check the counting and the comparison on built-in samples
//
// What it counts, per published crate (the workspace members without `publish = false`), per lint and per file:
// - The RATCHET lints, from `cargo clippy --all-features --message-format=json` on the published crates, with those
//   lints raised to `warn` on the command line (no `-D warnings`). Library code is what `--lib --bins` compiles; everything that only `--all-targets` adds
//   (`#[cfg(test)]` modules, integration tests, benches, examples of the crate) is test code. Test code is counted
//   only for the lints marked `tests: true`; `unwrap_used`, `expect_used` and `panic` are also allowed in tests by
//   clippy.toml. `print_stdout` and `print_stderr` are counted only in crates with a library (not in `corvis-cli`).
// - Doc examples fenced as ```ignore (or ```rust,ignore) under `src/`. A fence that states why it cannot run is not
//   counted: ```ignore (needs a database). Prefer `no_run`, which still compiles the example.
//
// The comparison is per crate and lint (the sum over files). A count above the baseline fails and lists the findings
// of the files whose count went up. A count below the baseline fails too, until the baseline is lowered in the same
// commit with `--update`, so the baseline only moves down. A lint at zero in every crate is reported as ready to move
// to `[workspace.lints]` (a reminder, not a failure).
//
// Platform: CI runs it on Linux, and the baseline holds the Linux counts. Code under `cfg(windows)` or `cfg(target_os
// = "macos")` is not compiled there and not counted; on another platform the counts differ (the script says so). On
// Windows, run it in WSL or a Linux container to reproduce CI.
//
// A toolchain bump can rename or add lints: an unknown lint name fails the run, and the bump pull request updates the
// list below and the baseline (`--update --allow-increase`, with the reason in the pull request).
import { execFileSync, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const BASELINE = path.join(root, 'lint-baseline.json')

/**
 * The ratchet lints. `tests`: also counted in test code. `libraryOnly`: counted only in crates with a library target.
 * When a lint reaches zero in every crate, move it to `[workspace.lints]` and remove it here.
 */
const RATCHET = [
  { lint: 'missing_docs' },
  { lint: 'unreachable_pub' },
  { lint: 'missing_debug_implementations' },
  { lint: 'clippy::unwrap_used' },
  { lint: 'clippy::expect_used' },
  { lint: 'clippy::panic' },
  { lint: 'clippy::undocumented_unsafe_blocks', tests: true },
  { lint: 'clippy::missing_panics_doc' },
  { lint: 'clippy::missing_errors_doc' },
  { lint: 'clippy::print_stdout', libraryOnly: true },
  { lint: 'clippy::print_stderr', libraryOnly: true },
]
const FENCE = 'rustdoc: ```ignore without a reason'
const TEST_SUFFIX = ' (tests)'

// ---------- counting ----------

const slash = (p) => p.replaceAll('\\', '/')

/** Every diagnostic of a ratchet lint in a cargo JSON stream, as `{ crate, lint, file, line, column }`. */
function diagnostics(jsonLines, crateOf) {
  const known = new Set(RATCHET.map((r) => r.lint))
  const out = []
  const unknown = []
  for (const line of jsonLines) {
    if (!line.startsWith('{')) continue
    const m = JSON.parse(line)
    if (m.reason !== 'compiler-message') continue
    const code = m.message.code?.code
    if (code === 'unknown_lints' || code === 'clippy::unknown_clippy_lints') {
      unknown.push(m.message.message)
      continue
    }
    if (!known.has(code)) continue
    const crate = crateOf(m.package_id)
    if (!crate) continue
    const span = m.message.spans.find((s) => s.is_primary) ?? m.message.spans[0]
    if (!span) continue
    out.push({ crate, lint: code, file: slash(span.file_name), line: span.line_start, column: span.column_start })
  }
  return { out, unknown }
}

const keyOf = (d) => `${d.file}:${d.line}:${d.column}:${d.lint}`

/**
 * Library findings come from the `--lib --bins` run; the `--all-targets` run adds the test code. Returns the counted
 * findings, each with `group` (the lint, plus TEST_SUFFIX for test code).
 */
function classify(library, all, crates) {
  const libraryKeys = new Set(library.map(keyOf))
  const rules = new Map(RATCHET.map((r) => [r.lint, r]))
  const seen = new Set()
  const counted = []
  for (const d of [...library, ...all]) {
    const key = keyOf(d)
    if (seen.has(key)) continue // the lib and the lib-test builds report library code twice
    seen.add(key)
    const rule = rules.get(d.lint)
    const test = !libraryKeys.has(key)
    if (test && !rule.tests) continue
    if (rule.libraryOnly && !crates.get(d.crate)?.hasLibrary) continue
    counted.push({ ...d, group: test ? d.lint + TEST_SUFFIX : d.lint })
  }
  return counted
}

/** The ```ignore fences without a reason in one source file, as line numbers. */
function ignoreFences(content) {
  const lines = []
  content.split('\n').forEach((text, i) => {
    const m = /^\s*\/\/[/!]\s*(?:`{3,}|~{3,})\s*([^`~]*?)\s*$/.exec(text)
    if (!m || !m[1]) return
    const info = m[1]
    const reason = info.indexOf('(')
    const attrs = (reason === -1 ? info : info.slice(0, reason)).split(/[\s,]+/).filter(Boolean)
    if (attrs.includes('ignore') && reason === -1) lines.push(i + 1)
  })
  return lines
}

/** `{ crate: { group: { file: count } } }` with sorted keys. */
function tally(findings) {
  const counts = {}
  for (const f of findings) {
    const groups = (counts[f.crate] ??= {})
    const files = (groups[f.group] ??= {})
    files[f.file] = (files[f.file] ?? 0) + 1
  }
  return sortDeep(counts)
}

function sortDeep(v) {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return v
  return Object.fromEntries(Object.keys(v).sort().map((k) => [k, sortDeep(v[k])]))
}

const sum = (files) => Object.values(files ?? {}).reduce((a, b) => a + b, 0)

// ---------- comparison ----------

/**
 * Compares two tallies. `above` and `below` hold `{ crate, group, was, now, files }` (`files`: the files whose count
 * went up); `ready` holds the lints at zero in every crate.
 */
function compare(baseline, current) {
  const above = []
  const below = []
  const crates = new Set([...Object.keys(baseline), ...Object.keys(current)])
  for (const crate of [...crates].sort()) {
    const b = baseline[crate] ?? {}
    const c = current[crate] ?? {}
    for (const group of [...new Set([...Object.keys(b), ...Object.keys(c)])].sort()) {
      const was = sum(b[group])
      const now = sum(c[group])
      if (now > was) {
        const files = Object.keys(c[group]).filter((f) => c[group][f] > (b[group]?.[f] ?? 0))
        above.push({ crate, group, was, now, files })
      } else if (now < was) {
        below.push({ crate, group, was, now, files: [] })
      }
    }
  }
  const ready = RATCHET.map((r) => r.lint).filter((lint) =>
    Object.values(current).every((groups) => !sum(groups[lint]) && !sum(groups[lint + TEST_SUFFIX])),
  )
  return { above, below, ready }
}

// ---------- running cargo ----------

function workspace() {
  const meta = JSON.parse(
    execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], { cwd: root, maxBuffer: 64 << 20, encoding: 'utf8' }),
  )
  const ids = new Set(meta.workspace_members)
  const crates = new Map()
  const byId = new Map()
  for (const p of meta.packages) {
    if (!ids.has(p.id) || (Array.isArray(p.publish) && p.publish.length === 0)) continue
    const hasLibrary = p.targets.some((t) => t.kind.some((k) => k === 'lib' || k === 'proc-macro' || k.endsWith('lib')))
    crates.set(p.name, { dir: path.dirname(p.manifest_path), hasLibrary })
    byId.set(p.id, p.name)
  }
  return { crates, crateOf: (id) => byId.get(id) }
}

function clippy(crates, targets) {
  const args = ['clippy', ...[...crates.keys()].flatMap((c) => ['-p', c]), '--all-features', ...targets, '--message-format=json', '--']
  for (const r of RATCHET) args.push('-W', r.lint)
  const run = spawnSync('cargo', args, { cwd: root, maxBuffer: 1 << 30, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] })
  if (run.error) throw run.error
  if (run.status !== 0) {
    console.error(`\nlint ratchet: \`cargo ${args.join(' ')}\` failed (status ${run.status}); fix the build first`)
    process.exit(1)
  }
  return run.stdout.split('\n')
}

function sourceFiles(dir) {
  const out = []
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) out.push(...sourceFiles(p))
    else if (e.name.endsWith('.rs')) out.push(p)
  }
  return out
}

function measure() {
  const { crates, crateOf } = workspace()
  const library = diagnostics(clippy(crates, ['--lib', '--bins']), crateOf)
  const all = diagnostics(clippy(crates, ['--all-targets']), crateOf)
  const unknown = [...new Set([...library.unknown, ...all.unknown])]
  if (unknown.length) {
    for (const u of unknown) console.error(`lint ratchet: ${u}`)
    console.error('\nlint ratchet: a lint of RATCHET is unknown to this toolchain; rename or remove it in scripts/lint-ratchet.mjs')
    process.exit(1)
  }
  const findings = classify(library.out, all.out, crates)
  for (const [crate, { dir }] of crates) {
    const src = path.join(dir, 'src')
    if (!fs.existsSync(src)) continue
    for (const file of sourceFiles(src)) {
      const rel = slash(path.relative(root, file))
      for (const line of ignoreFences(fs.readFileSync(file, 'utf8'))) findings.push({ crate, group: FENCE, file: rel, line, column: 1 })
    }
  }
  return findings
}

// ---------- self-test ----------

function selfTest() {
  const results = []
  const expect = (label, ok) => results.push(`${ok ? 'ok  ' : 'FAIL'} ${label}`)
  const crates = new Map([
    ['lib-crate', { hasLibrary: true }],
    ['bin-crate', { hasLibrary: false }],
  ])
  const msg = (pkg, code, file, line) =>
    JSON.stringify({
      reason: 'compiler-message',
      package_id: pkg,
      message: { code: { code }, message: 'x', spans: [{ is_primary: true, file_name: file, line_start: line, column_start: 5 }] },
    })
  const crateOf = (id) => ({ L: 'lib-crate', B: 'bin-crate' })[id]
  const lib = diagnostics(
    [
      msg('L', 'clippy::unwrap_used', 'crates\\lib-crate\\src\\a.rs', 10),
      msg('L', 'missing_docs', 'crates/lib-crate/src/a.rs', 1),
      msg('B', 'clippy::print_stdout', 'crates/bin-crate/src/main.rs', 3),
      msg('L', 'clippy::print_stdout', 'crates/lib-crate/src/a.rs', 20),
      msg('X', 'clippy::unwrap_used', 'examples/x/src/main.rs', 1),
      msg('L', 'dead_code', 'crates/lib-crate/src/a.rs', 30),
      'Compiling lib-crate',
    ],
    crateOf,
  ).out
  const all = diagnostics(
    [
      msg('L', 'clippy::unwrap_used', 'crates/lib-crate/src/a.rs', 10),
      msg('L', 'clippy::unwrap_used', 'crates/lib-crate/tests/t.rs', 4),
      msg('L', 'clippy::undocumented_unsafe_blocks', 'crates/lib-crate/tests/t.rs', 8),
    ],
    crateOf,
  ).out
  const counted = classify(lib, all, crates)
  const groups = counted.map((f) => `${f.crate} ${f.group} ${f.file}:${f.line}`)
  expect('a library finding is counted once (lib and lib-test builds)', groups.filter((g) => g.includes('unwrap_used') && g.includes('a.rs:10')).length === 1)
  expect('Windows paths are normalized', groups.includes('lib-crate clippy::unwrap_used crates/lib-crate/src/a.rs:10'))
  expect('an unwrap in test code is not counted', !groups.some((g) => g.includes('tests/t.rs:4')))
  expect('a lint counted in tests is counted there, apart', groups.includes('lib-crate clippy::undocumented_unsafe_blocks (tests) crates/lib-crate/tests/t.rs:8'))
  expect('print_stdout is not counted in a binary crate', !groups.some((g) => g.startsWith('bin-crate')))
  expect('print_stdout is counted in a library crate', groups.some((g) => g.includes('lib-crate clippy::print_stdout')))
  expect('other packages and other lints are ignored', counted.length === 4)

  const unknown = diagnostics([JSON.stringify({ reason: 'compiler-message', package_id: 'L', message: { code: { code: 'unknown_lints' }, message: 'unknown lint: `clippy::gone`', spans: [] } })], crateOf)
  expect('an unknown lint is reported', unknown.unknown.length === 1)

  expect('a plain ignore fence is counted', ignoreFences('/// ```ignore\n/// x\n/// ```').length === 1)
  expect('rust,ignore is counted', ignoreFences('//! ```rust,ignore\n//! ```').length === 1)
  expect('an ignore fence with a reason is not counted', ignoreFences('/// ```ignore (needs a database)\n/// ```').length === 0)
  expect('no_run and plain fences are not counted', ignoreFences('/// ```no_run\n/// ```\n/// ```\n/// ```text').length === 0)
  expect('the fence line number is reported', ignoreFences('//! a\n//! ```ignore')[0] === 2)
  expect('a fence outside a doc comment is not counted', ignoreFences('let s = "\n```ignore\n";').length === 0)

  const base = tally(counted)
  const same = compare(base, tally(counted))
  expect('equal counts pass', same.above.length === 0 && same.below.length === 0)
  const more = compare(base, tally([...counted, { crate: 'lib-crate', group: 'clippy::unwrap_used', file: 'crates/lib-crate/src/b.rs', line: 1 }]))
  expect('a count above the baseline fails and names the file', more.above.length === 1 && more.above[0].files.join() === 'crates/lib-crate/src/b.rs' && more.above[0].now === 2)
  const fewer = compare(base, tally(counted.filter((f) => f.group !== 'missing_docs')))
  expect('a count below the baseline fails', fewer.below.length === 1 && fewer.below[0].group === 'missing_docs')
  const moved = compare(
    { c: { 'clippy::panic': { 'a.rs': 1 } } },
    { c: { 'clippy::panic': { 'b.rs': 1 } } },
  )
  expect('a finding moved to another file of the crate passes', moved.above.length === 0 && moved.below.length === 0)
  expect('a lint at zero everywhere is ready to move', compare({}, base).ready.includes('clippy::expect_used') && !compare({}, base).ready.includes('missing_docs'))

  for (const r of results) console.log(r)
  const failed = results.filter((r) => r.startsWith('FAIL')).length
  console.log(failed ? `\n${failed} self-test(s) failed` : '\nself-test passed')
  process.exit(failed ? 1 : 0)
}

// ---------- main ----------

if (process.argv.includes('--self-test')) selfTest()

const update = process.argv.includes('--update')
const allowIncrease = process.argv.includes('--allow-increase')
const exists = fs.existsSync(BASELINE)
const baseline = exists ? JSON.parse(fs.readFileSync(BASELINE, 'utf8')).crates ?? {} : {}
const findings = measure()
const current = tally(findings)
const { above, below, ready } = compare(baseline, current)
const total = findings.length
if (process.platform !== 'linux') {
  console.log(`note: the baseline holds the Linux counts; on ${process.platform}, platform-specific code makes them differ`)
}

const describe = (d) => `${d.crate}: ${d.group}: ${d.was} -> ${d.now}`

if (update) {
  if (above.length && exists && !allowIncrease) {
    for (const d of above) console.error(`raises ${describe(d)}`)
    console.error('\nlint ratchet: --update does not raise a count; fix the findings, or pass --allow-increase and say why in the pull request')
    process.exit(1)
  }
  const doc = {
    about: 'Counts of scripts/lint-ratchet.mjs (Linux): per crate, lint and file. Lower them with `node scripts/lint-ratchet.mjs --update`; never raise them by hand.',
    crates: current,
  }
  fs.writeFileSync(BASELINE, JSON.stringify(doc, null, 2) + '\n')
  console.log(`lint-baseline.json written: ${total} finding(s); ${below.length} count(s) lowered, ${above.length} raised`)
  process.exit(0)
}

for (const d of above) {
  console.log(`${describe(d)} (above the baseline)`)
  const files = new Set(d.files)
  for (const f of findings) if (f.crate === d.crate && f.group === d.group && files.has(f.file)) console.log(`  ${f.file}:${f.line}:${f.column}: ${f.group}`)
}
for (const d of below) console.log(`${describe(d)} (below the baseline)`)
for (const lint of ready) console.log(`${lint} has no finding left: move it to [workspace.lints] in Cargo.toml and remove it from scripts/lint-ratchet.mjs`)

if (above.length || below.length) {
  if (above.length) console.log(`\nlint ratchet failed: ${above.length} count(s) above the baseline; fix the findings listed above`)
  if (below.length) console.log(`\nlint ratchet failed: ${below.length} count(s) below the baseline; lower the baseline: \`node scripts/lint-ratchet.mjs --update\``)
  process.exit(1)
}
console.log(`lint ratchet passed (${total} finding(s) in ${Object.keys(current).length} crates, as in lint-baseline.json)`)
