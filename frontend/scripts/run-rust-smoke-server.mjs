import { existsSync, mkdirSync, rmSync } from 'node:fs'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const frontendRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(frontendRoot, '..')

function parsePort(argv) {
  const portIndex = argv.indexOf('--port')
  const rawPort = portIndex >= 0 ? argv[portIndex + 1] : process.env.SPARK_UI_SMOKE_PORT ?? '4173'
  const port = Number.parseInt(rawPort, 10)
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    throw new Error(`Invalid smoke server port: ${rawPort}`)
  }
  return port
}

const port = parsePort(process.argv.slice(2))
const distDir = path.join(frontendRoot, 'dist')
const indexPath = path.join(distDir, 'index.html')
if (!existsSync(indexPath)) {
  console.error(`frontend/dist/index.html is missing. Run "npm --prefix frontend run build" before "npm --prefix frontend run ui:smoke".`)
  process.exit(1)
}

const tmpRoot = path.join(frontendRoot, '.tmp-ui-smoke')
const dataDir = path.join(tmpRoot, 'spark-home')
const flowsDir = path.join(tmpRoot, 'flows')
rmSync(tmpRoot, { recursive: true, force: true })
mkdirSync(dataDir, { recursive: true })
mkdirSync(flowsDir, { recursive: true })

const cargo = process.platform === 'win32' ? 'cargo.exe' : 'cargo'
const child = spawn(
  cargo,
  [
    'run',
    '--quiet',
    '-p',
    'spark-server',
    '--',
    'serve',
    '--host',
    '127.0.0.1',
    '--port',
    String(port),
    '--data-dir',
    dataDir,
    '--flows-dir',
    flowsDir,
    '--ui-dir',
    distDir,
  ],
  {
    cwd: repoRoot,
    env: {
      ...process.env,
      SPARK_HOME: dataDir,
      SPARK_FLOWS_DIR: flowsDir,
      SPARK_UI_DIR: distDir,
    },
    stdio: 'inherit',
  },
)

let stopping = false
function stopChild(signal) {
  if (stopping) {
    return
  }
  stopping = true
  child.kill(signal)
}

process.on('SIGINT', () => stopChild('SIGINT'))
process.on('SIGTERM', () => stopChild('SIGTERM'))
process.on('exit', () => {
  if (!child.killed) {
    child.kill('SIGTERM')
  }
})

child.on('exit', (code, signal) => {
  if (signal) {
    process.exitCode = stopping ? 0 : 1
  } else {
    process.exitCode = code ?? 1
  }
})
