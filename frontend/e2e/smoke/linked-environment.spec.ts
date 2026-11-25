import { spawn, type ChildProcess } from 'node:child_process'
import { once } from 'node:events'
import { mkdirSync, mkdtempSync, realpathSync, rmSync } from 'node:fs'
import { createServer } from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, request, test, type APIRequestContext } from '@playwright/test'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(repoRoot, process.env.CARGO_TARGET_DIR) : path.join(repoRoot, 'target')

// ponytail: the port is released before the peer binds it, as in playwright.config.ts; a lost race fails loudly.
const freePort = () => new Promise<number>((resolve, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
        const address = server.address()
        server.close(() => (address && typeof address === 'object' ? resolve(address.port) : reject(new Error('no port'))))
    })
})

type LiveEvent = { url: string; type: string }

test('a linked Spark’s workspace loads and updates live through this Spark’s proxy', async ({ page }) => {
    // A second Spark, B, with its own home, beside the smoke server, A; the smoke server built its binary.
    const peerRoot = realpathSync(mkdtempSync(path.join(os.tmpdir(), 'spark-linked-peer-')))
    let peer: ChildProcess | undefined
    let peerApi: APIRequestContext | undefined
    let peerId = ''
    try {
        const port = await freePort()
        const origin = `http://127.0.0.1:${port}`
        const env: NodeJS.ProcessEnv = {
            ...process.env,
            HOME: path.join(peerRoot, 'home'),
            SPARK_HOME: path.join(peerRoot, 'spark-home'),
            GIT_AUTHOR_NAME: 'Spark Smoke',
            GIT_AUTHOR_EMAIL: 'smoke@spark.invalid',
            GIT_COMMITTER_NAME: 'Spark Smoke',
            GIT_COMMITTER_EMAIL: 'smoke@spark.invalid',
            // B needs no agents: a probe of a real CLI runs in its own process group, outlives B and writes into its home.
            SPARK_CLAUDE_CODE_BIN: path.join(peerRoot, 'no-claude'),
            SPARK_CODEX_APP_SERVER_BIN: path.join(peerRoot, 'no-codex'),
        }
        delete env.SPARK_API_BASE_URL
        mkdirSync(env.HOME!, { recursive: true })
        let spawnError: Error | undefined
        peer = spawn(
            path.join(targetDir, 'debug', 'spark-server'),
            ['serve', '--host', '127.0.0.1', '--port', String(port), '--data-dir', env.SPARK_HOME!],
            { cwd: repoRoot, env, stdio: 'inherit' },
        )
        peer.once('error', (error) => { spawnError = error })
        peerApi = await request.newContext({ baseURL: origin })
        const api = peerApi
        await expect.poll(async () => {
            if (spawnError || peer?.exitCode !== null) return `B is not running: ${spawnError?.message ?? `exit ${peer?.exitCode}`}`
            return (await api.get('/environment').catch(() => null))?.status()
        }, { timeout: 60_000 }).toBe(200)
        peerId = (await (await peerApi.get('/environment')).json()).environment_id
        const local = await (await page.request.get('/environment')).json()
        expect(peerId).not.toBe(local.environment_id)

        // Pair A with B, and give B a project A does not have.
        const pairing = await peerApi.post('/workspace/api/environment/pairing-codes', { data: { label: 'Smoke A' } })
        expect(pairing.ok()).toBe(true)
        const linked = await page.request.post('/workspace/api/environments/links', {
            data: { origin, expected_id: peerId, code: (await pairing.json()).code, label: 'Peer B' },
        })
        expect(linked.ok(), await linked.text()).toBe(true)
        const projectPath = path.join(peerRoot, 'peer-project')
        mkdirSync(projectPath)
        expect((await peerApi.post('/workspace/api/projects/register', { data: { project_path: projectPath } })).ok()).toBe(true)

        // The page records each live event it receives, with the stream it came on.
        await page.addInitScript(() => {
            const received: Array<{ url: string; type: string }> = []
            Object.assign(window, { sparkLiveEvents: received })
            const Native = window.EventSource
            window.EventSource = class extends Native {
                constructor(url: string | URL, init?: EventSourceInit) {
                    super(url, init)
                    this.addEventListener('message', (event) => {
                        try {
                            received.push({ url: this.url, type: JSON.parse(event.data).type })
                        } catch {
                            // Not an envelope.
                        }
                    })
                }
            }
        })
        const liveEvents = () => page.evaluate(() => (window as unknown as { sparkLiveEvents: LiveEvent[] }).sparkLiveEvents)
        await page.goto('/')
        await page.getByTestId('activity-chats').click()
        const peerProject = page.locator(`[data-testid="chats-project-group"][data-project-path="${projectPath}"]`)
        await expect(page.getByTestId('chats-project-group').first()).toBeVisible()
        await expect(peerProject).toHaveCount(0)

        // B's tab opens B's workspace, its requests and live events relayed by A.
        const proxy = `/workspace/api/environments/links/${peerId}/proxy`
        const liveStream = `${proxy}/workspace/api/live/events`
        const isTriggerList = (url: string) => new URL(url).pathname === `${proxy}/workspace/api/triggers`
        const triggerLists = { sent: 0, settled: 0 }
        page.on('request', (sent) => { if (sent.method() === 'GET' && isTriggerList(sent.url())) triggerLists.sent += 1 })
        const settled = (done: { url: () => string; method: () => string }) => {
            if (done.method() === 'GET' && isTriggerList(done.url())) triggerLists.settled += 1
        }
        page.on('requestfinished', settled)
        page.on('requestfailed', settled)
        const projectsLoaded = page.waitForResponse((response) => new URL(response.url()).pathname === `${proxy}/workspace/api/projects` && response.ok())
        await page.getByTestId(`environment-tab-${peerId}`).click()
        await expect(page.getByTestId('remote-environment-indicator')).toContainText(origin)
        await projectsLoaded
        // B's workspace opens on its own default view.
        await page.getByTestId('activity-chats').click()
        await expect(peerProject).toBeVisible()

        // B's triggers have loaded, from its live snapshot and any list request, before B changes them.
        await page.getByTestId('activity-triggers').click()
        await expect(page.getByTestId('triggers-panel')).toBeVisible()
        await expect.poll(async () => (await liveEvents()).some((event) => new URL(event.url).pathname === liveStream && event.type === 'trigger.snapshot')).toBe(true)
        await expect.poll(() => triggerLists.settled === triggerLists.sent).toBe(true)
        const listedBefore = triggerLists.sent

        // A trigger made on B shows in A's Triggers view as B's live event, relayed by A, with no list request.
        const created = await peerApi.post('/workspace/api/triggers', {
            data: { project_path: projectPath, name: 'Peer live trigger', source_type: 'webhook', action: { flow_name: 'ops/run' }, source: {} },
        })
        expect(created.ok(), await created.text()).toBe(true)
        await expect(page.getByTestId(`trigger-row-${(await created.json()).id}`)).toContainText('Peer live trigger')
        expect((await liveEvents()).filter((event) => event.type === 'trigger.upsert').map((event) => new URL(event.url).pathname)).toEqual([liveStream])
        expect(triggerLists.sent).toBe(listedBefore)
    } finally {
        try {
            // Later specs share A, so the link goes first; B and its home go whatever happens here.
            if (peerId) expect((await page.request.delete(`/workspace/api/environments/links/${encodeURIComponent(peerId)}`)).ok()).toBe(true)
        } finally {
            await peerApi?.dispose().catch(() => {})
            if (peer?.pid !== undefined && peer.exitCode === null && peer.signalCode === null) {
                const exited = once(peer, 'exit')
                peer.kill('SIGKILL')
                await exited
            }
            rmSync(peerRoot, { recursive: true, force: true, maxRetries: 5 })
        }
    }
})
