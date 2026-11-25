// Run with `node --test frontend/scripts/vite-proxy.test.mjs`: the dev proxy's Spark forwarding rules.
import { after, before, test } from 'node:test'
import assert from 'node:assert/strict'
import http from 'node:http'
import net from 'node:net'
import { fileURLToPath } from 'node:url'
import { createServer } from 'vite'

const frontend = fileURLToPath(new URL('..', import.meta.url))
let received = []
const backend = http.createServer((req, res) => {
  let body = ''
  req.on('data', (chunk) => (body += chunk))
  req.on('end', () => {
    received.push({ method: req.method, url: req.url, headers: req.headers, rawHeaders: req.rawHeaders, body })
    res.writeHead(200, { 'Content-Type': 'application/json' }).end('{}')
  })
})
let vite, vitePort, backendOrigin, local

before(async () => {
  await new Promise((resolve) => backend.listen(0, '127.0.0.1', resolve))
  backendOrigin = `http://127.0.0.1:${backend.address().port}`
  process.env.VITE_BACKEND_URL = backendOrigin
  vite = await createServer({
    configFile: `${frontend}/vite.config.ts`,
    root: frontend,
    logLevel: 'silent',
    optimizeDeps: { noDiscovery: true },
    server: { host: '127.0.0.1', port: 0, hmr: false, ws: false, watch: null },
  })
  await vite.listen()
  vitePort = vite.httpServer.address().port
  local = `127.0.0.1:${vitePort}`
  assert.notEqual(vitePort, backend.address().port)
})

after(async () => {
  await vite?.close()
  backend.closeAllConnections()
  await new Promise((resolve) => backend.close(resolve))
})

// Sends one raw request (so duplicate and malformed headers reach Vite as written) and
// returns the status plus what the backend received for it, if anything.
async function send(target, headers, body = '') {
  received = []
  const status = await new Promise((resolve, reject) => {
    const socket = net.connect(vitePort, '127.0.0.1')
    socket.once('data', (chunk) => {
      resolve(Number(chunk.toString().split(' ')[1]))
      socket.destroy()
    })
    socket.on('error', reject)
    socket.write([target + ' HTTP/1.1', ...headers, `Content-Length: ${Buffer.byteLength(body)}`, '', body].join('\r\n'))
  })
  return { status, upstream: received[0], forwarded: received.length }
}

test('same-origin local mutation reaches Spark as a same-origin backend request', async () => {
  const { status, upstream } = await send('POST /workspace/api/runs', [
    `Host: ${local}`, `Origin: http://${local}`, 'Sec-Fetch-Site: same-origin', 'Content-Type: application/json',
  ], '{"a":1}')
  assert.equal(status, 200)
  assert.equal(upstream.method, 'POST')
  assert.equal(upstream.url, '/workspace/api/runs')
  assert.equal(`http://${upstream.headers.host}`, backendOrigin)
  assert.equal(upstream.headers.origin, backendOrigin)
  assert.equal(upstream.headers['x-forwarded-for'], undefined)
  assert.equal(upstream.body, '{"a":1}')
})

test('direct clients without Origin or fetch metadata, and the localhost alias, are forwarded', async () => {
  const direct = await send('GET /attractor/status', [`Host: ${local}`])
  assert.equal(direct.status, 200)
  assert.equal(direct.upstream.headers.origin, undefined)
  const alias = await send('GET /workspace/api/runs', [`Host: localhost:${vitePort}`, `Origin: http://localhost:${vitePort}`])
  assert.equal(alias.status, 200)
  assert.equal(alias.upstream.headers.origin, backendOrigin)
})

test('requests failing the local guard are rejected at Vite', async () => {
  const cases = {
    'other literal': [`Host: 127.0.0.2:${vitePort}`],
    'other family': [`Host: [::1]:${vitePort}`],
    'wrong port': [`Host: 127.0.0.1:${vitePort + 1}`],
    'missing port': ['Host: 127.0.0.1'],
    userinfo: [`Host: user@${local}`],
    'duplicate Host': [`Host: ${local}`, `Host: ${local}`],
    'duplicate Origin': [`Host: ${local}`, `Origin: http://${local}`, `Origin: http://${local}`],
    'duplicate fetch metadata': [`Host: ${local}`, 'Sec-Fetch-Site: same-origin', 'Sec-Fetch-Site: same-origin'],
    'foreign Origin': [`Host: ${local}`, `Origin: http://192.0.2.1:${vitePort}`],
    'DNS Origin for a literal Host': [`Host: ${local}`, `Origin: http://localhost:${vitePort}`],
    'HTTPS Origin': [`Host: ${local}`, `Origin: https://${local}`],
    'Origin with a path': [`Host: ${local}`, `Origin: http://${local}/`],
    'opaque Origin': [`Host: ${local}`, 'Origin: null'],
    'cross-site metadata': [`Host: ${local}`, 'Sec-Fetch-Site: cross-site'],
    'webhook path read': [`Host: ${local}`, 'Origin: https://ui.example'],
  }
  for (const [name, headers] of Object.entries(cases)) {
    const target = name === 'webhook path read' ? 'GET /workspace/api/webhooks' : 'POST /workspace/api/runs'
    const { status, forwarded } = await send(target, headers)
    assert.deepEqual({ name, status, forwarded }, { name, status: 403, forwarded: 0 })
  }
})

test('incoming forwarding headers on a local request reach Spark unchanged', async () => {
  const { status, upstream } = await send('GET /workspace/api/runs', [`Host: ${local}`, 'X-Forwarded-For: 203.0.113.9'])
  assert.equal(status, 200)
  assert.equal(upstream.headers['x-forwarded-for'], '203.0.113.9')
})

test('a bearer request from any client is forwarded with a marker for Spark to authenticate', async () => {
  const { status, upstream } = await send('POST /workspace/api/runs', [
    `Host: 192.0.2.10:${vitePort}`, 'Origin: https://ui.example', 'Sec-Fetch-Site: cross-site',
    'Authorization: Bearer t0ken', 'Connection: upgrade, x-forwarded-for',
  ])
  assert.equal(status, 200)
  assert.equal(`http://${upstream.headers.host}`, backendOrigin)
  assert.equal(upstream.headers.authorization, 'Bearer t0ken')
  assert.equal(upstream.headers.origin, 'https://ui.example')
  assert.equal(upstream.headers['x-forwarded-for'], '127.0.0.1')
  assert.equal(upstream.headers.connection, 'close')
})

test('repeated or combined Authorization is rejected before forwarding', async () => {
  for (const auth of [['Authorization: Bearer a', 'Authorization: Bearer b'], ['Authorization: Bearer a, Bearer b']]) {
    const { status, forwarded } = await send('GET /workspace/api/runs', [`Host: ${local}`, ...auth])
    assert.deepEqual({ status, forwarded }, { status: 400, forwarded: 0 })
  }
})

test('a public webhook POST is forwarded intact for Spark to check its own credentials', async () => {
  const { status, upstream } = await send('POST /workspace/api/webhooks?source=ci', [
    `Host: 192.0.2.10:${vitePort}`, 'Content-Type: application/json',
    'X-Spark-Webhook-Key: key-1', 'X-Spark-Webhook-Secret: secret-1',
  ], '{"event":"push"}')
  assert.equal(status, 200)
  assert.equal(upstream.url, '/workspace/api/webhooks?source=ci')
  assert.equal(upstream.headers['content-type'], 'application/json')
  assert.equal(upstream.headers['x-spark-webhook-key'], 'key-1')
  assert.equal(upstream.headers['x-spark-webhook-secret'], 'secret-1')
  assert.equal(upstream.headers['x-forwarded-for'], '127.0.0.1')
  assert.equal(upstream.body, '{"event":"push"}')
})

test('Spark is proxied over HTTP only', () => {
  const { proxy } = vite.config.server
  assert.deepEqual(Object.keys(proxy), ['/attractor', '/workspace'])
  for (const entry of Object.values(proxy)) assert.ok(!entry.ws && /^https?:/.test(entry.target))
})
