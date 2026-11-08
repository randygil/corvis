#!/usr/bin/env node
// Port-versus-original benchmark orchestrator. No npm dependencies; Node 22 or later and oha required.
//
//   node bench/port-vs-original/run.mjs [options]        (see --help)
//
// What it does: resets one database per side from the same template, starts both servers, seeds them through the
// HTTP API with one apidiff scenario (scenario.mjs) and aborts on any difference, then measures every scenario on a
// freshly started server of each target: throughput (closed loop), latency (open loop at half of what the original
// sustains) and the memory of the server process. It never builds anything and stops every process it starts.
import { spawn, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { renderResults } from './report.mjs'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const REPO = path.resolve(HERE, '..', '..')
const IS_WIN = process.platform === 'win32'

// ---------- options ----------

const HELP = `Usage: node bench/port-vs-original/run.mjs [options]

  --tests-dir <dir>   folder with the private reference apps (default: a "corvis-tests" folder next to the repository,
                      or CORVIS_TESTS_DIR)
  --app <name>        reference application (default: facten); the port is <tests-dir>/corvis/<name>-api and the
                      original <tests-dir>/nest/<name>-api
  --targets <list>    original,corvis,corvis-1t (default: all three; the original runs first)
  --scenarios <list>  health,products-list,product-get,invoice-preview (default); login is optional
  --pool <mode>       equal (default): the original's URL gets connection_limit=10, like the port's pool;
                      default: both sides with their shipped pool sizes
  --quick             one run per phase and short windows; the output goes to raw/quick and is not publishable
  --label <text>      suffix of the results folder (default: the pool mode)
  --rate <n>          fixed request rate of the latency phase (default: half the original's median throughput)
  --ports <a,b>       ports of the original and the port (default: 3611,3612)
  --keep-db           do not drop the benchmark databases at the end
  --help
`

function parseArgs(argv) {
  const o = {
    testsDir: process.env.CORVIS_TESTS_DIR ?? null,
    app: 'facten',
    targets: ['original', 'corvis', 'corvis-1t'],
    scenarios: ['health', 'products-list', 'product-get', 'invoice-preview'],
    pool: 'equal',
    quick: false,
    label: null,
    rate: null,
    ports: [3611, 3612],
    keepDb: false,
  }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    const val = () => {
      if (i + 1 >= argv.length) throw new Error(`${a} needs a value`)
      return argv[++i]
    }
    if (a === '--help' || a === '-h') {
      process.stdout.write(HELP)
      process.exit(0)
    } else if (a === '--tests-dir') o.testsDir = val()
    else if (a === '--app') o.app = val()
    else if (a === '--targets') o.targets = val().split(',')
    else if (a === '--scenarios') o.scenarios = val().split(',')
    else if (a === '--pool') o.pool = val()
    else if (a === '--quick') o.quick = true
    else if (a === '--label') o.label = val()
    else if (a === '--rate') o.rate = Number(val())
    else if (a === '--ports') o.ports = val().split(',').map(Number)
    else if (a === '--keep-db') o.keepDb = true
    else throw new Error(`unknown option ${a} (see --help)`)
  }
  if (!['equal', 'default'].includes(o.pool)) throw new Error('--pool must be equal or default')
  for (const t of o.targets) if (!TARGETS[t]) throw new Error(`unknown target ${t}`)
  for (const s of o.scenarios) if (!SCENARIOS[s]) throw new Error(`unknown scenario ${s}`)
  if (o.ports.length !== 2 || o.ports.some((p) => !Number.isInteger(p))) throw new Error('--ports needs two ports')
  // The original goes first: its throughput fixes the rate of the latency phase.
  o.targets.sort((x, y) => Object.keys(TARGETS).indexOf(x) - Object.keys(TARGETS).indexOf(y))
  return o
}

const TARGETS = {
  original: { side: 'a', label: 'Original (Node.js)' },
  corvis: { side: 'b', label: 'Corvis (all cores)' },
  'corvis-1t': { side: 'b', label: 'Corvis (1 worker)' },
}

// The seeded administrator of the reference application (email derived from --app; set in main).
const LOGIN = { email: '', password: 'admin123' }
const SCENARIOS = {
  health: { desc: 'Health check: no authentication, no database', method: 'GET', path: () => '/health', auth: false },
  'products-list': {
    desc: 'Product list (100 products): authentication, ORM query, large JSON',
    method: 'GET',
    path: () => '/api/v1/products',
    auth: true,
  },
  'product-get': {
    desc: 'One product by id: id validation, ORM lookup, small JSON',
    method: 'GET',
    path: (s) => `/api/v1/products/${s.productId}`,
    auth: true,
  },
  'invoice-preview': {
    desc: 'Invoice preview (5 items, 1 payment): body validation, several reads, decimal arithmetic, nothing persisted',
    method: 'POST',
    path: () => '/api/v1/invoices/preview',
    auth: true,
    body: (s) => s.previewBody,
  },
  login: {
    desc: 'Login: password hashing cost (optional)',
    method: 'POST',
    path: () => '/api/v1/auth/login',
    auth: false,
    body: () => LOGIN,
  },
}

// ---------- processes ----------

const children = new Set()

function killTree(pid) {
  try {
    if (IS_WIN) spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' })
    else {
      try {
        process.kill(-pid, 'SIGKILL')
      } catch {
        process.kill(pid, 'SIGKILL')
      }
    }
  } catch {
    // already gone
  }
}

function killAll() {
  for (const c of children) killTree(c.pid)
  children.clear()
}

function track(child) {
  children.add(child)
  child.on('exit', () => children.delete(child))
  return child
}

for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.on(sig, () => {
    killAll()
    process.exit(130)
  })
}
process.on('exit', killAll)

function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { encoding: 'utf8', windowsHide: true, ...opts })
  if (r.error) throw new Error(`${cmd}: ${r.error.message}`)
  return r
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

function portOpen(port) {
  return new Promise((resolve) => {
    const s = net.connect({ port, host: '127.0.0.1' })
    s.once('connect', () => (s.destroy(), resolve(true)))
    s.once('error', () => resolve(false))
  })
}

async function waitPortFree(port, ms = 15000) {
  const end = Date.now() + ms
  while (Date.now() < end) {
    if (!(await portOpen(port))) return
    await sleep(200)
  }
  throw new Error(`port ${port} is still in use`)
}

/**
 * fetch that survives a short lack of local ports: right after a closed-loop run thousands of sockets sit in
 * TIME_WAIT, and on Windows a new client connection can fail until some of them expire.
 */
async function fetchRetry(url, init, ms = 30000) {
  const end = Date.now() + ms
  for (;;) {
    try {
      return await fetch(url, init)
    } catch (e) {
      if (Date.now() > end) throw e
      await sleep(500)
    }
  }
}

/** Sockets in TIME_WAIT on the machine (0 when it cannot be read). */
function timeWaitSockets() {
  const r = spawnSync('netstat', IS_WIN ? ['-ano', '-p', 'tcp'] : ['-tan'], { encoding: 'utf8', maxBuffer: 1 << 26 })
  return r.error || r.status !== 0 ? 0 : (r.stdout.match(/TIME_WAIT/g) ?? []).length
}

// ---------- environment of the apps ----------

function locateTestsDir(given) {
  if (given) return path.resolve(given)
  let dir = REPO
  for (let i = 0; i < 8; i++) {
    const c = path.join(dir, 'corvis-tests')
    if (fs.existsSync(c)) return c
    const sibling = path.join(path.dirname(dir), 'corvis-tests')
    if (fs.existsSync(sibling)) return sibling
    dir = path.dirname(dir)
  }
  return path.resolve(REPO, '..', 'corvis-tests')
}

function appPaths(o) {
  const tests = locateTestsDir(o.testsDir)
  const port = path.join(tests, 'corvis', `${o.app}-api`)
  const original = path.join(tests, 'nest', `${o.app}-api`)
  const bin = path.join(port, 'target', 'release', `${o.app}-api${IS_WIN ? '.exe' : ''}`)
  return {
    tests,
    port,
    original,
    bin,
    dist: path.join(original, 'dist', 'main.js'),
    loopback: path.join(port, 'scripts', 'loopback.cjs'),
    apidiff: path.join(tests, 'tools', 'apidiff', 'apidiff.mjs'),
    template: `${o.app}_tpl`,
  }
}

function checkPrerequisites(o, p) {
  const problems = []
  if (Number(process.versions.node.split('.')[0]) < 22) problems.push(`Node 22 or later is required (this is ${process.version})`)
  if (!fs.existsSync(p.tests)) problems.push(`the tests directory ${p.tests} does not exist (use --tests-dir)`)
  const oha = spawnSync('oha', ['--version'], { encoding: 'utf8' })
  if (oha.error || oha.status !== 0) problems.push('oha is not installed: cargo install oha --locked')
  const pg = spawnSync('docker', ['exec', 'corvis-pg', 'pg_isready', '-U', 'postgres'], { encoding: 'utf8' })
  if (pg.error || pg.status !== 0) problems.push('the Postgres container is not running: docker start corvis-pg')
  if (!fs.existsSync(p.apidiff)) problems.push(`apidiff not found at ${p.apidiff}`)
  const builds = []
  // The setup phase starts both sides, so both artifacts are needed whatever --targets says.
  if (!fs.existsSync(p.bin)) {
    builds.push(`the port binary is missing: cd ${p.port} && CARGO_TARGET_DIR="$PWD/target" cargo build --release`)
  }
  if (!fs.existsSync(p.dist)) builds.push(`the original is not built: cd ${p.original} && npm run build`)
  if (!problems.length && !builds.length) {
    const t = spawnSync('docker', ['exec', 'corvis-pg', 'psql', '-U', 'postgres', '-tAc', `SELECT 1 FROM pg_database WHERE datname='${p.template}'`], { encoding: 'utf8' })
    if (t.stdout.trim() !== '1') problems.push(`the template database ${p.template} does not exist`)
  }
  const all = [...problems, ...builds]
  if (all.length) {
    process.stderr.write(`run.mjs: cannot start:\n${all.map((x) => `  - ${x}`).join('\n')}\n`)
    process.stderr.write('run.mjs never builds anything; build the missing artifacts as shown and run it again.\n')
    process.exit(2)
  }
}

const PG_URL = (db) => `postgresql://postgres:postgres@127.0.0.1:55432/${db}?schema=public`

function resetDatabases(p, dbs) {
  for (const db of dbs) {
    const r = run('docker', ['exec', 'corvis-pg', 'psql', '-q', '-U', 'postgres', '-v', 'ON_ERROR_STOP=1', '-c', `DROP DATABASE IF EXISTS ${db} WITH (FORCE);`, '-c', `CREATE DATABASE ${db} TEMPLATE ${p.template};`])
    if (r.status !== 0) throw new Error(`could not reset ${db}: ${r.stderr}`)
  }
}

function dropDatabases(dbs) {
  for (const db of dbs) spawnSync('docker', ['exec', 'corvis-pg', 'psql', '-q', '-U', 'postgres', '-c', `DROP DATABASE IF EXISTS ${db} WITH (FORCE);`], { stdio: 'ignore' })
}

function serverSpec(o, p, target, ctx) {
  const base = { ...process.env }
  for (const k of ['IMPRENTA_DIGITAL_BASE_URL', 'WEB_ORIGIN', 'FORMA_LIBRE_LAYOUT_PATH', 'TOKIO_WORKER_THREADS', 'DATABASE_URL']) delete base[k]
  base.JWT_SECRET = 'apidiff-secret'
  base.JWT_EXPIRES_IN = '8h'
  const side = TARGETS[target].side
  if (side === 'a') {
    const url = PG_URL(ctx.dbA) + (o.pool === 'equal' ? '&connection_limit=10' : '')
    return {
      port: o.ports[0],
      cmd: process.execPath,
      args: ['-r', p.loopback, 'dist/main.js'],
      cwd: p.original,
      env: { ...base, DATABASE_URL: url, PORT: String(o.ports[0]) },
    }
  }
  const env = { ...base, DATABASE_URL: PG_URL(ctx.dbB), PORT: String(o.ports[1]), HOST: '127.0.0.1' }
  if (target === 'corvis-1t') env.TOKIO_WORKER_THREADS = '1'
  return { port: o.ports[1], cmd: p.bin, args: [], cwd: p.port, env }
}

async function startServer(spec, logFile) {
  if (await portOpen(spec.port)) throw new Error(`port ${spec.port} is already in use`)
  const fd = fs.openSync(logFile, 'w')
  const child = track(spawn(spec.cmd, spec.args, { cwd: spec.cwd, env: spec.env, stdio: ['ignore', fd, fd], windowsHide: true, detached: !IS_WIN }))
  fs.closeSync(fd)
  let exited = null
  child.on('exit', (code) => (exited = code))
  try {
    const end = Date.now() + 90000
    for (;;) {
      if (exited !== null) throw new Error(`the server exited with code ${exited} on startup (see ${logFile})`)
      try {
        const r = await fetch(`http://127.0.0.1:${spec.port}/health`)
        if (r.status === 200) break
      } catch {
        // not up yet
      }
      if (Date.now() > end) throw new Error(`the server on ${spec.port} did not answer /health (see ${logFile})`)
      await sleep(250)
    }
  } catch (e) {
    killTree(child.pid)
    throw e
  }
  return {
    pid: child.pid,
    port: spec.port,
    async stop() {
      if (exited === null) {
        killTree(child.pid)
        const end = Date.now() + 10000
        while (exited === null && Date.now() < end) await sleep(100)
      }
      await waitPortFree(spec.port)
    },
  }
}

// ---------- memory ----------

/** Samples the resident set and the OS peak of one process every 500 ms (Windows: PowerShell; Linux: /proc). */
function startSampler(pid) {
  const samples = []
  const waiters = []
  const push = (ws, peak) => {
    const s = { t: Date.now(), ws, peak }
    samples.push(s)
    while (waiters.length) waiters.shift()(s)
  }
  let timer = null
  let ps = null
  if (IS_WIN) {
    const script = `$ErrorActionPreference='Stop'; while($true){ try { $p = Get-Process -Id ${pid}; [Console]::Out.WriteLine(('{0} {1}' -f $p.WorkingSet64, $p.PeakWorkingSet64)) } catch { exit }; Start-Sleep -Milliseconds 500 }`
    ps = track(spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { stdio: ['ignore', 'pipe', 'ignore'], windowsHide: true }))
    let buf = ''
    ps.stdout.on('data', (d) => {
      buf += d
      let i
      while ((i = buf.indexOf('\n')) >= 0) {
        const m = buf.slice(0, i).trim().match(/^(\d+) (\d+)$/)
        buf = buf.slice(i + 1)
        if (m) push(Number(m[1]), Number(m[2]))
      }
    })
  } else {
    const read = () => {
      try {
        const st = fs.readFileSync(`/proc/${pid}/status`, 'utf8')
        const kb = (k) => Number(st.match(new RegExp(`^${k}:\\s+(\\d+) kB`, 'm'))?.[1] ?? 0) * 1024
        push(kb('VmRSS'), kb('VmHWM'))
      } catch {
        // process gone
      }
    }
    read()
    timer = setInterval(read, 500)
  }
  return {
    samples,
    /** The next sample taken from now on. */
    next: () =>
      new Promise((resolve, reject) => {
        const t = setTimeout(() => reject(new Error('the memory sampler produced no sample')), 15000)
        waiters.push((s) => (clearTimeout(t), resolve(s)))
      }),
    stop() {
      if (timer) clearInterval(timer)
      if (ps) killTree(ps.pid)
    },
  }
}

// ---------- load ----------

const ohaVersion = () => spawnSync('oha', ['--version'], { encoding: 'utf8' }).stdout.trim()
const OHA_THREADS = 8
const CONNECTIONS = 100

function oha(opts) {
  const args = ['--no-tui', '-w', '-c', String(CONNECTIONS), '-z', `${opts.seconds}s`, '--worker-threads', String(OHA_THREADS), '--output-format', 'json', '-o', opts.out, '-m', opts.method]
  if (opts.rate) args.push('-q', String(opts.rate), '--latency-correction')
  for (const h of opts.headers ?? []) args.push('-H', h)
  if (opts.bodyFile) args.push('-T', 'application/json', '-D', opts.bodyFile)
  args.push(opts.url)
  return new Promise((resolve, reject) => {
    const child = track(spawn('oha', args, { stdio: ['ignore', 'ignore', 'pipe'], windowsHide: true }))
    let err = ''
    child.stderr.on('data', (d) => (err += d))
    child.on('exit', (code) => {
      if (code !== 0) return reject(new Error(`oha exited with ${code}: ${err.trim()}`))
      try {
        resolve(parseOha(JSON.parse(fs.readFileSync(opts.out, 'utf8'))))
      } catch (e) {
        reject(new Error(`could not read oha output ${opts.out}: ${e.message}`))
      }
    })
  })
}

function parseOha(j) {
  const ms = (s) => (s == null ? null : s * 1000)
  const statuses = j.statusCodeDistribution ?? {}
  const total = Object.values(statuses).reduce((a, b) => a + b, 0)
  const ok = statuses['200'] ?? 0
  const errors = Object.values(j.errorDistribution ?? {}).reduce((a, b) => a + b, 0)
  return {
    rps: j.summary.requestsPerSec,
    p50: ms(j.latencyPercentiles.p50),
    p90: ms(j.latencyPercentiles.p90),
    p99: ms(j.latencyPercentiles.p99),
    max: ms(j.summary.slowest),
    requests: total,
    errors,
    unexpected: total - ok,
    statuses,
  }
}

const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b)
  const m = s.length >> 1
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2
}
const spread = (xs) => ({ median: median(xs), min: Math.min(...xs), max: Math.max(...xs) })

// ---------- setup phase ----------

function runSetup(o, p, ctx) {
  const env = { ...process.env, BENCH_STATE_FILE: ctx.stateFile, BENCH_SCENARIOS: o.scenarios.join(','), BENCH_ADMIN_EMAIL: LOGIN.email, BENCH_ADMIN_PASSWORD: LOGIN.password }
  if (ctx.expectProducts != null) env.BENCH_EXPECT_PRODUCTS = String(ctx.expectProducts)
  const r = run(process.execPath, [p.apidiff, 'run', '--a', `http://127.0.0.1:${o.ports[0]}`, '--b', `http://127.0.0.1:${o.ports[1]}`, '--quiet', '--json', path.join(ctx.rawDir, 'setup-apidiff.json'), ctx.scenarioFile], { env, cwd: HERE, maxBuffer: 1 << 26 })
  fs.writeFileSync(path.join(ctx.rawDir, 'setup-apidiff.log'), (r.stdout ?? '') + (r.stderr ?? ''))
  return r
}

async function setupPhase(o, p, ctx) {
  log('resetting the databases from the template')
  resetDatabases(p, [ctx.dbA, ctx.dbB])
  const a = await startServer(serverSpec(o, p, 'original', ctx), path.join(ctx.rawDir, 'server-setup-original.log'))
  let b
  try {
    b = await startServer(serverSpec(o, p, 'corvis', ctx), path.join(ctx.rawDir, 'server-setup-corvis.log'))
    log('seeding both sides and comparing every measured request (apidiff)')
    const r = runSetup(o, p, ctx)
    if (r.status !== 0) {
      const tail = ((r.stdout ?? '') + (r.stderr ?? '')).trim().split('\n').slice(-25).join('\n')
      throw new Error(`the setup scenario found differences or failed (exit ${r.status}); nothing was measured.\n${tail}\nFull log: ${path.join(ctx.rawDir, 'setup-apidiff.log')}`)
    }
    const last = (r.stdout ?? '').trim().split('\n').filter(Boolean).pop()
    log(`apidiff: ${last}`)
  } finally {
    if (b) await b.stop()
    await a.stop()
  }
}

// ---------- measurement ----------

function requestFor(scenario, state, ctx) {
  const def = SCENARIOS[scenario]
  const headers = def.auth ? [`authorization: Bearer ${state.token}`] : []
  const body = def.body ? def.body(state) : null
  return { def, headers, body, path: def.path(state) }
}

async function precheck(scenario, target, state, port, ctx) {
  const { def, headers, body, path: p } = requestFor(scenario, state, ctx)
  const res = await fetchRetry(`http://127.0.0.1:${port}${p}`, {
    method: def.method,
    headers: { ...Object.fromEntries(headers.map((h) => h.split(': '))), ...(body ? { 'content-type': 'application/json' } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  })
  const text = await res.text()
  if (res.status !== 200) throw new Error(`${target}/${scenario}: expected 200, got ${res.status}: ${text.slice(0, 200)}`)
  let print = String(text.length)
  try {
    const j = JSON.parse(text)
    if (scenario === 'products-list') print = `count=${Array.isArray(j) ? j.length : '?'}`
    if (scenario === 'invoice-preview') print = `total=${j.totalUsd}/${j.totalVes}`
  } catch {
    // health returns whatever it returns
  }
  return print
}

async function measureTarget(o, p, ctx, scenario, target, rateBox) {
  const runs = o.quick ? 1 : 3
  const win = o.quick ? { warm: 2, tp: 4, lat: 6 } : { warm: 5, tp: 15, lat: 30 }
  const state = ctx.state[TARGETS[target].side]
  const { def, headers, body } = requestFor(scenario, state, ctx)
  const tag = `${scenario}-${target}`
  const spec = serverSpec(o, p, target, ctx)
  const url = `http://127.0.0.1:${spec.port}${def.path(state)}`
  let bodyFile = null
  if (body) {
    bodyFile = path.join(ctx.rawDir, `body-${scenario}-${TARGETS[target].side}.json`)
    fs.writeFileSync(bodyFile, JSON.stringify(body))
  }
  const srv = await startServer(spec, path.join(ctx.rawDir, `server-${tag}.log`))
  const sampler = startSampler(srv.pid)
  const mem = {}
  const out = { throughput: { runs: [] }, latency: { runs: [] } }
  try {
    await sampler.next()
    out.fingerprint = await precheck(scenario, target, state, spec.port, ctx)
    const ref = ctx.fingerprints[scenario]
    if (ref === undefined) ctx.fingerprints[scenario] = out.fingerprint
    else if (ref !== out.fingerprint) throw new Error(`${target}/${scenario}: the response differs from the first target's (${out.fingerprint} against ${ref}); nothing published from this run`)
    await sleep(1000)
    mem.idle = (await sampler.next()).ws
    const base = { method: def.method, headers, bodyFile, url }
    await oha({ ...base, seconds: win.warm, out: path.join(ctx.rawDir, `${tag}-warmup.json`) })
    await waitPortHealthy(spec.port)
    for (let i = 1; i <= runs; i++) {
      log(`${tag}: throughput run ${i}/${runs} (${win.tp} s)`)
      out.throughput.runs.push(await oha({ ...base, seconds: win.tp, out: path.join(ctx.rawDir, `${tag}-throughput-${i}.json`) }))
      await waitPortHealthy(spec.port)
    }
    out.throughput.rps = spread(out.throughput.runs.map((r) => r.rps))
    mem.afterThroughputPeak = (await sampler.next()).peak
    if (target === 'original' && rateBox.rate == null) rateBox.rate = Math.max(1, Math.round(0.5 * out.throughput.rps.median))
    if (rateBox.rate == null) throw new Error('the rate of the latency phase is unknown: run the original or pass --rate')
    out.latency.rate = rateBox.rate
    for (let i = 1; i <= runs; i++) {
      log(`${tag}: latency run ${i}/${runs} (${win.lat} s at ${rateBox.rate} req/s)`)
      out.latency.runs.push(await oha({ ...base, seconds: win.lat, rate: rateBox.rate, out: path.join(ctx.rawDir, `${tag}-latency-${i}.json`) }))
      await waitPortHealthy(spec.port)
    }
    out.latency.achieved = spread(out.latency.runs.map((r) => r.rps))
    for (const k of ['p50', 'p90', 'p99', 'max']) out.latency[k] = spread(out.latency.runs.map((r) => r[k]))
    await sleep(o.quick ? 1000 : 3000)
    const after = await sampler.next()
    mem.after = after.ws
    mem.peak = Math.max(after.peak, ...sampler.samples.map((s) => s.ws))
    mem.samples = sampler.samples.length
    fs.writeFileSync(path.join(ctx.rawDir, `${tag}-memory.json`), JSON.stringify(sampler.samples))
    out.memory = mem
    out.errors = [...out.throughput.runs, ...out.latency.runs].reduce((a, r) => a + r.errors, 0)
    out.unexpected = [...out.throughput.runs, ...out.latency.runs].reduce((a, r) => a + r.unexpected, 0)
    return out
  } finally {
    sampler.stop()
    await srv.stop()
  }
}

async function waitPortHealthy(port) {
  const r = await fetchRetry(`http://127.0.0.1:${port}/health`)
  if (r.status !== 200) throw new Error(`health check after the run: ${r.status}`)
}

// ---------- summary ----------

function gitInfo(dir) {
  const g = (...a) => spawnSync('git', ['-C', dir, ...a], { encoding: 'utf8' })
  const head = g('rev-parse', '--short=12', 'HEAD')
  if (head.status !== 0) return { commit: null, dirty: null }
  return { commit: head.stdout.trim(), dirty: g('status', '--porcelain', '--', '.').stdout.trim() !== '' }
}

function corvisCheckout(p) {
  const toml = fs.readFileSync(path.join(p.port, 'Cargo.toml'), 'utf8')
  const m = toml.match(/^corvis\s*=\s*\{[^}]*path\s*=\s*"([^"]+)"/m)
  return m ? path.resolve(p.port, m[1], '..', '..') : null
}

function environment(o, p) {
  const dep = (name) => {
    try {
      const pkg = JSON.parse(fs.readFileSync(path.join(p.original, 'package.json'), 'utf8'))
      return (pkg.dependencies?.[name] ?? pkg.devDependencies?.[name] ?? '').replace(/^[\^~]/, '') || null
    } catch {
      return null
    }
  }
  const pgv = run('docker', ['exec', 'corvis-pg', 'psql', '-U', 'postgres', '-tAc', 'SHOW server_version']).stdout.trim()
  const root = corvisCheckout(p)
  const git = root ? gitInfo(root) : { commit: null, dirty: null }
  const rustc = spawnSync('rustc', ['--version'], { encoding: 'utf8' }).stdout.trim()
  const cpus = os.cpus()
  return {
    os: `${os.type()} ${os.release()} (${process.platform} ${os.arch()})`,
    cpu: cpus[0].model.trim(),
    logicalCores: cpus.length,
    memoryGB: Math.round(os.totalmem() / 2 ** 30),
    node: process.version,
    rustc: rustc || null,
    oha: ohaVersion(),
    ohaOptions: `closed and open loop, ${CONNECTIONS} connections, ${OHA_THREADS} worker threads, keep-alive, wait for in-flight requests`,
    postgres: `${pgv} (Docker container on 127.0.0.1)`,
    corvisCommit: git.commit,
    corvisDirty: git.dirty,
    portBinaryBuilt: fs.statSync(p.bin).mtime.toISOString(),
    originalOrmVersion: dep('@prisma/client'),
    originalFrameworkVersion: dep('@nestjs/core'),
    originalHttpPlatform: dep('@nestjs/platform-express') ? 'express' : dep('@nestjs/platform-fastify') ? 'fastify' : null,
  }
}

function log(msg) {
  process.stdout.write(`[${new Date().toTimeString().slice(0, 8)}] ${msg}\n`)
}

// ---------- main ----------

async function main() {
  const o = parseArgs(process.argv.slice(2))
  const p = appPaths(o)
  LOGIN.email = `admin@${o.app}.local`
  checkPrerequisites(o, p)
  const timeWait = timeWaitSockets()
  if (timeWait > 5000) log(`warning: ${timeWait} sockets in TIME_WAIT on this machine; the load generator may run out of local ports (other work still running?)`)
  if (!o.targets.includes('original') && o.rate == null) throw new Error('without the original in --targets the latency rate is unknown: pass --rate')

  const date = new Date().toISOString().slice(0, 10)
  const label = o.label ?? o.pool
  const folder = `${date}-${label}`
  const rawDir = path.join(HERE, 'raw', o.quick ? `quick-${folder}` : folder)
  const outDir = o.quick ? path.join(HERE, 'raw', 'quick', folder) : path.join(HERE, 'results', folder)
  fs.mkdirSync(rawDir, { recursive: true })
  fs.mkdirSync(outDir, { recursive: true })
  const ctx = {
    rawDir,
    dbA: 'bench_original',
    dbB: 'bench_port',
    stateFile: path.join(rawDir, 'state.json'),
    scenarioFile: process.env.BENCH_SCENARIO_FILE ? path.resolve(process.env.BENCH_SCENARIO_FILE) : path.join(HERE, 'scenario.mjs'),
    expectProducts: process.env.BENCH_EXPECT_PRODUCTS ? Number(process.env.BENCH_EXPECT_PRODUCTS) : null,
    fingerprints: {},
  }
  const started = new Date()
  const dbs = [ctx.dbA, ctx.dbB]
  try {
    await setupPhase(o, p, ctx)
    ctx.state = JSON.parse(fs.readFileSync(ctx.stateFile, 'utf8'))
    const results = {}
    for (const sc of o.scenarios) {
      results[sc] = {}
      const rateBox = { rate: o.rate }
      for (const t of o.targets) {
        log(`${sc} on ${t}`)
        results[sc][t] = await measureTarget(o, p, ctx, sc, t, rateBox)
      }
    }
    const summary = {
      schema: 1,
      publishable: !o.quick,
      date,
      startedAt: started.toISOString(),
      finishedAt: new Date().toISOString(),
      label,
      pool: o.pool,
      poolSizes: { port: 10, original: o.pool === 'equal' ? 10 : 'ORM default (2 x physical cores + 1)' },
      quick: o.quick,
      protocol: o.quick
        ? { runs: 1, warmupSeconds: 2, throughputSeconds: 4, latencySeconds: 6 }
        : { runs: 3, warmupSeconds: 5, throughputSeconds: 15, latencySeconds: 30 },
      connections: CONNECTIONS,
      products: ctx.state.products,
      targets: o.targets.map((id) => ({ id, label: TARGETS[id].label })),
      scenarios: o.scenarios.map((id) => ({ id, description: SCENARIOS[id].desc })),
      environment: environment(o, p),
      results,
    }
    fs.writeFileSync(path.join(outDir, 'summary.json'), JSON.stringify(summary, null, 2) + '\n')
    fs.writeFileSync(path.join(outDir, 'results.md'), renderResults(summary))
    log(`wrote ${path.relative(REPO, path.join(outDir, 'summary.json'))} and results.md`)
  } finally {
    killAll()
    if (!o.keepDb) dropDatabases(dbs)
  }
}

main().then(
  () => process.exit(0),
  (e) => {
    killAll()
    process.stderr.write(`run.mjs: ${e.message}\n`)
    process.exit(1)
  },
)
