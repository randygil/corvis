// Checks the bilingual documentation site. Run after `vitepress build` (see `npm run docs:check`).
//
// - pages:     every page exists in English (root) and Spanish (es/), with the same `status`
// - structure: each pair has the same headings per level and the same number of code fences
// - sidebar:   both locales list the same pages, and every listed page exists
// - links:     every internal link and anchor in the built HTML resolves, and stays in the page's language
// - language:  no Spanish left in English pages, no voseo in Spanish pages
// - naming:    the public naming rule (no "Nest", "NestJS" or "Prisma")
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const docs = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const dist = path.join(docs, '.vitepress', 'dist')
const SKIP_DIRS = new Set(['node_modules', '.vitepress', 'scripts', 'public', 'es'])

/**
 * Known exceptions to the language checks. Each entry: page (relative to docs/), the exact text allowed, and why.
 * Keep this list short; every entry needs a reason.
 */
const ALLOW = [
  // { page: 'faq/examples.md', text: 'Tertulia', reason: 'name of the example app' },
]

const failures = []
const fail = (check, where, msg) => failures.push(`[${check}] ${where}: ${msg}`)

// ---------- sources ----------

function walk(dir, skip) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    if (e.isDirectory()) return skip.has(e.name) ? [] : walk(path.join(dir, e.name), new Set())
    return e.name.endsWith('.md') && e.name !== 'README.md' ? [path.join(dir, e.name)] : []
  })
}

const rel = (base, f) => path.relative(base, f).split(path.sep).join('/')
const enPages = new Map(walk(docs, SKIP_DIRS).map((f) => [rel(docs, f), f]))
const esPages = new Map(walk(path.join(docs, 'es'), new Set()).map((f) => [rel(path.join(docs, 'es'), f), f]))
const read = (f) => fs.readFileSync(f, 'utf8').replace(/\r\n/g, '\n')

const status = (src) => /^---\n[\s\S]*?^status:\s*(\S+)/m.exec(src)?.[1] ?? 'documented'

/** Splits a page into prose lines and code lines (fenced blocks), with line numbers. */
function lines(src) {
  const out = []
  let fence = null
  src.split('\n').forEach((text, i) => {
    const m = /^\s*(`{3,}|~{3,})/.exec(text)
    if (m) {
      if (!fence) fence = m[1][0]
      else if (m[1][0] === fence) fence = null
      out.push({ n: i + 1, text, kind: 'fence' })
      return
    }
    out.push({ n: i + 1, text, kind: fence ? 'code' : 'prose' })
  })
  return out
}

function shape(src) {
  const ls = lines(src.replace(/^---\n[\s\S]*?\n---\n/, ''))
  const headings = [0, 0, 0, 0, 0, 0]
  for (const l of ls) {
    const m = l.kind === 'prose' && /^(#{1,6})\s/.exec(l.text)
    if (m) headings[m[1].length - 1]++
  }
  return { headings: headings.join(','), fences: ls.filter((l) => l.kind === 'fence').length }
}

// pages + structure
for (const p of enPages.keys()) if (!esPages.has(p)) fail('pages', p, 'missing Spanish page es/' + p)
for (const p of esPages.keys()) if (!enPages.has(p)) fail('pages', 'es/' + p, 'missing English page ' + p)
for (const [p, f] of enPages) {
  if (!esPages.has(p)) continue
  const en = read(f)
  const es = read(esPages.get(p))
  if (status(en) !== status(es)) fail('pages', p, `status "${status(en)}" differs from Spanish "${status(es)}"`)
  const a = shape(en)
  const b = shape(es)
  if (a.headings !== b.headings) fail('structure', p, `headings per level ${a.headings} vs Spanish ${b.headings}`)
  if (a.fences !== b.fences) fail('structure', p, `${a.fences} code fences vs Spanish ${b.fences}`)
}

// sidebar
const { sidebar, allLinks } = await import('../.vitepress/sidebar.mts')
const flat = (items) => items.flatMap((i) => [...(i.link ? [i.link] : []), ...(i.items ? flat(i.items) : [])])
const enSide = flat(sidebar('en'))
const esSide = flat(sidebar('es'))
if (enSide.length !== esSide.length) fail('sidebar', 'sidebar.mts', `${enSide.length} English entries vs ${esSide.length} Spanish`)
esSide.forEach((l, i) => {
  if (l !== (enSide[i] === '/' ? '/es/' : '/es' + enSide[i])) fail('sidebar', 'sidebar.mts', `entry ${i} differs: ${enSide[i]} / ${l}`)
})
const pageOf = (link) => (link === '/' ? 'index.md' : link.replace(/^\//, '') + '.md')
for (const l of allLinks()) if (!enPages.has(pageOf(l))) fail('sidebar', 'sidebar.mts', `links to missing page ${l}`)

// language + naming
const allowed = (page, text) => ALLOW.some((a) => a.page === page && text.includes(a.text))
const strip = (text, page) => {
  let t = text.replace(/`[^`]*`/g, '').replace(/\]\([^)]*\)/g, ']').replace(/<[^>]+>/g, '')
  for (const a of ALLOW) if (a.page === page) t = t.split(a.text).join('')
  return t
}
const SPANISH_CHARS = /[áéíóúñÁÉÍÓÚÑ¿¡]/
const SPANISH_WORDS = /\b(el|los|las|del|para|que|una|con|por|como|pero|cuando|también|puede|esta|este|sin|sobre|hasta|desde)\b/gi
const VOSEO = /\b(tenés|podés|querés|sabés|hacés|decís|vos|hacé|usá|mirá|fijate|poné|agregá|ejecutá|creá|dale|che)\b/i
const NAMING = /\b(Nest|NestJS|Prisma)\b/

for (const [p, f] of enPages) {
  for (const l of lines(read(f))) {
    if (l.kind === 'fence') continue
    const t = strip(l.text, p)
    if (SPANISH_CHARS.test(t) && !allowed(p, l.text)) fail('language', `${p}:${l.n}`, `Spanish characters: ${l.text.trim().slice(0, 100)}`)
    if (l.kind === 'prose') {
      const words = new Set((t.match(SPANISH_WORDS) ?? []).map((w) => w.toLowerCase()))
      if (words.size >= 2) fail('language', `${p}:${l.n}`, `Spanish words (${[...words].join(', ')}): ${l.text.trim().slice(0, 100)}`)
    }
  }
}
for (const [p, f] of esPages) {
  for (const l of lines(read(f))) {
    if (l.kind === 'prose' && VOSEO.test(strip(l.text, 'es/' + p))) fail('language', `es/${p}:${l.n}`, `voseo: ${l.text.trim().slice(0, 100)}`)
  }
}
for (const [label, pages] of [['', enPages], ['es/', esPages]]) {
  for (const [p, f] of pages) {
    read(f).split('\n').forEach((text, i) => {
      if (NAMING.test(text)) fail('naming', `${label}${p}:${i + 1}`, text.trim().slice(0, 100))
    })
  }
}

// ---------- built HTML ----------

if (!fs.existsSync(dist)) {
  fail('links', '.vitepress/dist', 'not found; run `vitepress build` first')
} else {
  const htmlFiles = (function list(dir) {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
      e.isDirectory() ? (e.name === 'assets' ? [] : list(path.join(dir, e.name))) : e.name.endsWith('.html') ? [path.join(dir, e.name)] : [],
    )
  })(dist)
  const ids = new Map()
  const html = new Map()
  for (const f of htmlFiles) {
    const src = fs.readFileSync(f, 'utf8')
    const route = rel(dist, f)
    html.set(route, src)
    ids.set(route, new Set([...src.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])))
  }
  const target = (p) => {
    const clean = decodeURIComponent(p).replace(/^\//, '')
    for (const c of [clean, clean + '.html', clean.replace(/\/?$/, '/index.html'), clean.replace(/\.html$/, '') + '.html']) {
      if (ids.has(c.replace(/^\//, ''))) return c.replace(/^\//, '')
    }
    return null
  }
  /** The same page in the other language: what the language switcher links to. */
  const counterpart = (route) => {
    const r = '/' + route.replace(/(^|\/)index\.html$/, '$1').replace(/\.html$/, '')
    return r.startsWith('/es/') ? r.slice(3) || '/' : r === '/' ? '/es/' : '/es' + r
  }
  for (const [route, src] of html) {
    if (route === '404.html') continue
    const inEs = route.startsWith('es/')
    const other = counterpart(route)
    for (const m of src.matchAll(/<a\b[^>]*\shref="([^"]+)"/g)) {
      const href = m[1].replace(/&amp;/g, '&')
      if (/^(https?:|mailto:|tel:|\/\/)/.test(href)) continue
      const [p, anchor] = href.split('#')
      if (p && /\.(svg|png|jpe?g|gif|ico|webp|css|js|txt|xml|json)$/i.test(p)) continue
      const dest = p ? target(p.split('?')[0]) : route
      if (!dest) {
        fail('links', route, `broken link ${href}`)
        continue
      }
      if (anchor && !ids.get(dest).has(decodeURIComponent(anchor))) fail('links', route, `missing anchor ${href}`)
      if (p && p.replace(/\/$/, '') !== other.replace(/\/$/, '')) {
        const destEs = dest.startsWith('es/')
        if (inEs !== destEs) fail('links', route, `link leaves the page's language: ${href}`)
      }
    }
  }
}

// ---------- report ----------

if (failures.length) {
  console.error(failures.join('\n'))
  console.error(`\n${failures.length} problem(s) found.`)
  process.exit(1)
}
console.log(`docs check passed: ${enPages.size} English pages, ${esPages.size} Spanish pages.`)
