import { chooseModel, customModel, chooseEffort, openPicker } from '../fixtures/model-picker'
import { spawn, type ChildProcess } from 'node:child_process'
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:net'
import path from 'node:path'
import { expect, test, type Page } from '@playwright/test'

test.beforeEach(async ({ context }) => {
    // The picker lists only reachable providers, so the fixture reaches the ones these tests choose from.
    // On the context, so second pages a test opens get it too; a test's own page route still wins.
    await context.route('**/workspace/api/projects/chat-models**', route => route.fulfill({
        json: {
            provider_reasoning_efforts: { codex: ['low', 'medium', 'high', 'xhigh'], openai: ['low', 'medium', 'high'], anthropic: ['low', 'medium', 'high'] },
            providers: { codex: { status: 'available', error: null }, openai: { status: 'available', error: null }, anthropic: { status: 'available', error: null } },
            models: [
                { provider: 'openai', id: 'gpt-5.5', display: 'gpt-5.5', is_default: true, default_reasoning_effort: 'medium', supported_reasoning_efforts: ['low', 'medium', 'high'] },
                { provider: 'anthropic', id: 'claude-opus-4-6', display: 'claude-opus-4-6', is_default: true, default_reasoning_effort: 'medium', supported_reasoning_efforts: ['low', 'medium', 'high'] },
            ],
        },
    }))
})

const card = (page: Page, name: string) => page.locator('[data-slot=card]').filter({ has: page.getByRole('heading', { name, exact: true }) })

test('runtime settings Save, Discard, validation, navigation guard, live conflict, and reload', async ({ page, context }) => {
    const read = async () => (await page.request.get('/workspace/api/settings')).json()
    const original = (await read()).runtime.stored
    await page.goto('/')
    await page.getByTestId('activity-settings').click()
        await page.getByRole('tab', { name: 'System', exact: true }).click()
    const input = page.getByLabel('Flows directory', { exact: true })
    await expect(input).toBeVisible()
    const second = await context.newPage()
    try {
        await second.goto('/')
        await second.getByTestId('activity-settings').click()
        await second.getByRole('tab', { name: 'System', exact: true }).click()
        const otherInput = second.getByLabel('Flows directory', { exact: true })
        await expect(otherInput).toBeVisible()
        await input.fill('/tmp/spark-settings-draft')
        expect((await read()).runtime.stored).toEqual(original)
        await page.getByLabel('Project roots (one absolute path per line)').fill('relative')
        await expect(page.getByText('Each project root must be an absolute path.')).toBeVisible()
        await expect(card(page, 'Runtime paths').getByRole('button', { name: /^Save\b/ })).toBeDisabled()
        await card(page, 'Runtime paths').getByRole('button', { name: /^Discard\b/ }).click()
        await expect(input).toHaveValue(original.flows_dir ?? '')
        await input.fill('/tmp/spark-settings-first')
        await page.getByTestId('activity-chats').click()
        await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
        await expect(page.getByTestId('settings-panel')).toBeVisible()
        await expect(input).toHaveValue('/tmp/spark-settings-first')

        await otherInput.fill('/tmp/spark-settings-second')
        await card(second, 'Runtime paths').getByRole('button', { name: /^Save\b/ }).click()
        await expect(second.getByText('Saved. Restart Spark to apply runtime changes.')).toBeVisible()
        await expect(page.getByText('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')).toBeVisible()
        await expect(input).toHaveValue('/tmp/spark-settings-first')
        await card(page, 'Runtime paths').getByRole('button', { name: /^Save\b/ }).click()
        await expect(page.getByText('Settings changed since this document was read. Reload before saving.', { exact: false })).toBeVisible()
        await expect(input).toHaveValue('/tmp/spark-settings-first')
        expect((await read()).runtime.stored.flows_dir).toBe('/tmp/spark-settings-second')
        await card(page, 'Runtime paths').getByRole('button', { name: /^Discard\b/ }).click()
        await expect(input).toHaveValue('/tmp/spark-settings-second')
        await page.reload()
        await page.getByRole('tab', { name: 'System', exact: true }).click()
        await expect(page.getByLabel('Flows directory', { exact: true })).toHaveValue('/tmp/spark-settings-second')
    } finally {
        const current = await read()
        const restored = await page.request.patch('/workspace/api/settings', { data: {
            expected_revision: current.runtime.revision, section: 'runtime', value: original,
        } })
        expect(restored.ok()).toBeTruthy()
        await second.close()
    }
})

test('workspace model defaults require Save and survive reload; dirty drafts survive other clients', async ({ page, context }) => {
    const read = async () => (await page.request.get('/workspace/api/settings')).json()
    const original = (await read()).models.effective
    await page.goto('/')
    await page.getByTestId('activity-settings').click()
    const provider = page.getByRole('button', { name: /^Model:/ })
    await expect(provider).toBeEnabled()
    const other = await context.newPage()
    try {
        await chooseModel(page, 'anthropic', 'claude-opus-4-6')
        expect((await read()).models.effective).toEqual(original)
        await card(page, 'Model defaults (Workspace)').getByRole('button', { name: /^Save\b/ }).click()
        await expect(page.getByText('Saved. Applies to the next message.', { exact: true })).toBeVisible()
        await page.reload()
        await expect(provider).toContainText('claude-opus-4-6')
        await customModel(page, 'retained-draft')
        await other.goto('/')
        await other.getByTestId('activity-settings').click()
        await expect(other.getByRole('button', { name: /^Model:/ })).toBeEnabled()
        await chooseModel(other, 'openai', 'gpt-5.5')
        await card(other, 'Model defaults (Workspace)').getByRole('button', { name: /^Save\b/ }).click()
        await expect(other.getByText('Saved. Applies to the next message.', { exact: true })).toBeVisible()
        await expect(page.getByText('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.', { exact: true })).toBeVisible()
        await expect(provider).toContainText('retained-draft')
        await card(page, 'Model defaults (Workspace)').getByRole('button', { name: /^Save\b/ }).click()
        await expect(page.getByText('Settings changed since this document was read. Reload before saving.', { exact: false })).toBeVisible()
        await expect(provider).toContainText('retained-draft')
        await card(page, 'Model defaults (Workspace)').getByRole('button', { name: /^Discard\b/ }).click()
        await expect(provider).toContainText('gpt-5.5')
    } finally {
        const current = await read()
        const restored = await page.request.patch('/workspace/api/settings', { data: { expected_revision: current.models.revision, section: 'models', value: original } })
        expect(restored.ok()).toBeTruthy()
        await other.close()
    }
})

test('client preferences save explicitly, survive reload, and stay isolated between browser clients', async ({ page, browser }) => {
    const otherContext = await browser.newContext()
    const other = await otherContext.newPage()
    try {
        await page.goto('/')
        await page.getByTestId('activity-settings').click()
        await page.getByRole('tab', { name: 'Preferences', exact: true }).click()
        const width = page.getByLabel('Editor sidebar width (pixels)', { exact: true })
        await expect(width).toBeEnabled()
        const clientId = await page.evaluate(() => localStorage.getItem('spark.client_id'))
        expect(clientId).toBeTruthy()
        await width.fill('400')
        await page.getByLabel('Home sidebar primary split (0–1)', { exact: true }).fill('0.6')
        await page.getByLabel('Show advanced controls', { exact: true }).selectOption('true')
        await page.getByLabel('Expand child flows', { exact: true }).selectOption('true')
        await page.getByLabel('Open graph settings panel', { exact: true }).selectOption('true')
        await card(page, 'Client preferences').getByRole('button', { name: /^Save\b/ }).click()
        await expect(card(page, 'Client preferences').getByText('Saved.', { exact: true })).toBeVisible()
        await page.reload()
        await page.getByRole('tab', { name: 'Preferences', exact: true }).click()
        await expect(width).toHaveValue('400')
        await expect(page.getByLabel('Home sidebar primary split (0–1)', { exact: true })).toHaveValue('0.6')
        await expect(page.getByLabel('Show advanced controls', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Expand child flows', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Open graph settings panel', { exact: true })).toHaveValue('true')
        expect(await page.evaluate(() => localStorage.getItem('spark.client_id'))).toBe(clientId)
        await other.goto(page.url())
        await other.getByTestId('activity-settings').click()
        await other.getByRole('tab', { name: 'Preferences', exact: true }).click()
        const otherWidth = other.getByLabel('Editor sidebar width (pixels)', { exact: true })
        await expect(otherWidth).toBeEnabled()
        await expect(otherWidth).toHaveValue('')
        for (const label of ['Home sidebar primary split (0–1)', 'Show advanced controls', 'Expand child flows', 'Open graph settings panel']) {
            await expect(other.getByLabel(label, { exact: true })).toHaveValue('')
        }
        expect(await other.evaluate(() => localStorage.getItem('spark.client_id'))).not.toBe(clientId)
        await otherWidth.fill('500')
        await card(other, 'Client preferences').getByRole('button', { name: /^Save\b/ }).click()
        await expect(card(other, 'Client preferences').getByText('Saved.', { exact: true })).toBeVisible()
        await page.reload()
        await page.getByRole('tab', { name: 'Preferences', exact: true }).click()
        await expect(width).toHaveValue('400')
        await expect(page.getByLabel('Home sidebar primary split (0–1)', { exact: true })).toHaveValue('0.6')
        await expect(page.getByLabel('Show advanced controls', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Expand child flows', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Open graph settings panel', { exact: true })).toHaveValue('true')
    } finally { await otherContext.close() }
})

test('profile CRUD saves explicitly, retains conflicts, validates mounts, and survives reload', async ({ page }) => {
    const read = async () => (await page.request.get('/workspace/api/settings')).json()
    const initial = await read()
    const id = `settings-e2e-${Date.now()}`
    await page.goto('/')
    await page.getByTestId('activity-settings').click()
    try {
        await page.getByRole('button', { name: 'Add LLM profile', exact: true }).click()
        const profile = page.locator('details').filter({ has: page.locator(`[aria-label="LLM profile ${initial.llm_profiles.stored.length + 1} ID"]`) })
        await profile.getByRole('textbox', { name: `LLM profile ${initial.llm_profiles.stored.length + 1} ID` }).fill(id)
        await profile.getByLabel('Endpoint', { exact: true }).fill('http://localhost:9999/v1')
        await profile.getByLabel('Models (one per line)').fill('test-model')
        await profile.getByLabel('Default model', { exact: true }).selectOption('test-model')
        await profile.getByLabel('Credential environment variable').fill('SPARK_E2E_MISSING_CREDENTIAL')
        expect((await read()).llm_profiles.stored).toEqual(initial.llm_profiles.stored)
        await card(page, 'LLM profiles').getByRole('button', { name: /^Save\b/ }).click()
        await expect(card(page, 'LLM profiles').getByText('Saved. New work uses these settings.')).toBeVisible()
        await page.reload()
        await profile.locator('summary').click()
        await expect(profile.getByLabel('Endpoint', { exact: true })).toHaveValue('http://localhost:9999/v1')
        await profile.getByLabel('Label', { exact: true }).fill('Retained draft')
        let latest = await read()
        const external = await page.request.patch('/attractor/api/llm-profiles', { data: {
            expected_revision: latest.llm_profiles.revision,
            value: latest.llm_profiles.stored.map((entry: { id: string }) => entry.id === id ? { ...entry, label: 'External update' } : entry),
        } })
        expect(external.ok()).toBeTruthy()
        await expect(page.getByText('Profiles changed elsewhere. Your draft is retained; Discard reloads the latest values.')).toBeVisible()
        await card(page, 'LLM profiles').getByRole('button', { name: /^Save\b/ }).click()
        await expect(profile.getByLabel('Label', { exact: true })).toHaveValue('Retained draft')
        await expect(page.getByText('Settings changed since this document was read. Reload before saving.', { exact: false })).toBeVisible()
        await card(page, 'LLM profiles').getByRole('button', { name: /^Discard\b/ }).click()
        await expect(profile.getByLabel('Label', { exact: true })).toHaveValue('External update')
        await page.getByRole('button', { name: `Delete LLM profile ${id}`, exact: true }).click()
        await card(page, 'LLM profiles').getByRole('button', { name: /^Save\b/ }).click()
        await expect.poll(async () => (await read()).llm_profiles.stored.some((entry: { id: string }) => entry.id === id)).toBe(false)

        await page.getByRole('tab', { name: 'Execution', exact: true }).click()
        await page.getByRole('button', { name: 'Add execution profile', exact: true }).click()
        const execution = page.locator('[data-slot=card]').filter({ has: page.getByRole('heading', { name: 'Execution profiles', exact: true }) }).locator(':scope > [data-slot=card-content] > fieldset > details').nth(initial.execution_profiles.stored.profiles.length)
        await execution.getByLabel('Profile ID', { exact: true }).fill(id)
        await execution.getByLabel('Label', { exact: true }).fill('E2E container')
        await execution.getByLabel('Mode', { exact: true }).selectOption('local_container')
        await expect(card(page, 'Execution profiles').getByRole('button', { name: /^Save\b/ })).toBeDisabled()
        await execution.getByLabel('Container image').fill('worker:test')
        await execution.getByText('Advanced', { exact: true }).click()
        await execution.getByLabel('Capabilities (one per line)').fill('shell\nnetwork')
        await execution.getByLabel('Mounts (host:container[:options], one per line)').fill('invalid-mount')
        await card(page, 'Execution profiles').getByRole('button', { name: /^Save\b/ }).click()
        await expect(page.getByText('Mounts must be an array of nonempty host:container[:options] strings.', { exact: false })).toBeVisible()
        await execution.getByLabel('Mounts (host:container[:options], one per line)').fill('/tmp:/workspace:ro')
        await card(page, 'Execution profiles').getByRole('button', { name: /^Save\b/ }).click()
        await expect.poll(async () => (await read()).execution_profiles.stored.profiles.some((entry: { id: string }) => entry.id === id)).toBe(true)
        await page.reload()
        await page.getByRole('tab', { name: 'Execution', exact: true }).click()
        await execution.locator(':scope > summary').click()
        await expect(execution.getByLabel('Container image')).toHaveValue('worker:test')
        latest = await read()
        expect(latest.execution_profiles.stored.profiles.find((entry: { id: string }) => entry.id === id).metadata['container.mounts']).toEqual(['/tmp:/workspace:ro'])
    } finally {
        let latest = await read()
        expect((await page.request.patch('/workspace/api/settings', { data: { section: 'llm_profiles', expected_revision: latest.llm_profiles.revision, value: initial.llm_profiles.stored } })).ok()).toBeTruthy()
        latest = await read()
        expect((await page.request.patch('/workspace/api/settings', { data: { section: 'execution_profiles', expected_revision: latest.execution_profiles.revision, value: initial.execution_profiles.stored } })).ok()).toBeTruthy()
    }
})

test('connection settings validate, save, preserve running binding, and reload', async ({ page }) => {
    const read = async () => (await page.request.get('/workspace/api/settings')).json()
    const original = (await read()).connections
    await page.goto('/')
    await page.getByTestId('activity-settings').click()
        await page.getByRole('tab', { name: 'System', exact: true }).click()
    const port = page.getByLabel('Server port', { exact: true })
    const target = page.getByLabel('Client API target', { exact: true })
    await expect(port).toBeVisible()
    try {
        await port.fill('70000')
        await expect(card(page, 'Server and client connections').getByRole('button', { name: /^Save\b/ })).toBeDisabled()
        await port.fill('8123')
        await target.fill('https://spark.example')
        expect((await read()).connections.stored).toEqual(original.stored)
        await card(page, 'Server and client connections').getByRole('button', { name: /^Save\b/ }).click()
        await expect(page.getByText('Saved. Restart Spark to apply server binding changes.', { exact: true })).toBeVisible()
        const saved = (await read()).connections
        expect(saved.stored.server_port).toBe(8123)
        expect(saved.effective.server_port).toBe(original.effective.server_port)
        await page.reload()
        await page.getByRole('tab', { name: 'System', exact: true }).click()
        await expect(port).toHaveValue('8123')
        await expect(target).toHaveValue('https://spark.example')
        await target.fill('https://discard.example')
        await card(page, 'Server and client connections').getByRole('button', { name: /^Discard\b/ }).click()
        await expect(target).toHaveValue('https://spark.example')
    } finally {
        const current = await read()
        expect((await page.request.patch('/workspace/api/settings', { data: {
            expected_revision: current.connections.revision, section: 'connections', value: original.stored,
        } })).ok()).toBeTruthy()
    }
})

test('provider and agent sections validate, save, report references and restart requirements, and survive reload', async ({ page }) => {
    const read = async () => (await page.request.get('/workspace/api/settings')).json()
    const original = await read()
    await page.goto('/')
    await page.getByTestId('activity-settings').click()
    await page.locator('summary').filter({ hasText: /^OpenAI ·/ }).click()
    const group = page.getByRole('group', { name: 'OpenAI', exact: true })
    const endpoint = group.getByLabel('Base URL', { exact: true })
    await expect(endpoint).toBeVisible()
    try {
        await endpoint.fill('https://user:SECRET@example.com')
        await expect(card(page, 'Provider connections').getByRole('button', { name: /^Save\b/ })).toBeDisabled()
        await endpoint.fill('https://example.com/v1')
        await group.getByLabel('Credential environment variable').fill('CR_SETTINGS_TEST_REFERENCE')
        expect((await read()).providers.stored).toEqual(original.providers.stored)
        await card(page, 'Provider connections').getByRole('button', { name: /^Save\b/ }).click()
        await expect.poll(async () => (await read()).providers.stored.openai.api_key_env).toBe('CR_SETTINGS_TEST_REFERENCE')
        await expect(group.getByText(/Effective:.*(Configured|Missing)/)).toBeVisible()
        // Both sections share a document revision; load the new revision before the next edit.
        await page.reload()
        await page.getByRole('tab', { name: 'Execution', exact: true }).click()
        await page.locator('summary').filter({ hasText: /^Runtime paths$/ }).click()
        const timeout = page.getByLabel('Default command timeout (milliseconds)', { exact: true })
        await timeout.fill('1234')
        await page.getByLabel('Codex runtime root', { exact: true }).fill('/tmp/spark-cr-native-home')
        await expect(page.getByText(/Effective:.*Requires restart/).first()).toBeVisible()
        await card(page, 'Agent session limits').getByRole('button', { name: /^Save\b/ }).click()
        await expect.poll(async () => (await read()).agents.stored.default_command_timeout_ms).toBe(1234)
        const saved = await read()
        expect(saved.agents.effective.native.codex_runtime_root).toEqual(original.agents.effective.native.codex_runtime_root)
        expect(saved.providers.stored.openai.api_key_env).toBe('CR_SETTINGS_TEST_REFERENCE')
        expect(typeof saved.providers.credential_status.openai).toBe('boolean')
        expect(JSON.stringify(saved)).not.toContain('SECRET')
        await page.reload()
        await page.locator('summary').filter({ hasText: /^OpenAI ·/ }).click()
        await expect(endpoint).toHaveValue('https://example.com/v1')
        await page.getByRole('tab', { name: 'Execution', exact: true }).click()
        await expect(timeout).toHaveValue('1234')
    } finally {
        for (const section of ['providers', 'agents']) {
            const current = await read()
            const response = await page.request.patch('/workspace/api/settings', { data: { section, expected_revision: current[section].revision, value: original[section].stored } })
            expect(response.ok()).toBeTruthy()
        }
    }
})

test('legacy graph positions migrate once into the current client while caches remain local', async ({ page }) => {
    const legacy = 'spark.saved_flow_layout.v1:/legacy:flow:editor-parent-only'
    await page.addInitScript(({ legacy }) => {
        if (!localStorage.getItem('spark.client_id')) {
            localStorage.setItem('spark.client_id', 'browser-layout-migration-e2e')
            localStorage.setItem(legacy, JSON.stringify({ version: 1, topologyStamp: 'cache-only', nodePositions: { task: { x: 42, y: 84 } }, edgeLayouts: {} }))
        }
    }, { legacy })
    await page.goto('/')
    const read = async () => (await page.request.get('/workspace/api/settings?client_id=browser-layout-migration-e2e')).json()
    await expect.poll(async () => (await read()).preferences.browser_migration_version).toBe(1)
    const view = (await read()).preferences
    expect(view.stored.flow_node_positions['/legacy:flow:editor-parent-only']).toEqual({ task: { x: 42, y: 84 } })
    expect(JSON.stringify(view.stored)).not.toContain('cache-only')
    await expect.poll(() => page.evaluate((key) => localStorage.getItem(key), legacy)).toBeNull()
    expect(await page.evaluate((key) => localStorage.getItem(`${key}.v0.bak`), legacy)).not.toBeNull()
    await page.reload()
    expect((await read()).preferences.revision).toBe(view.revision)
})


test('saved core and Desktop-identity preferences survive a real server restart on a different port', async ({ page }, testInfo) => {
    test.setTimeout(60000)
    const home = testInfo.outputPath('restart-home')
    mkdirSync(home, { recursive: true })
    const repo = path.resolve(process.cwd(), '..')
    const ports = new Set<number>()
    let child: ChildProcess | null = null
    const stop = async () => {
        if (!child || child.exitCode !== null || child.signalCode !== null) return
        const stopped = new Promise<void>((resolve) => child!.once('exit', () => resolve()))
        child.kill('SIGTERM')
        await stopped
    }
    const start = async () => {
        let port: number
        do {
            const listener = createServer()
            await new Promise<void>((resolve) => listener.listen(0, '127.0.0.1', resolve))
            port = (listener.address() as { port: number }).port
            await new Promise<void>((resolve) => listener.close(() => resolve()))
        } while (ports.has(port))
        ports.add(port)
        const url = `http://127.0.0.1:${port}`
        child = spawn(path.join(repo, 'target/debug/spark-server'), ['serve', '--host', '127.0.0.1', '--port', String(port), '--data-dir', home, '--ui-dir', path.join(repo, 'frontend/dist')], {
            cwd: repo, stdio: 'ignore', env: { ...process.env, SPARK_HOME: home, SPARK_UI_DIR: path.join(repo, 'frontend/dist'), SPARK_FLOWS_DIR: '', ATTRACTOR_CODEX_RUNTIME_ROOT: '' },
        })
        await expect.poll(async () => { try { return (await fetch(`${url}/workspace/api/settings`)).status } catch { return 0 } }, { timeout: 20000 }).toBe(200)
        return url
    }
    // Exercise the browser/native boundary with the same platform-owned identity on both origins.
    // Rust Desktop contracts cover creation and persistence of that native identity.
    await page.addInitScript(() => {
        window.__TAURI__ = { core: { invoke: async <T>(command: string): Promise<T> => {
            if (command === 'desktop_client_identity') return 'desktop-restart-e2e' as T
            throw new Error('This browser fixture provides only Desktop identity.')
        } } }
    })
    try {
        const first = await start()
        await page.goto(first)
        await page.getByTestId('activity-settings').click()
        await page.getByRole('tab', { name: 'Preferences', exact: true }).click()
        const width = page.getByLabel('Editor sidebar width (pixels)', { exact: true })
        await width.fill('420')
        await card(page, 'Client preferences').getByRole('button', { name: /^Save\b/ }).click()
        await expect(card(page, 'Client preferences').getByText('Saved.', { exact: true })).toBeVisible()
        const nativeHome = path.join(home, 'saved-agent-home')
        await page.getByRole('tab', { name: 'Execution', exact: true }).click()
        await page.locator('summary').filter({ hasText: /^Runtime paths$/ }).click()
        await page.getByLabel('Codex runtime root', { exact: true }).fill(nativeHome)
        await card(page, 'Agent session limits').getByRole('button', { name: /^Save\b/ }).click()
        await expect.poll(async () => (await (await fetch(`${first}/workspace/api/settings`)).json()).agents.stored.native.codex_runtime_root).toBe(nativeHome)
        await page.goto('about:blank')
        await stop()
        const second = await start()
        expect(second).not.toBe(first)
        await page.goto(second)
        await page.getByTestId('activity-settings').click()
        await page.getByRole('tab', { name: 'Preferences', exact: true }).click()
        await expect(page.getByLabel('Editor sidebar width (pixels)', { exact: true })).toHaveValue('420')
        await page.getByRole('tab', { name: 'Execution', exact: true }).click()
        await page.locator('summary').filter({ hasText: /^Runtime paths$/ }).click()
        await expect(page.getByLabel('Codex runtime root', { exact: true })).toHaveValue(nativeHome)
        const settings = await (await fetch(`${second}/workspace/api/settings`)).json()
        expect(settings.agents.effective.native.codex_runtime_root).toBe(nativeHome)
        expect(await page.evaluate(() => localStorage.getItem('spark.client_id'))).toBeNull()
    } finally { await page.goto('about:blank'); await stop() }
})

const chatsGroup = (page: Page, project: string) => page.locator(`[data-testid="chats-project-group"][data-project-path="${project}"]`)
const openChat = async (page: Page, project: string, title: string) => {
    const group = chatsGroup(page, project)
    if (await group.getByTestId('chats-project-toggle').getAttribute('aria-expanded') !== 'true') await group.getByTestId('chats-project-toggle').click()
    await group.getByRole('button', { name: new RegExp(`Open thread ${title}`) }).click()
}
const openProjectPage = async (page: Page, project: string) => {
    await page.getByTestId('activity-chats').click()
    await chatsGroup(page, project).getByTestId('chats-project-name').click()
    await expect(page.getByTestId('project-page-path')).toHaveText(project)
}

for (const transition of ['switch', 'chat']) {
    test(`leaving a project page by ${transition} confirms draft loss and cancellation preserves the editor`, async ({ page }, testInfo) => {
        const projects = [testInfo.outputPath('project-one'), testInfo.outputPath('project-two')]
        for (const project of projects) {
            mkdirSync(project, { recursive: true })
            expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
        }
        const chatId = `conversation-draft-guard-${Date.now()}`
        expect((await page.request.put(`/workspace/api/conversations/${chatId}/settings`, { data: { project_path: projects[1], expected_revision: '0' } })).ok()).toBeTruthy()
        await page.goto('/')
        await openProjectPage(page, projects[0])
        const projectCard = page.getByTestId('project-page').locator('[data-slot=card]').filter({ has: page.getByRole('heading', { name: 'Project model defaults', exact: true }) })
        await projectCard.getByRole('switch', { name: 'Override workspace model settings' }).click()
        await customModel(page, 'unsaved-project-model', projectCard)
        const model = projectCard.getByRole('button', { name: /^Model:/ })
        const navigate = async () => {
            if (transition === 'switch') {
                await chatsGroup(page, projects[1]).getByTestId('chats-project-name').click()
                return
            }
            const group = chatsGroup(page, projects[1])
            if (await group.getByTestId('chats-project-toggle').getAttribute('aria-expanded') !== 'true') await group.getByTestId('chats-project-toggle').click()
            await group.getByTestId('chats-chat-row').first().click()
        }
        await navigate()
        await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
        await expect(model).toContainText('unsaved-project-model')
        await expect(page.getByTestId('project-page-path')).toHaveText(projects[0])
        // Other views keep the page mounted, so visiting them keeps the draft without asking.
        await page.getByTestId('activity-runs').click()
        await page.getByTestId('activity-chats').click()
        await expect(model).toContainText('unsaved-project-model')
        const read = async () => (await page.request.get(`/workspace/api/settings?project_path=${encodeURIComponent(projects[0])}`)).json()
        expect((await read()).models.stored).toBeNull()
        await navigate()
        await page.getByRole('button', { name: 'Discard and leave', exact: true }).click()
        if (transition === 'switch') await expect(page.getByTestId('project-page-path')).toHaveText(projects[1])
        else await expect(page.getByTestId('chat-composer-project')).toContainText('project-two')
        await expect(page.getByText('unsaved-project-model')).toHaveCount(0)
        expect((await read()).models.stored).toBeNull()
    })
}

test('project page execution settings protect leaving and handle clean and dirty external updates', async ({ page }, testInfo) => {
    const defaults = (await (await page.request.get('/workspace/api/settings')).json()).execution_profiles
    expect((await page.request.patch('/workspace/api/settings', { data: { section: 'execution_profiles', expected_revision: defaults.revision, value: defaults.stored } })).ok()).toBeTruthy()
    const project = testInfo.outputPath('execution-project')
    mkdirSync(project, { recursive: true })
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
    await page.goto('/')
    await openProjectPage(page, project)
    const section = page.getByTestId('project-settings-dialog')
    const select = page.getByTestId('project-default-execution-profile')
    await expect(select).toBeEnabled()
    const read = async () => (await page.request.get(`/workspace/api/settings?project_path=${encodeURIComponent(project)}`)).json()
    const external = async (value: string | null) => {
        const current = await read()
        expect((await page.request.patch('/workspace/api/projects/state', { data: {
            project_path: project, expected_revision: current.execution.revision, execution_profile_id: value,
        } })).ok()).toBeTruthy()
    }
    await external('native')
    await expect(select).toContainText(/native/i)
    await select.click()
    await page.getByRole('option', { name: 'Use workspace default', exact: true }).click()
    const home = (await (await page.request.get('/workspace/api/projects')).json()).find((entry: { is_default: boolean }) => entry.is_default)
    await chatsGroup(page, home.project_path).getByTestId('chats-project-name').click()
    await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
    await expect(select).toContainText('Use workspace default')
    await expect(section).toBeVisible()
    await external(null)
    await expect(page.getByText('Project execution settings changed elsewhere. Your draft is retained; Discard reloads the latest values.', { exact: true })).toBeVisible()
    await expect(select).toContainText('Use workspace default')
    await page.getByTestId('project-settings-save-button').click()
    await expect(page.getByTestId('project-settings-save-error')).toContainText('Settings changed')
    await expect(select).toContainText('Use workspace default')
    await page.getByRole('button', { name: 'Discard project settings changes', exact: true }).click()
    await expect(page.getByTestId('project-settings-save-error')).toHaveCount(0)
    await expect(page.getByTestId('project-settings-save-button')).toBeDisabled()
    await select.click()
    await page.getByRole('option').filter({ hasText: /native/i }).click()
    await chatsGroup(page, home.project_path).getByTestId('chats-project-name').click()
    await page.getByRole('button', { name: 'Discard and leave', exact: true }).click()
    await expect(page.getByTestId('project-page-path')).toHaveText(home.project_path)
    expect((await read()).execution.stored).toBeNull()
})

test('scoped configuration links open existing flow and trigger editors with navigation protection', async ({ page }) => {
    await page.goto('/')
    await page.getByTestId('activity-settings').click()
    await page.getByRole('tab', { name: 'System', exact: true }).click()
    const draft = page.getByLabel('Flows directory', { exact: true })
    await draft.fill('/tmp/scoped-link-draft')
    await page.getByRole('tab', { name: 'Execution', exact: true }).click()
    await page.getByRole('button', { name: 'Open flow editor', exact: true }).click()
    await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
    await expect(draft).toHaveValue('/tmp/scoped-link-draft')
    await page.getByRole('button', { name: 'Open flow editor', exact: true }).click()
    await page.getByRole('button', { name: 'Discard and leave', exact: true }).click()
    await expect(page.getByTestId('settings-panel')).toHaveCount(0)
    await expect(page.getByTestId('editor-no-flow-state')).toBeVisible()
    await page.getByTestId('activity-settings').click()
    await page.getByRole('tab', { name: 'Execution', exact: true }).click()
    await page.getByRole('button', { name: 'Edit triggers', exact: true }).click()
    await expect(page.getByTestId('triggers-panel')).toBeVisible()
})

test('conversation effort and model edits preserve an inherited profile; selection and reset work without a model call', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('profile-conversation-project')
    mkdirSync(project, { recursive: true })
    const read = async () => (await page.request.get('/workspace/api/settings')).json()
    const original = await read()
    const save = async (section: string, value: unknown) => {
        const current = await read()
        const response = await page.request.patch('/workspace/api/settings', {data:{section, value, expected_revision:current[section].revision}})
        expect(response.ok(), await response.text()).toBeTruthy()
    }
    const id = 'cr0118-browser-profile'
    const conversationPath = '/workspace/api/conversations/cr0118-browser-conversation'
    const conversation = async () => (await page.request.get(`${conversationPath}?project_path=${encodeURIComponent(project)}`)).json()
    try {
        expect((await page.request.post('/workspace/api/projects/register', {data:{project_path:project}})).ok()).toBeTruthy()
        await save('llm_profiles', [...original.llm_profiles.stored, {id, provider:'openai_compatible', base_url:'http://127.0.0.1:1', models:['model-one','model-two'], default_model:'model-one', reasoning_efforts:['low','medium','high']}])
        await save('models', {llm_profile:id, model:null, reasoning_effort:'low'})
        const created = await page.request.put(`${conversationPath}/settings`, {data:{project_path:project, expected_revision:'0', model_settings:null}})
        expect(created.ok(), await created.text()).toBeTruthy()
        const title = (await created.json()).title as string
        await page.goto('/')
        await openChat(page, project, title)
        const provider = page.getByRole('button', { name: /^Model:/ })
        await expect(provider).toContainText('model-one')
        // An inherited choice has no effort of its own; choosing a model comes first.
        await openPicker(page)
        await expect(page.getByRole('group', { name: 'Reasoning effort' }).getByRole('button', { name: 'High', exact: true })).toBeDisabled()
        await chooseModel(page, `openai_compatible / ${id}`, 'model-two')
        await expect.poll(async () => (await conversation()).settings.models.stored).toMatchObject({provider:null,llm_profile:id,model:'model-two',reasoning_effort:null})
        await chooseEffort(page, 'High')
        await expect.poll(async () => (await conversation()).settings.models.stored).toMatchObject({provider:null,llm_profile:id,model:'model-two',reasoning_effort:'high'})
        await chooseModel(page, 'anthropic', 'claude-opus-4-6')
        await expect.poll(async () => (await conversation()).settings.models.stored.provider).toBe('anthropic')
        await expect(provider).toContainText('claude-opus-4-6 · High')
        await openPicker(page)
        await expect(page.getByRole('button', { name: /^Use default ·/ })).toHaveText('Use default · model-one · Low')
        let releaseReset!: () => void
        const resetGate = new Promise<void>(resolve => { releaseReset = resolve })
        await page.route(`**${conversationPath}/settings`, async route => { await resetGate; await route.continue() })
        await page.getByRole('button', { name: /^Use default ·/ }).click()
        try {
            await expect(provider).toHaveText('Default: model-one · Low⌄')
            expect((await conversation()).settings.models.stored.provider).toBe('anthropic')
            await openPicker(page)
            await expect(page.getByRole('group', { name: 'Reasoning effort' }).getByRole('button', { name: 'High', exact: true })).toBeDisabled()
            await page.keyboard.press('Escape')
        } finally { releaseReset() }
        await expect.poll(async () => (await conversation()).settings.models.stored).toBeNull()
        await expect(provider).toContainText('model-one · Low')
        expect((await conversation()).turns).toEqual([])
    } finally {
        const current = await conversation()
        if (current.settings?.models) await page.request.put(`${conversationPath}/settings`, {data:{project_path:project,expected_revision:current.settings.models.revision,model_settings:null}})
        await save('models', original.models.effective)
        await save('llm_profiles', original.llm_profiles.stored)
    }
})


test('malformed project execution selection is repairable with explicit Save and revision protection', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('malformed-execution-project')
    mkdirSync(project, { recursive: true })
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
    // Listing a project's chats, missions or attention items rewrites a project file it finds
    // malformed; this test repairs it through the project page alone, so the page lists nothing.
    await page.route('**/workspace/api/projects/conversations**', route => route.fulfill({ json: [] }))
    await page.route(url => url.pathname === '/workspace/api/missions', route => route.fulfill({ json: { missions: [] } }))
    await page.route('**/workspace/api/attention', route => route.fulfill({ json: { items: [] } }))
    await page.goto('/')
    const home = process.env.SPARK_SETTINGS_TEST_HOME ?? path.resolve('.tmp-ui-smoke/spark-home')
    const directory = path.join(home, 'workspace/projects')
    const file = readdirSync(directory).map(id => path.join(directory, id, 'project.toml'))
        .find(file => readFileSync(file, 'utf8').includes(project))
    expect(file).toBeDefined()
    const original = readFileSync(file!, 'utf8')
    const malformed = original + '\nexecution_profile_id = 42\n[repair_metadata]\nnote = "preserve me"\n'
    writeFileSync(file!, malformed)
    const read = async () => (await page.request.get('/workspace/api/settings?project_path=' + encodeURIComponent(project))).json()
    const initial = await read()
    expect(initial.execution.stored).toBe(42)
    expect(initial.execution.effective).toBeNull()
    await openProjectPage(page, project)
    const select = page.getByTestId('project-default-execution-profile')
    const save = page.getByTestId('project-settings-save-button')
    await expect(page.getByTestId('project-settings-error')).toBeVisible()
    await expect(select).toBeEnabled()
    await expect(select).toContainText('Select a replacement or workspace default')
    await expect(save).toBeDisabled()
    await select.click()
    await page.getByRole('option').filter({ hasText: /native/i }).click()
    expect(readFileSync(file!, 'utf8')).toBe(malformed)
    await save.click()
    await expect.poll(async () => (await read()).execution.stored).toBe('native')
    await expect(save).toBeDisabled()
    expect(readFileSync(file!, 'utf8')).toContain('note = "preserve me"')
    const repaired = readFileSync(file!, 'utf8')
    const stale = await page.request.patch('/workspace/api/projects/state', { data: {
        project_path: project, expected_revision: initial.execution.revision, execution_profile_id: null,
    } })
    expect(stale.status()).toBe(409)
    expect(readFileSync(file!, 'utf8')).toBe(repaired)
    expect((await read()).execution.stored).toBe('native')

    // Exercise reset from the same malformed stored selection, also without an implicit write.
    writeFileSync(file!, malformed)
    // A settings change elsewhere refetches the clean section.
    await page.evaluate(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await expect(select).toContainText('Select a replacement or workspace default')
    await select.click()
    await page.getByRole('option', { name: 'Use workspace default', exact: true }).click()
    expect(readFileSync(file!, 'utf8')).toBe(malformed)
    await save.click()
    await expect.poll(async () => (await read()).execution.stored).toBeNull()
    expect(readFileSync(file!, 'utf8')).toContain('note = "preserve me"')
})

for (const width of [1440, 390]) {
    test(`Settings categories retain drafts and support keyboard access without overflow at ${width}px`, async ({ page }, testInfo) => {
        await page.setViewportSize({ width, height: 900 })
        await page.goto('/')
        await page.getByTestId('activity-settings').click()
        const panel = page.getByTestId('settings-panel')
        const models = page.getByRole('tab', { name: 'Models & accounts' })
        await expect(models).toHaveAttribute('aria-selected', 'true')
        await expect(models).toHaveCSS('white-space', 'nowrap')
        if (width === 390) expect(await page.getByRole('tablist', { name: 'Settings categories' }).evaluate((element) => element.parentElement!.scrollWidth > element.parentElement!.clientWidth)).toBe(true)
        await models.focus()
        await page.keyboard.press('ArrowRight')
        const preferences = page.getByRole('tab', { name: 'Preferences', exact: true })
        await expect(preferences).toBeFocused()
        await expect(page.getByRole('heading', { name: 'Client preferences' })).toBeVisible()
        const sidebar = page.getByLabel('Editor sidebar width (pixels)', { exact: true })
        await sidebar.fill('12')
        await page.getByRole('tab', { name: 'System', exact: true }).click()
        await expect(sidebar).toBeHidden()
        await page.getByTestId('activity-chats').click()
        await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
        await preferences.click()
        await expect(sidebar).toHaveValue('12')
        await expect(page.getByText('Choose a whole number from 256 to 560.')).toBeVisible()
        await card(page, 'Client preferences').getByRole('button', { name: /^Discard\b/ }).click()
        for (const category of ['Models & accounts', 'Preferences', 'Execution', 'System']) {
            await page.getByRole('tab', { name: category, exact: true }).click()
            await expect(page.getByRole('tabpanel')).toHaveCount(1)
            await expect.poll(() => panel.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
            const active = page.getByRole('tabpanel')
            for (const heading of await active.getByRole('heading').all()) await expect(heading).toBeVisible()
            // The only horizontal scrolling is the category strip; card content and actions fit.
            for (const card of await active.locator('[data-slot=card]').all()) {
                expect(await card.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
            }
            await panel.evaluate((element) => { element.scrollTop = 0 })
            await page.screenshot({ path: testInfo.outputPath(`${width}-${category.replaceAll(/[^a-z]/gi, '-')}.png`) })
            for (const summary of await active.locator('details > summary').all()) {
                if (!await summary.evaluate((element) => (element.parentElement as HTMLDetailsElement).open)) await summary.click()
            }
            for (const card of await active.locator('[data-slot=card]').all()) {
                expect(await card.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
            }
            await panel.evaluate((element) => { element.scrollTop = 0 })
            await page.screenshot({ path: testInfo.outputPath(`${width}-${category.replaceAll(/[^a-z]/gi, '-')}-expanded.png`) })
        }
        await models.click()
        const providerSummary = page.locator('summary').filter({ hasText: /^OpenAI ·/ })
        if (await providerSummary.evaluate((element) => (element.parentElement as HTMLDetailsElement).open)) await providerSummary.click()
        await providerSummary.focus()
        await page.keyboard.press('Enter')
        const endpoint = page.getByRole('group', { name: 'OpenAI', exact: true }).getByLabel('Base URL', { exact: true })
        await expect(endpoint).toBeVisible()
        await page.keyboard.press('Tab')
        await expect(endpoint).toBeFocused()
        await page.getByRole('tab', { name: 'Preferences', exact: true }).click()
        await page.getByTestId('activity-chats').click()
        await page.getByTestId('activity-settings').click()
        await expect(models).toHaveAttribute('aria-selected', 'true')
    })
}


test('chat coalesces rapid custom model edits against real backend revisions', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('chat-model-project')
    mkdirSync(project, { recursive: true })
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
    const conversationPath = '/workspace/api/conversations/cr0125-browser-chat'
    const initial = { provider: 'codex', llm_profile: null, model: null, reasoning_effort: 'low' }
    const created = await page.request.put(`${conversationPath}/settings`, { data: {
        project_path: project, expected_revision: '0', model_settings: initial,
    } })
    expect(created.ok()).toBeTruthy()
    const snapshot = await created.json()
    const read = async () => (await page.request.get(`${conversationPath}?project_path=${encodeURIComponent(project)}`)).json()
    await page.goto('/')
    await openChat(page, project, snapshot.title)
    const requests: { expected_revision: string; model_settings: unknown }[] = []
    const statuses: number[] = []
    const releases: (() => void)[] = []
    // Forward every save to Rust unchanged; hold only its response to force overlapping edits.
    await page.route(`**${conversationPath}/settings`, async (route) => {
        requests.push(route.request().postDataJSON())
        const response = await route.fetch()
        statuses.push(response.status())
        await new Promise<void>((resolve) => releases.push(resolve))
        await route.fulfill({ response })
    })
    const release = async () => {
        await expect.poll(() => releases.length).toBe(1)
        const response = page.waitForResponse((response) => response.url().endsWith(`${conversationPath}/settings`))
        releases.shift()!()
        await (await response).finished()
    }
    const model = page.getByRole('button', { name: /^Model:/ })
    await openPicker(page)
    await page.getByRole('combobox', { name: 'Search models' }).fill('my-model')
    expect(requests).toHaveLength(0)
    await page.getByRole('option', { name: 'Use "my-model" as a custom model' }).click()
    await page.keyboard.press('Escape')
    await expect(model).toContainText('my-model')
    await release()
    await expect.poll(async () => (await read()).settings.models.stored.model).toBe('my-model')
    await expect(page.getByRole('button', { name: 'Use defaults', exact: true })).toBeEnabled()

    await chooseModel(page, 'anthropic', 'claude-opus-4-6')
    await customModel(page, 'final-model')
    await chooseEffort(page, 'High')
    expect(requests).toHaveLength(2)
    await release()
    await expect.poll(() => requests.length).toBe(3)
    await release()
    await expect(page.getByRole('button', { name: 'Use defaults', exact: true })).toBeEnabled()
    const final = { provider: 'anthropic', llm_profile: null, model: 'final-model', reasoning_effort: 'high' }
    await expect.poll(async () => (await read()).settings.models.stored).toMatchObject(final)
    expect(statuses).toEqual([200, 200, 200])
    expect(requests.map((request) => request.expected_revision)).toEqual(
        Array.from({ length: 3 }, (_, index) => String(snapshot.revision + index)),
    )
    await page.reload()
    await expect(model).toContainText('final-model · High')
})

test('picker default persists workspace and project resets', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('picker-reset-project')
    mkdirSync(project, { recursive: true })
    const read = async (scoped = false) => (await page.request.get('/workspace/api/settings' + (scoped ? `?project_path=${encodeURIComponent(project)}` : ''))).json()
    const original = (await read()).models
    await page.route('**/workspace/api/projects/chat-models**', route => route.fulfill({ json: {
        provider_reasoning_efforts: { codex: ['low', 'medium', 'high', 'xhigh'], openai: ['low', 'medium', 'high'], anthropic: ['low', 'medium', 'high'] },
        providers: { codex: { status: 'available', error: null }, anthropic: { status: 'available', error: null } },
        models: [{ provider: 'codex', id: 'discovery-default', display: 'Discovery default', is_default: true, default_reasoning_effort: 'medium', supported_reasoning_efforts: ['low', 'medium', 'high'] }, { provider: 'anthropic', id: 'claude-opus-4-6', display: 'claude-opus-4-6', is_default: true, default_reasoning_effort: 'medium', supported_reasoning_efforts: ['low', 'medium', 'high'] }],
    } }))
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
    try {
        await page.goto('/')
        await page.getByTestId('activity-settings').click()
        for (const scoped of [false, true]) {
            // Workspace defaults live in Settings; a project's model defaults live on its project page.
            const scope = card(page, scoped ? 'Project model defaults' : 'Model defaults (Workspace)')
            if (scoped) {
                const response = await page.request.patch('/workspace/api/settings', { data: { section: 'models', expected_revision: (await read()).models.revision,
                    value: { provider: 'anthropic', llm_profile: null, model: 'workspace-parent', reasoning_effort: 'low' } } })
                expect(response.ok()).toBeTruthy()
                await page.reload()
                await openProjectPage(page, project)
                await page.getByRole('switch', { name: 'Override workspace model settings' }).click()
            }
            await chooseModel(page, 'anthropic', 'claude-opus-4-6', scope)
            await openPicker(page, scope)
            await page.getByRole('group', { name: 'Reasoning effort' }).getByRole('button', { name: 'High', exact: true }).click()
            await customModel(page, 'picker-reset-custom', scope)
            await scope.getByRole('button', { name: /^Save/ }).click()
            await expect.poll(async () => (await read(scoped)).models.stored.model).toBe('picker-reset-custom')
            const expected = scoped ? 'workspace-parent · Low' : 'Discovery default · Medium'
            await expect(scope.getByRole('button', { name: /^Model:/ })).toContainText('picker-reset-custom · High')
            await openPicker(page, scope)
            await expect(page.getByRole('button', { name: /^Use default ·/ })).toHaveText(`Use default · ${expected}`)
            await page.getByRole('button', { name: /^Use default ·/ }).click()
            await expect(scope.getByRole('button', { name: /^Model:/ })).toContainText(`Default: ${expected}`)
            await openPicker(page, scope)
            const high = page.getByRole('group', { name: 'Reasoning effort' }).getByRole('button', { name: 'High', exact: true })
            if (scoped) {
                // Inheriting the workspace group leaves no effort to set without pinning its model.
                await expect(high).toBeDisabled()
                await page.keyboard.press('Escape')
            } else {
                // The workspace default is the stored Codex group, so its effort stays settable.
                await high.click()
                const saved = page.waitForResponse(response => response.url().endsWith('/workspace/api/settings') && response.request().method() === 'PATCH')
                await scope.getByRole('button', { name: /^Save/ }).click()
                expect((await saved).status()).toBe(200)
                await expect.poll(async () => (await read(scoped)).models.stored).toMatchObject({ provider: 'codex', llm_profile: null, model: null, reasoning_effort: 'high' })
                await page.reload()
                await page.getByTestId('activity-settings').click()
                await expect(scope.getByRole('button', { name: /^Model:/ })).toContainText('Discovery default · High')
                await openPicker(page, scope)
                await page.getByRole('button', { name: /^Use default ·/ }).click()
            }
            await scope.getByRole('button', { name: /^Save/ }).click()
            if (scoped) await expect.poll(async () => (await read(scoped)).models.stored).toBeNull()
            else await expect.poll(async () => (await read(scoped)).models.stored).toMatchObject({ provider: 'codex', llm_profile: null, model: null, reasoning_effort: null })
            if (scoped) await page.getByRole('switch', { name: 'Override workspace model settings' }).click()
            await expect(scope.getByRole('button', { name: /^Model:/ })).toContainText(expected)
            if (scoped) await scope.getByRole('button', { name: /^Discard/ }).click()
        }
        await page.reload()
        await openProjectPage(page, project)
        await expect(page.getByRole('switch', { name: 'Override workspace model settings' })).not.toBeChecked()
    } finally {
        await page.request.patch('/workspace/api/settings', { data: { section: 'models', expected_revision: (await read()).models.revision, value: original.stored ?? original.effective } })
    }
})

test('mission picker default persists through the real conversation backend', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('mission-picker-reset')
    mkdirSync(project, { recursive: true })
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
    const settings = await (await page.request.get(`/workspace/api/settings?project_path=${encodeURIComponent(project)}`)).json()
    const parent = { provider: 'anthropic', llm_profile: null, model: 'project-parent', reasoning_effort: 'low' }
    expect((await page.request.patch('/workspace/api/settings', { data: { section: 'project_models', expected_revision: settings.models.revision,
        value: { project_path: project, model_settings: parent } } })).ok()).toBeTruthy()
    const response = await page.request.post(`/workspace/api/missions?project_path=${encodeURIComponent(project)}`, { data: { fields: { title: 'Picker reset mission', description: 'Reset model', archived: false }, actor: 'human' } })
    expect(response.ok(), await response.text()).toBeTruthy()
    const mission = await response.json()
    const read = async () => (await page.request.get(`/workspace/api/conversations/${mission.id}?project_path=${encodeURIComponent(project)}`)).json()
    await page.goto('/')
    await page.getByTestId('activity-missions').click()
    await page.getByRole('button', { name: 'Picker reset mission', exact: true }).click()
    const detail = page.getByRole('region', { name: 'Mission details' })
    const openModel = async () => {
        await detail.getByRole('button', { name: 'Mission actions' }).click()
        await page.getByRole('menuitem', { name: 'Model', exact: true }).click()
    }
    await openModel()
    await expect(detail.getByRole('button', { name: /^Model:/ })).toContainText('project-parent · Low')
    await customModel(page, 'mission-custom', detail)
    await chooseEffort(page, 'High')
    await detail.getByRole('button', { name: 'Save model', exact: true }).click()
    await expect.poll(async () => (await read()).settings.models.stored.model).toBe('mission-custom')
    await openModel()
    await expect(detail.getByRole('button', { name: /^Model:/ })).toContainText('mission-custom · High')
    await openPicker(page, detail)
    await expect(page.getByRole('button', { name: /^Use default ·/ })).toHaveText('Use default · project-parent · Low')
    await page.getByRole('button', { name: /^Use default ·/ }).click()
    await expect(detail.getByRole('button', { name: /^Model:/ })).toContainText('Default: project-parent · Low')
    // Inheriting leaves no effort to set without pinning the parent's model.
    await openPicker(page, detail)
    await expect(page.getByRole('group', { name: 'Reasoning effort' }).getByRole('button', { name: 'High', exact: true })).toBeDisabled()
    await page.keyboard.press('Escape')
    await detail.getByRole('button', { name: 'Save model', exact: true }).click()
    await expect.poll(async () => (await read()).settings.models.stored).toBeNull()
    await expect(detail.getByRole('button', { name: 'Save model', exact: true })).toHaveCount(0)
    await openModel()
    await expect(detail.getByRole('button', { name: /^Model:/ })).toContainText('project-parent · Low')
    expect((await read()).turns).toEqual([])
})
