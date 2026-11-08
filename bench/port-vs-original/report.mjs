#!/usr/bin/env node
// Turns a summary.json of the port-versus-original benchmark into text.
//
//   node bench/port-vs-original/report.mjs <summary.json>
//       writes results.md (English) next to the summary
//   node bench/port-vs-original/report.mjs --docs <summary.json> [--secondary <summary.json>] [--allow-quick]
//       rewrites the blocks between the bench:port-vs-original markers of the English and Spanish docs pages
//   node bench/port-vs-original/report.mjs --docs-placeholder
//       puts the "results pending" placeholder between the markers
//
// Quick summaries are refused by --docs unless --allow-quick is given (to check the plumbing only).
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const REPO = path.resolve(HERE, '..', '..')

export const START = '<!-- bench:port-vs-original:start -->'
export const END = '<!-- bench:port-vs-original:end -->'
const PAGES = { en: 'docs/faq/port-vs-original-benchmark.md', es: 'docs/es/faq/port-vs-original-benchmark.md' }

const L = {
  en: {
    scenario: 'Scenario',
    throughput: 'Throughput (requests per second, median of the runs)',
    ratio: 'Corvis (all cores) / original',
    latency: 'Latency at a fixed rate (ms, median of the runs)',
    rate: 'Rate (req/s)',
    achieved: 'Achieved (req/s)',
    target: 'Target',
    memory: 'Memory of the server process (MB)',
    idle: 'Idle',
    peak: 'Peak',
    after: 'After the load',
    errors: 'Errors and unexpected statuses',
    errorsWord: 'errors',
    unexpectedWord: 'unexpected statuses',
    none: 'None in any run.',
    pending: 'Results pending the publishable run.',
    equal: 'Headline: the same database pool size on both sides (10 connections)',
    default: 'Secondary: each side with its shipped pool size',
    runOf: (s) =>
      `Run of ${s.date} on ${s.environment.cpu} (${s.environment.logicalCores} threads), ${s.environment.os}. Corvis commit ${s.environment.corvisCommit ?? 'unknown'}${s.environment.corvisDirty ? ' (with local changes)' : ''}. ${s.protocol.runs} runs per measurement, ${s.connections} connections.`,
  },
  es: {
    scenario: 'Escenario',
    throughput: 'Rendimiento (peticiones por segundo, mediana de las ejecuciones)',
    ratio: 'Corvis (todos los núcleos) / original',
    latency: 'Latencia con una tasa fija (ms, mediana de las ejecuciones)',
    rate: 'Tasa (req/s)',
    achieved: 'Alcanzada (req/s)',
    target: 'Objetivo',
    memory: 'Memoria del proceso del servidor (MB)',
    idle: 'En reposo',
    peak: 'Pico',
    after: 'Después de la carga',
    errors: 'Errores y estados inesperados',
    errorsWord: 'errores',
    unexpectedWord: 'estados inesperados',
    none: 'Ninguno en ninguna ejecución.',
    pending: 'Resultados pendientes de la ejecución publicable.',
    equal: 'Principal: el mismo tamaño de pool de base de datos en ambos lados (10 conexiones)',
    default: 'Secundaria: cada lado con el tamaño de pool que trae por defecto',
    runOf: (s) =>
      `Ejecución del ${s.date} en ${s.environment.cpu} (${s.environment.logicalCores} hilos), ${s.environment.os}. Commit de Corvis ${s.environment.corvisCommit ?? 'desconocido'}${s.environment.corvisDirty ? ' (con cambios locales)' : ''}. ${s.protocol.runs} ejecuciones por medición, ${s.connections} conexiones.`,
  },
}

const DESC_ES = {
  health: 'Comprobación de salud: sin autenticación ni base de datos',
  'products-list': 'Lista de productos (100 productos): autenticación, consulta con el ORM, JSON grande',
  'product-get': 'Un producto por id: validación del id, búsqueda con el ORM, JSON pequeño',
  'invoice-preview': 'Vista previa de factura (5 ítems, 1 pago): validación del cuerpo, varias lecturas, aritmética decimal, sin persistir',
  login: 'Inicio de sesión: costo del hash de contraseñas (opcional)',
}

const MB = 1024 * 1024
const num = (n, d, lang) => {
  const s = n.toFixed(d)
  return lang === 'es' ? s.replace('.', ',') : s
}
const ms = (n, lang) => num(n, n >= 100 ? 0 : n >= 10 ? 1 : 2, lang)
const int = (n, lang) => num(Math.round(n), 0, lang)
const mb = (n, lang) => num(n / MB, 1, lang)
const row = (cells) => `| ${cells.join(' | ')} |`
const sep = (aligns) => row(aligns.map((a) => (a === 'l' ? '---' : '---:')))

function desc(s, id, lang) {
  return lang === 'es' && DESC_ES[id] ? DESC_ES[id] : s.scenarios.find((x) => x.id === id).description
}

/** Targets in column order: original, one Corvis worker, all cores. */
function orderedTargets(s) {
  const order = ['original', 'corvis-1t', 'corvis']
  return s.targets.filter((t) => order.includes(t.id)).sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id))
}

function cellFlag(cell, lang) {
  const bad = []
  if (cell.errors) bad.push(`${cell.errors} ${L[lang].errorsWord}`)
  if (cell.unexpected) bad.push(`${cell.unexpected} ${L[lang].unexpectedWord}`)
  return bad.length ? ` (!${bad.join(', ')})` : ''
}

const TARGET_ES = { original: 'Original (Node.js)', corvis: 'Corvis (todos los núcleos)', 'corvis-1t': 'Corvis (1 hilo de trabajo)' }
const tl = (x, lang) => (lang === 'es' ? (TARGET_ES[x.id] ?? x.label) : x.label)

export function tables(s, lang) {
  const t = L[lang]
  const targets = orderedTargets(s)
  const out = []
  const hasAll = targets.some((x) => x.id === 'corvis') && targets.some((x) => x.id === 'original')

  out.push(`**${t.throughput}**`, '')
  out.push(row([t.scenario, ...targets.map((x) => tl(x, lang)), ...(hasAll ? [t.ratio] : [])]))
  out.push(sep(['l', ...targets.map(() => 'r'), ...(hasAll ? ['r'] : [])]))
  for (const sc of s.scenarios) {
    const cells = targets.map((x) => {
      const c = s.results[sc.id][x.id]
      return `${int(c.throughput.rps.median, lang)}${cellFlag(c, lang)}`
    })
    const ratio = hasAll ? [`${num(s.results[sc.id].corvis.throughput.rps.median / s.results[sc.id].original.throughput.rps.median, 1, lang)}x`] : []
    out.push(row([desc(s, sc.id, lang), ...cells, ...ratio]))
  }

  out.push('', `**${t.latency}**`, '')
  out.push(row([t.scenario, t.rate, t.target, t.achieved, 'p50', 'p90', 'p99', 'max']))
  out.push(sep(['l', 'r', 'l', 'r', 'r', 'r', 'r', 'r']))
  for (const sc of s.scenarios) {
    targets.forEach((x, i) => {
      const c = s.results[sc.id][x.id]
      out.push(row([i === 0 ? desc(s, sc.id, lang) : '', i === 0 ? int(c.latency.rate, lang) : '', tl(x, lang), int(c.latency.achieved.median, lang), ...['p50', 'p90', 'p99', 'max'].map((k) => ms(c.latency[k].median, lang))]))
    })
  }

  out.push('', `**${t.memory}**`, '')
  out.push(row([t.scenario, t.target, t.idle, t.peak, t.after]))
  out.push(sep(['l', 'l', 'r', 'r', 'r']))
  for (const sc of s.scenarios) {
    targets.forEach((x, i) => {
      const m = s.results[sc.id][x.id].memory
      out.push(row([i === 0 ? desc(s, sc.id, lang) : '', tl(x, lang), mb(m.idle, lang), mb(m.peak, lang), mb(m.after, lang)]))
    })
  }
  return out
}

function errorsSection(s, lang) {
  const t = L[lang]
  const lines = []
  for (const sc of s.scenarios) {
    for (const x of orderedTargets(s)) {
      const c = s.results[sc.id][x.id]
      if (c.errors || c.unexpected) lines.push(`- ${desc(s, sc.id, lang)}, ${tl(x, lang)}: ${c.errors} ${t.errorsWord}, ${c.unexpected} ${t.unexpectedWord}`)
    }
  }
  return [`**${t.errors}**`, '', ...(lines.length ? lines : [t.none])]
}

/** The English results.md next to a summary.json. */
export function renderResults(s) {
  const e = s.environment
  const spreadNote = 'Each cell of the throughput table is the median of the runs; `summary.json` has every run with its minimum and maximum.'
  const lines = [`# Benchmark: a migrated application versus its original`, '']
  if (!s.publishable) lines.push('> **Quick run: not publishable.** One run per measurement and short windows, only to check that the setup works.', '')
  lines.push(
    `Date: ${s.date}. Pool mode: ${s.pool} (${s.pool === 'equal' ? 'the original and the port both use 10 database connections' : 'each side uses its shipped pool size'}).`,
    `Protocol: ${s.protocol.runs} run(s) per measurement, ${s.connections} connections, warmup ${s.protocol.warmupSeconds} s, throughput ${s.protocol.throughputSeconds} s (closed loop), latency ${s.protocol.latencySeconds} s (open loop at half of the original's median throughput, with latency correction). ${s.products} products seeded.`,
    '',
    ...tables(s, 'en'),
    '',
    ...errorsSection(s, 'en'),
    '',
    '## Environment',
    '',
    `- OS: ${e.os}`,
    `- CPU: ${e.cpu}, ${e.logicalCores} logical cores, ${e.memoryGB} GB of memory`,
    `- Node ${e.node.replace(/^v/, '')}, ${e.rustc}, ${e.oha}`,
    `- Postgres: ${e.postgres}`,
    `- Corvis commit: ${e.corvisCommit ?? 'unknown'}${e.corvisDirty ? ' (with local changes)' : ''}; port binary built ${e.portBinaryBuilt}`,
    `- Original: framework ${e.originalFrameworkVersion ?? 'unknown'} on ${e.originalHttpPlatform ?? 'unknown'}, ORM ${e.originalOrmVersion ?? 'unknown'}`,
    `- Load generator: ${e.ohaOptions}`,
    '',
    '## How to read it',
    '',
    '- The load generator, both servers and the database run on one machine; absolute numbers depend on it, ratios are what carries over.',
    '- Throughput is a closed loop at saturation. Latency is measured separately at a fixed rate that the original sustains, so it compares service time, not queueing.',
    '- If a target\'s achieved rate is below the fixed rate, it was saturated and its percentiles include queueing. An error such as "os error 10048" (Windows) is a failure to open a new client connection (the load generator ran out of local ports); check the raw data for how often.',
    '- Memory is the resident set of the server process only (the database is shared). Peak is the operating system counter of the process lifetime, including startup and the warmup.',
    `- ${spreadNote}`,
    '- Every measured request was sent once to both servers before measuring, and their responses were compared (status, headers, body).',
    '',
  )
  return lines.join('\n')
}

export function docsBlock(lang, primary, secondary) {
  const t = L[lang]
  if (!primary) return `${START}\n\n${t.pending}\n\n${END}`
  const lines = [START, '', t.runOf(primary), '', `### ${t.equal}`, '', ...tables(primary, lang), '', ...errorsSection(primary, lang)]
  if (secondary) lines.push('', `### ${t.default}`, '', t.runOf(secondary), '', ...tables(secondary, lang), '', ...errorsSection(secondary, lang))
  lines.push('', END)
  return lines.join('\n')
}

/** Replaces the text between the markers (inclusive); everything else stays byte for byte. */
export function replaceBlock(page, block) {
  const a = page.indexOf(START)
  const b = page.indexOf(END)
  if (a < 0 || b < a) throw new Error('the page has no bench:port-vs-original markers')
  return page.slice(0, a) + block + page.slice(b + END.length)
}

function readSummary(file) {
  return JSON.parse(fs.readFileSync(path.resolve(file), 'utf8'))
}

function writeDocs(primary, secondary) {
  for (const [lang, rel] of Object.entries(PAGES)) {
    const file = path.join(REPO, rel)
    const page = fs.readFileSync(file, 'utf8')
    const next = replaceBlock(page, docsBlock(lang, primary, secondary))
    if (next !== page) fs.writeFileSync(file, next)
    console.log(`${next === page ? 'unchanged' : 'updated'} ${rel}`)
  }
}

function cli(argv) {
  const flag = (n) => argv.includes(n)
  const after = (n) => (argv.includes(n) ? argv[argv.indexOf(n) + 1] : null)
  if (flag('--docs-placeholder')) return writeDocs(null, null)
  if (flag('--docs')) {
    const primary = readSummary(after('--docs'))
    const secondary = after('--secondary') ? readSummary(after('--secondary')) : null
    for (const s of [primary, secondary]) {
      if (s && !s.publishable && !flag('--allow-quick')) throw new Error('refusing to publish a quick (not publishable) summary; pass --allow-quick only to check the plumbing')
    }
    return writeDocs(primary, secondary)
  }
  const file = argv.find((a) => !a.startsWith('--'))
  if (!file) throw new Error('usage: report.mjs <summary.json> | --docs <summary.json> [--secondary <summary.json>] | --docs-placeholder')
  const s = readSummary(file)
  const out = path.join(path.dirname(path.resolve(file)), 'results.md')
  fs.writeFileSync(out, renderResults(s))
  console.log(`wrote ${out}`)
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    cli(process.argv.slice(2))
  } catch (e) {
    console.error(`report.mjs: ${e.message}`)
    process.exit(1)
  }
}
