// Smoke test of the example applications: starts each example binary, sends the documented requests and checks
// status codes and key fields. It builds nothing: run `cargo build -p blog-orm -p tertulia -p cats-app -p auth-app` first.
//
//   node scripts/smoke-examples.mjs [blog-orm|tertulia|cats-app|auth-app ...]
//
// The binaries come from `$CARGO_TARGET_DIR/debug` (or `target/debug`). Every server binds to 127.0.0.1 on a free
// port, runs in its own temporary directory with a temporary SQLite database and a random JWT secret, and is killed
// at the end together with its temporary files. Exits with status 1 when a check fails.
import { spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(root, 'target')
const binDir = path.join(targetDir, 'debug')
const exe = (name) => path.join(binDir, name + (process.platform === 'win32' ? '.exe' : ''))

const children = new Set()
const tempDirs = new Set()
let failures = 0
let checks = 0

function check(label, ok, detail = '') {
  checks++
  if (!ok) failures++
  console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${label}${ok || !detail ? '' : `  (${detail})`}`)
}

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address()
      server.close(() => resolve(port))
    })
  })
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

async function start(name, env, args = []) {
  const bin = exe(name)
  if (!fs.existsSync(bin)) throw new Error(`${bin} not found: run \`cargo build -p ${name}\` first`)
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `corvis-smoke-${name}-`))
  tempDirs.add(dir)
  const port = await freePort()
  const child = spawn(bin, args, {
    cwd: dir, // no .env and no stray database file in the repository
    env: { ...process.env, PORT: String(port), DATABASE_URL: `file:${path.join(dir, 'smoke.db').replaceAll('\\', '/')}`, ...env },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  children.add(child)
  let log = ''
  child.stdout.on('data', (d) => (log += d))
  child.stderr.on('data', (d) => (log += d))
  let exited = null
  child.on('exit', (code) => (exited = code))
  const base = `http://127.0.0.1:${port}`
  for (let i = 0; i < 100; i++) {
    if (exited !== null) throw new Error(`${name} exited with ${exited} at startup:\n${log}`)
    try {
      await fetch(base + '/', { signal: AbortSignal.timeout(500) })
      return { base, child, log: () => log }
    } catch {
      await sleep(100)
    }
  }
  throw new Error(`${name} did not answer on ${base}:\n${log}`)
}

async function req(base, method, url, { body, token, cookie, headers: extra } = {}) {
  const headers = { ...extra }
  if (cookie) headers.cookie = cookie
  if (body !== undefined) headers['content-type'] = 'application/json'
  if (token) headers.authorization = `Bearer ${token}`
  const res = await fetch(base + url, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), redirect: 'manual' })
  const text = await res.text()
  let json
  try {
    json = JSON.parse(text)
  } catch {}
  return { status: res.status, text, json, type: res.headers.get('content-type') ?? '', setCookie: res.headers.getSetCookie?.() ?? [], location: res.headers.get('location') }
}

/** First chunk of a Server-Sent Events stream. */
async function firstSseChunk(base, url) {
  const ctl = new AbortController()
  const timer = setTimeout(() => ctl.abort(), 5000)
  try {
    const res = await fetch(base + url, { signal: ctl.signal })
    const reader = res.body.getReader()
    const { value } = await reader.read()
    await reader.cancel()
    return { status: res.status, type: res.headers.get('content-type') ?? '', text: new TextDecoder().decode(value) }
  } finally {
    clearTimeout(timer)
    ctl.abort()
  }
}

const suites = {
  async 'blog-orm'() {
    const { base } = await start('blog-orm', {})
    const email = `ana-${randomBytes(3).toString('hex')}@corvis.dev`
    let r = await req(base, 'POST', '/users', { body: { email, name: 'Ana' } })
    check('POST /users creates a user (201)', r.status === 201 && r.json?.email === email, `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/users', { body: { email, name: 'Ana' } })
    check('POST /users with the same email is a conflict (409)', r.status === 409, `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/users', { body: { email: 'not-an-email', name: '' } })
    check('POST /users with invalid data is rejected (400)', r.status === 400, `${r.status} ${r.text}`)
    for (const [title, published] of [['Draft', false], ['Live', true]]) {
      r = await req(base, 'POST', '/posts', { body: { title, published, authorEmail: email } })
      check(`POST /posts creates "${title}" (201)`, r.status === 201, `${r.status} ${r.text}`)
    }
    r = await req(base, 'GET', '/posts?page=1&limit=10')
    check('GET /posts lists posts (200)', r.status === 200, `${r.status} ${r.text}`)
    const titles = async (q) => {
      const res = await req(base, 'GET', `/posts${q}`)
      const rows = Array.isArray(res.json) ? res.json : res.json?.data
      return res.status === 200 && Array.isArray(rows) ? rows.map((p) => p.title).sort().join(',') : `${res.status} ${res.text}`
    }
    check('GET /posts?published=false returns only the draft', (await titles('?published=false')) === 'Draft', await titles('?published=false'))
    check('GET /posts?published=true returns only the live post', (await titles('?published=true')) === 'Live', await titles('?published=true'))
    r = await req(base, 'GET', '/posts?published=maybe')
    check('GET /posts?published=maybe is rejected (400)', r.status === 400, `${r.status} ${r.text}`)
  },

  async tertulia() {
    const { base } = await start('tertulia', { JWT_SECRET: randomBytes(32).toString('hex'), STATS_INTERVAL_MS: '200' })
    let r = await req(base, 'GET', '/')
    check('GET / serves the browser client (200 html)', r.status === 200 && r.type.includes('text/html'), `${r.status} ${r.type}`)
    r = await req(base, 'GET', '/api/rooms')
    check('GET /api/rooms lists rooms (200)', r.status === 200 && (Array.isArray(r.json) || Array.isArray(r.json?.data)), `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/api/auth/me')
    check('GET /api/auth/me without a token is 401', r.status === 401, `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/api/auth/login', { body: { username: 'smoke_user' } })
    const token = r.json?.accessToken
    check('POST /api/auth/login returns a token (200)', r.status === 200 && typeof token === 'string', `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/api/auth/me', { token })
    check('GET /api/auth/me with the token is 200', r.status === 200, `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/api/auth/login', { body: { username: 'x' } })
    check('POST /api/auth/login with an invalid nickname is 400', r.status === 400, `${r.status} ${r.text}`)
    const sse = await firstSseChunk(base, '/api/stats/stream')
    check('GET /api/stats/stream sends a first "stats" event', sse.status === 200 && sse.type.includes('text/event-stream') && sse.text.includes('event: stats'), `${sse.status} ${sse.type} ${sse.text}`)
  },

  async 'cats-app'() {
    const { base } = await start('cats-app', { JWT_SECRET: randomBytes(32).toString('hex') })
    let r = await req(base, 'GET', '/api/v1')
    check('GET /api/v1 says hello (200)', r.status === 200, `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/api/health')
    check('GET /api/health is healthy (200)', r.status === 200 && r.json?.status === 'ok', `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/api/v1/cats')
    check('GET /api/v1/cats is public (200)', r.status === 200, `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/api/v1/cats/count')
    check('GET /api/v1/cats/count returns a count (200)', r.status === 200 && typeof r.json?.count === 'number', `${r.status} ${r.text}`)
    const cat = { name: 'Tom', age: 3, breed: 'siamese' }
    r = await req(base, 'POST', '/api/v1/cats', { body: cat })
    check('POST /api/v1/cats without a token is 401', r.status === 401, `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/api/v1/auth/login', { body: { username: 'john', password: 'wrong-password' } })
    check('POST /api/v1/auth/login with a wrong password is 401', r.status === 401, `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/api/v1/auth/login', { body: { username: 'john', password: 'changeme' } })
    const token = r.json?.access_token
    check('POST /api/v1/auth/login returns a token (200)', r.status === 200 && typeof token === 'string', `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/api/v1/auth/profile', { token })
    check('GET /api/v1/auth/profile with the token is 200', r.status === 200 && r.json?.username === 'john', `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/api/v1/cats', { body: cat, token })
    check('POST /api/v1/cats creates a cat (201)', r.status === 201 && r.json?.ownerId === 1, `${r.status} ${r.text}`)
    const id = r.json?.id
    r = await req(base, 'POST', '/api/v1/cats', { body: { ...cat, breed: 'dragon' }, token })
    check('POST /api/v1/cats with an invalid breed is 400', r.status === 400, `${r.status} ${r.text}`)
    r = await req(base, 'GET', `/api/v1/cats/${id}`)
    check('GET /api/v1/cats/:id finds the cat (200)', r.status === 200 && r.json?.name === 'Tom', `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/api/v1/cats/9999')
    check('GET /api/v1/cats/9999 is 404', r.status === 404, `${r.status} ${r.text}`)
    r = await req(base, 'DELETE', `/api/v1/cats/${id}`, { token })
    check('DELETE /api/v1/cats/:id as admin is 204', r.status === 204, `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/docs')
    check('GET /docs serves Swagger UI (200 html)', r.status === 200 && r.type.includes('text/html'), `${r.status} ${r.type}`)
    r = await req(base, 'GET', '/docs-json')
    check('GET /docs-json documents the cats routes', r.status === 200 && !!r.json?.paths?.['/api/v1/cats'] && !!r.json?.components?.schemas?.CreateCatDto, `${r.status} ${r.text.slice(0, 120)}`)
    const sse = await firstSseChunk(base, '/api/v1/cats/stream')
    check('GET /api/v1/cats/stream sends a first event', sse.status === 200 && sse.type.includes('text/event-stream') && sse.text.includes('data:'), `${sse.status} ${sse.type} ${sse.text}`)
  },

  async 'auth-app'() {
    // `--dev` generates the secrets, starts the mock OAuth2 provider on 127.0.0.1 and prints the service API key.
    const { base, log } = await start('auth-app', {}, ['--dev'])
    const key = /service API key for X-Api-Key: (\S+)/.exec(log())?.[1]
    check('--dev prints the service API key', typeof key === 'string' && key.length >= 32)
    let r = await req(base, 'GET', '/')
    check('GET / is public (200)', r.status === 200, `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/me')
    check('GET /me without a token is 401', r.status === 401 && r.text === '{"message":"Unauthorized","statusCode":401}', `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/auth/login', { body: { username: 'john', password: 'wrong-password' } })
    check('POST /auth/login with a wrong password is 401', r.status === 401, `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/auth/login', { body: { username: 'john', password: 'john-demo-pass' } })
    const token = r.json?.access_token
    check('POST /auth/login returns a token (200)', r.status === 200 && typeof token === 'string', `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/me', { token })
    check('GET /me with the token is 200', r.status === 200 && r.json?.username === 'john', `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/reports', { headers: { 'x-api-key': key } })
    check('GET /reports with the service key is 200', r.status === 200 && r.json?.caller?.service === 'reports', `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/reports', { headers: { 'x-api-key': 'wrong' } })
    check('GET /reports with a wrong key is 401', r.status === 401, `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/admin/users', { token })
    check('GET /admin/users as admin is 200', r.status === 200 && Array.isArray(r.json), `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/session/login', { body: { username: 'maria', password: 'maria-demo-pass' } })
    const cookie = r.setCookie[0]?.split(';')[0]
    check('POST /session/login sets a session cookie (200)', r.status === 200 && cookie?.startsWith('sid='), `${r.status} ${r.setCookie}`)
    r = await req(base, 'GET', '/session/me', { cookie })
    check('GET /session/me with the cookie is 200', r.status === 200 && r.json?.username === 'maria', `${r.status} ${r.text}`)
    r = await req(base, 'POST', '/session/logout', { cookie })
    check('POST /session/logout is 204', r.status === 204, `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/session/me', { cookie })
    check('the cookie is anonymous after the logout (401)', r.status === 401, `${r.status} ${r.text}`)
    // OAuth2 against the mock provider: the redirects are followed by hand, carrying the state cookie.
    r = await req(base, 'GET', '/auth/provider')
    const state = r.setCookie[0]?.split(';')[0]
    check('GET /auth/provider redirects to the mock provider (302)', r.status === 302 && r.location?.startsWith('http://127.0.0.1:'), `${r.status} ${r.location}`)
    const approved = await fetch(r.location, { redirect: 'manual' })
    const callback = approved.headers.get('location')
    check('the mock provider redirects back to the callback (302)', approved.status === 302 && callback?.includes('/auth/provider/callback?code='), `${approved.status} ${callback}`)
    const target = new URL(callback)
    r = await req(base, 'GET', target.pathname + target.search, { cookie: state })
    const session = r.setCookie.map((c) => c.split(';')[0]).find((c) => c.startsWith('sid='))
    check('the callback logs in (200, session cookie)', r.status === 200 && !!session, `${r.status} ${r.text}`)
    r = await req(base, 'GET', '/session/me', { cookie: session })
    check('GET /session/me after the OAuth2 login is 200', r.status === 200 && r.json?.username?.startsWith('provider:'), `${r.status} ${r.text}`)
  },
}

function cleanup() {
  for (const c of children) {
    try {
      c.kill()
    } catch {}
  }
  children.clear()
}

async function removeTempDirs() {
  for (const dir of tempDirs) {
    // The processes may still hold the SQLite file for a moment on Windows.
    for (let i = 0; i < 20; i++) {
      try {
        fs.rmSync(dir, { recursive: true, force: true })
        break
      } catch {
        await sleep(100)
      }
    }
  }
  tempDirs.clear()
}

const wanted = process.argv.slice(2)
const names = wanted.length ? wanted : Object.keys(suites)
for (const n of names) {
  if (!suites[n]) {
    console.error(`unknown example: ${n} (expected one of ${Object.keys(suites).join(', ')})`)
    process.exit(2)
  }
}
process.on('SIGINT', () => {
  cleanup()
  process.exit(130)
})

for (const name of names) {
  console.log(name)
  try {
    await suites[name]()
  } catch (e) {
    failures++
    console.log(`  FAIL ${e.message}`)
  }
}
cleanup()
await sleep(300)
await removeTempDirs()
console.log(`\n${checks - failures}/${checks} checks passed${failures ? `, ${failures} failure(s)` : ''}`)
process.exit(failures ? 1 : 0)
