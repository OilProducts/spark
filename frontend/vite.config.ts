import { defineConfig, type ProxyOptions } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import path from "path"
import net from 'node:net'
import type { IncomingMessage } from 'node:http'

const backendUrl = process.env.VITE_BACKEND_URL || 'http://127.0.0.1:8000'
const backend = new URL(backendUrl)

const family = (ip: string) => (net.isIPv6(ip) ? 'ipv6' : 'ipv4')
// BlockList compares canonical IPs, so IPv4-mapped IPv6 equals its IPv4 address.
const sameIp = (a: string, b: string) => {
  const list = new net.BlockList()
  list.addAddress(a, family(a))
  return list.check(b, family(b))
}
const loopback = new net.BlockList()
loopback.addSubnet('127.0.0.0', 8, 'ipv4')
loopback.addAddress('::1', 'ipv6')
const isLoopback = (ip: string) => loopback.check(ip, family(ip))
const backendIsLoopback = backend.hostname === 'localhost' || isLoopback(backend.hostname.replace(/^\[(.*)\]$/, '$1'))

// Every raw field of a header, so duplicates Node would merge or drop stay visible.
const fields = (req: IncomingMessage, name: string) =>
  req.rawHeaders.filter((_, i) => i % 2 === 1 && req.rawHeaders[i - 1].toLowerCase() === name)

// A Host-style authority: a DNS name, IPv4 literal or bracketed IPv6 literal, with HTTP's default port 80.
// `host` keeps the three kinds distinct; IPv6 spelling and DNS casing are normalized.
function authority(value: string) {
  const match = /^(?:\[([0-9a-f:.]+)\]|([a-z0-9.-]+))(?::(\d{1,5}))?$/i.exec(value)
  if (!match || (match[1] !== undefined && !net.isIPv6(match[1]))) return null
  const port = match[3] === undefined ? 80 : Number(match[3])
  if (match[1] !== undefined) return { host: new URL(`http://[${match[1]}]`).hostname, ip: match[1], port }
  const host = match[2].toLowerCase()
  return { host, ip: net.isIPv4(host) ? host : undefined, port }
}

// Spark's tokenless owner rule, applied to the browser's request before Host and Origin are translated.
function isLocalOwnerRequest(req: IncomingMessage) {
  const { remoteAddress = '', localAddress = '', localPort } = req.socket
  const hosts = fields(req, 'host')
  const origins = fields(req, 'origin')
  const sites = fields(req, 'sec-fetch-site')
  const host = hosts.length === 1 ? authority(hosts[0]) : null
  if (!host || host.port !== localPort || !isLoopback(remoteAddress) || origins.length > 1 || sites.length > 1) return false
  const hostNamesSocket = host.ip
    ? sameIp(localAddress, host.ip)
    : host.host === 'localhost' && ['127.0.0.1', '::1'].some((ip) => sameIp(localAddress, ip))
  const origin = origins.length && origins[0].startsWith('http://') ? authority(origins[0].slice(7)) : null
  const originMatches = !origins.length || (origin?.host === host.host && origin.port === host.port)
  return hostNamesSocket && originMatches && (!sites.length || ['same-origin', 'same-site', 'none'].includes(sites[0]))
}

const sparkProxy: ProxyOptions = {
  target: backendUrl,
  changeOrigin: true,
  bypass(req, res) {
    const reject = (status: number, reason: string) => {
      res?.writeHead(status, { 'Content-Type': 'text/plain' }).end(reason)
      return req.url ?? '/' // a string with the response ended stops Vite from forwarding
    }
    const auth = fields(req, 'authorization')
    if (auth.length > 1 || auth.some((value) => value.includes(','))) return reject(400, 'Send one Authorization value.')
    const webhook = req.method === 'POST' && req.url?.split('?')[0] === '/workspace/api/webhooks'
    if (webhook || /^bearer\s+\S/i.test(auth[0] ?? '')) {
      // Spark authenticates these itself; the marker keeps them off its tokenless local path.
      const prior = req.headers['x-forwarded-for']
      const peer = req.socket.remoteAddress ?? 'unknown'
      req.headers['x-forwarded-for'] = prior ? `${prior}, ${peer}` : peer
      // Upstream `Connection: close` lists no headers, so a client cannot mark the marker hop-by-hop.
      req.headers.connection = 'close'
      return
    }
    if (!isLocalOwnerRequest(req)) return reject(403, 'Not a same-origin request from this computer.')
    if (req.headers.origin && backendIsLoopback) req.headers.origin = backend.origin
  },
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  build: {
    // ponytail: a budget just above today's startup bundle (~1,530 kB), so growth past it still warns.
    // The editor, runs, triggers and missions screens stay mounted while hidden, so they load at startup;
    // loading them on first visit is what would bring this under Vite's 500 kB default.
    chunkSizeWarningLimit: 1600,
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    // HTTP only: no Spark route upgrades, so nothing here sets `ws`.
    proxy: {
      '/attractor': sparkProxy,
      '/workspace': sparkProxy,
      // The public descriptor the environment switcher reads; Spark serves it to anyone.
      '/environment': backendUrl,
    }
  }
})
