import { existsSync, mkdirSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawn, spawnSync } from 'node:child_process'
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
// Home is a project: its org repo is $HOME/.spark/agent-org, so the server gets its own HOME.
const homeDir = path.join(tmpRoot, 'home')
rmSync(tmpRoot, { recursive: true, force: true })
mkdirSync(dataDir, { recursive: true })
mkdirSync(homeDir, { recursive: true })

const cargo = process.platform === 'win32' ? 'cargo.exe' : 'cargo'
// Chat turns run against the repo's fake Codex app-server; the live-turn smoke opens its gates.
const fakeCodex = 'spark-agent-fake-codex-app-server'
const built = spawnSync(cargo, ['build', '--quiet', '-p', 'spark-agent-adapter', '--bin', fakeCodex], { cwd: repoRoot, stdio: 'inherit' })
if (built.status !== 0) {
  process.exit(built.status ?? 1)
}
const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(repoRoot, process.env.CARGO_TARGET_DIR) : path.join(repoRoot, 'target')
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
    '--ui-dir',
    distDir,
  ],
  {
    cwd: repoRoot,
    env: {
      ...process.env,
      HOME: homeDir,
      CARGO_HOME: process.env.CARGO_HOME ?? path.join(os.homedir(), '.cargo'),
      RUSTUP_HOME: process.env.RUSTUP_HOME ?? path.join(os.homedir(), '.rustup'),
      // Editor saves commit to the project's org repo; the isolated HOME has no git identity.
      GIT_AUTHOR_NAME: 'Spark Smoke',
      GIT_AUTHOR_EMAIL: 'smoke@spark.invalid',
      GIT_COMMITTER_NAME: 'Spark Smoke',
      GIT_COMMITTER_EMAIL: 'smoke@spark.invalid',
      SPARK_HOME: dataDir,
      SPARK_UI_DIR: distDir,
      SPARK_CODEX_APP_SERVER_BIN: path.join(targetDir, 'debug', fakeCodex),
      SPARK_FAKE_CODEX_APP_SERVER_MODE: 'gated',
      SPARK_FAKE_CODEX_APP_SERVER_GATE_DIR: path.join(tmpRoot, 'gates'),
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
