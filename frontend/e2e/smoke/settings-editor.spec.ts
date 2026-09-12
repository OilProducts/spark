import { spawn, type ChildProcess } from 'node:child_process'
import { mkdirSync } from 'node:fs'
import { createServer } from 'node:net'
import path from 'node:path'
import { expect, test } from '@playwright/test'

test('runtime settings Save, Discard, validation, navigation guard, live conflict, and reload', async ({ page, context }) => {
    const read = async () => (await page.request.get('/workspace/api/settings')).json()
    const original = (await read()).runtime.stored
    await page.goto('/')
    await page.getByTestId('nav-mode-settings').click()
    const input = page.getByLabel('Flows directory', { exact: true })
    await expect(input).toBeVisible()
    const second = await context.newPage()
    try {
        await second.goto('/')
        await second.getByTestId('nav-mode-settings').click()
        const otherInput = second.getByLabel('Flows directory', { exact: true })
        await expect(otherInput).toBeVisible()
        await input.fill('/tmp/spark-settings-draft')
        expect((await read()).runtime.stored).toEqual(original)
        await page.getByLabel('Project roots (one absolute path per line)').fill('relative')
        await expect(page.getByText('Each project root must be an absolute path.')).toBeVisible()
        await expect(page.getByRole('button', { name: 'Save runtime settings', exact: true })).toBeDisabled()
        await page.getByRole('button', { name: 'Discard runtime changes', exact: true }).click()
        await expect(input).toHaveValue(original.flows_dir ?? '')
        await input.fill('/tmp/spark-settings-first')
        await page.getByTestId('nav-mode-home').click()
        await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
        await expect(page.getByTestId('settings-panel')).toBeVisible()
        await expect(input).toHaveValue('/tmp/spark-settings-first')

        await otherInput.fill('/tmp/spark-settings-second')
        await second.getByRole('button', { name: 'Save runtime settings', exact: true }).click()
        await expect(second.getByText('Saved. Restart Spark to apply runtime changes.')).toBeVisible()
        await expect(page.getByText('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')).toBeVisible()
        await expect(input).toHaveValue('/tmp/spark-settings-first')
        await page.getByRole('button', { name: 'Save runtime settings', exact: true }).click()
        await expect(page.getByText('Settings changed since this document was read. Reload before saving.', { exact: false })).toBeVisible()
        await expect(input).toHaveValue('/tmp/spark-settings-first')
        expect((await read()).runtime.stored.flows_dir).toBe('/tmp/spark-settings-second')
        await page.getByRole('button', { name: 'Discard runtime changes', exact: true }).click()
        await expect(input).toHaveValue('/tmp/spark-settings-second')
        await page.reload()
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
    await page.getByTestId('nav-mode-settings').click()
    const provider = page.getByLabel('Default LLM Provider', { exact: true })
    await expect(provider).toBeEnabled()
    const other = await context.newPage()
    try {
        await provider.selectOption('claude-code')
        expect((await read()).models.effective).toEqual(original)
        await page.getByRole('button', { name: 'Save model defaults', exact: true }).click()
        await expect(page.getByText('Saved. Applies to the next message.', { exact: true })).toBeVisible()
        await page.reload()
        await expect(provider).toHaveValue('claude-code')
        await page.getByLabel('Default LLM Model', { exact: true }).selectOption('custom')
        await page.getByLabel('Custom model', { exact: true }).fill('retained-draft')
        await other.goto('/')
        await other.getByTestId('nav-mode-settings').click()
        await expect(other.getByLabel('Default LLM Provider', { exact: true })).toBeEnabled()
        await other.getByLabel('Default LLM Provider', { exact: true }).selectOption('codex')
        await other.getByRole('button', { name: 'Save model defaults', exact: true }).click()
        await expect(other.getByText('Saved. Applies to the next message.', { exact: true })).toBeVisible()
        await expect(page.getByText('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.', { exact: true })).toBeVisible()
        await expect(page.getByLabel('Custom model', { exact: true })).toHaveValue('retained-draft')
        await page.getByRole('button', { name: 'Save model defaults', exact: true }).click()
        await expect(page.getByText('Settings changed since this document was read. Reload before saving.', { exact: false })).toBeVisible()
        await expect(page.getByLabel('Custom model', { exact: true })).toHaveValue('retained-draft')
        await page.getByRole('button', { name: 'Discard model changes', exact: true }).click()
        await expect(provider).toHaveValue('codex')
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
        await page.getByTestId('nav-mode-settings').click()
        const width = page.getByLabel('Editor sidebar width (pixels)', { exact: true })
        await expect(width).toBeEnabled()
        const clientId = await page.evaluate(() => localStorage.getItem('spark.client_id'))
        expect(clientId).toBeTruthy()
        await width.fill('400')
        await page.getByLabel('Home sidebar primary split (0–1)', { exact: true }).fill('0.6')
        await page.getByLabel('Show advanced controls', { exact: true }).selectOption('true')
        await page.getByLabel('Expand child flows', { exact: true }).selectOption('true')
        await page.getByLabel('Open graph settings panel', { exact: true }).selectOption('true')
        await page.getByLabel('Run list scope', { exact: true }).selectOption('all')
        await page.getByLabel('Trigger list scope', { exact: true }).selectOption('active')
        await page.getByRole('button', { name: 'Save preferences', exact: true }).click()
        await expect(page.getByText('Client preferences saved.', { exact: true })).toBeVisible()
        await page.reload()
        await expect(width).toHaveValue('400')
        await expect(page.getByLabel('Home sidebar primary split (0–1)', { exact: true })).toHaveValue('0.6')
        await expect(page.getByLabel('Show advanced controls', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Expand child flows', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Open graph settings panel', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Run list scope', { exact: true })).toHaveValue('all')
        await expect(page.getByLabel('Trigger list scope', { exact: true })).toHaveValue('active')
        expect(await page.evaluate(() => localStorage.getItem('spark.client_id'))).toBe(clientId)
        await other.goto(page.url())
        await other.getByTestId('nav-mode-settings').click()
        const otherWidth = other.getByLabel('Editor sidebar width (pixels)', { exact: true })
        await expect(otherWidth).toBeEnabled()
        await expect(otherWidth).toHaveValue('')
        for (const label of ['Home sidebar primary split (0–1)', 'Show advanced controls', 'Expand child flows', 'Open graph settings panel', 'Run list scope', 'Trigger list scope']) {
            await expect(other.getByLabel(label, { exact: true })).toHaveValue('')
        }
        expect(await other.evaluate(() => localStorage.getItem('spark.client_id'))).not.toBe(clientId)
        await otherWidth.fill('500')
        await other.getByRole('button', { name: 'Save preferences', exact: true }).click()
        await expect(other.getByText('Client preferences saved.', { exact: true })).toBeVisible()
        await page.reload()
        await expect(width).toHaveValue('400')
        await expect(page.getByLabel('Home sidebar primary split (0–1)', { exact: true })).toHaveValue('0.6')
        await expect(page.getByLabel('Show advanced controls', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Expand child flows', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Open graph settings panel', { exact: true })).toHaveValue('true')
        await expect(page.getByLabel('Run list scope', { exact: true })).toHaveValue('all')
        await expect(page.getByLabel('Trigger list scope', { exact: true })).toHaveValue('active')
    } finally { await otherContext.close() }
})

test('profile CRUD saves explicitly, retains conflicts, validates mounts, and survives reload', async ({ page }) => {
    const read = async () => (await page.request.get('/workspace/api/settings')).json()
    const initial = await read()
    const id = `settings-e2e-${Date.now()}`
    await page.goto('/')
    await page.getByTestId('nav-mode-settings').click()
    try {
        await page.getByRole('button', { name: 'Add LLM profile', exact: true }).click()
        const profile = page.getByRole('group', { name: `LLM profile ${initial.llm_profiles.stored.length + 1}`, exact: true })
        await profile.getByRole('textbox', { name: `LLM profile ${initial.llm_profiles.stored.length + 1} ID` }).fill(id)
        await profile.getByLabel('Endpoint', { exact: true }).fill('http://localhost:9999/v1')
        await profile.getByLabel('Models (one per line)').fill('test-model')
        await profile.getByLabel('Default model', { exact: true }).selectOption('test-model')
        await profile.getByLabel('Credential environment variable').fill('SPARK_E2E_MISSING_CREDENTIAL')
        expect((await read()).llm_profiles.stored).toEqual(initial.llm_profiles.stored)
        await page.getByRole('button', { name: 'Save LLM profiles', exact: true }).click()
        await expect(page.getByText('Profiles saved. New work uses these settings.')).toBeVisible()
        await page.reload()
        await expect(profile.getByLabel('Endpoint', { exact: true })).toHaveValue('http://localhost:9999/v1')
        await profile.getByLabel('Label', { exact: true }).fill('Retained draft')
        let latest = await read()
        const external = await page.request.patch('/attractor/api/llm-profiles', { data: {
            expected_revision: latest.llm_profiles.revision,
            value: latest.llm_profiles.stored.map((entry: { id: string }) => entry.id === id ? { ...entry, label: 'External update' } : entry),
        } })
        expect(external.ok()).toBeTruthy()
        await expect(page.getByText('Profiles changed elsewhere. Your draft is retained; Discard reloads the latest values.')).toBeVisible()
        await page.getByRole('button', { name: 'Save LLM profiles', exact: true }).click()
        await expect(profile.getByLabel('Label', { exact: true })).toHaveValue('Retained draft')
        await expect(page.getByText('Settings changed since this document was read. Reload before saving.', { exact: false })).toBeVisible()
        await page.getByRole('button', { name: 'Discard LLM profile changes', exact: true }).click()
        await expect(profile.getByLabel('Label', { exact: true })).toHaveValue('External update')
        await page.getByRole('button', { name: `Delete LLM profile ${id}`, exact: true }).click()
        await page.getByRole('button', { name: 'Save LLM profiles', exact: true }).click()
        await expect.poll(async () => (await read()).llm_profiles.stored.some((entry: { id: string }) => entry.id === id)).toBe(false)

        await page.getByRole('button', { name: 'Add execution profile', exact: true }).click()
        const execution = page.getByRole('group', { name: `Execution profile ${initial.execution_profiles.stored.profiles.length + 1}`, exact: true })
        await execution.getByLabel('Profile ID', { exact: true }).fill(id)
        await execution.getByLabel('Label', { exact: true }).fill('E2E container')
        await execution.getByLabel('Mode', { exact: true }).selectOption('local_container')
        await expect(page.getByRole('button', { name: 'Save execution profiles', exact: true })).toBeDisabled()
        await execution.getByLabel('Container image').fill('worker:test')
        await execution.getByLabel('Capabilities (one per line)').fill('shell\nnetwork')
        await execution.getByLabel('Mounts (host:container[:options], one per line)').fill('invalid-mount')
        await page.getByRole('button', { name: 'Save execution profiles', exact: true }).click()
        await expect(page.getByText('Mounts must be an array of nonempty host:container[:options] strings.', { exact: false })).toBeVisible()
        await execution.getByLabel('Mounts (host:container[:options], one per line)').fill('/tmp:/workspace:ro')
        await page.getByRole('button', { name: 'Save execution profiles', exact: true }).click()
        await expect.poll(async () => (await read()).execution_profiles.stored.profiles.some((entry: { id: string }) => entry.id === id)).toBe(true)
        await page.reload()
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
    await page.getByTestId('nav-mode-settings').click()
    const port = page.getByLabel('Server port', { exact: true })
    const target = page.getByLabel('Client API target', { exact: true })
    await expect(port).toBeVisible()
    try {
        await port.fill('70000')
        await expect(page.getByRole('button', { name: 'Save connection settings', exact: true })).toBeDisabled()
        await port.fill('8123')
        await target.fill('https://spark.example')
        expect((await read()).connections.stored).toEqual(original.stored)
        await page.getByRole('button', { name: 'Save connection settings', exact: true }).click()
        await expect(page.getByText('Saved. Restart Spark to apply server binding changes.', { exact: true })).toBeVisible()
        const saved = (await read()).connections
        expect(saved.stored.server_port).toBe(8123)
        expect(saved.effective.server_port).toBe(original.effective.server_port)
        await page.reload()
        await expect(port).toHaveValue('8123')
        await expect(target).toHaveValue('https://spark.example')
        await target.fill('https://discard.example')
        await page.getByRole('button', { name: 'Discard connection changes', exact: true }).click()
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
    await page.getByTestId('nav-mode-settings').click()
    const group = page.getByRole('group', { name: 'openai', exact: true })
    const endpoint = group.getByLabel('base url', { exact: true })
    await expect(endpoint).toBeVisible()
    try {
        await endpoint.fill('https://user:SECRET@example.com')
        await expect(page.getByRole('button', { name: 'Save provider connections', exact: true })).toBeDisabled()
        await endpoint.fill('https://example.com/v1')
        await group.getByLabel('Credential environment variable').fill('CR_SETTINGS_TEST_REFERENCE')
        expect((await read()).providers.stored).toEqual(original.providers.stored)
        await page.getByRole('button', { name: 'Save provider connections', exact: true }).click()
        await expect.poll(async () => (await read()).providers.stored.openai.api_key_env).toBe('CR_SETTINGS_TEST_REFERENCE')
        await expect(group.getByText(/Effective:.*(Configured|Missing)/)).toBeVisible()
        const timeout = page.getByLabel('default command timeout ms', { exact: true })
        await timeout.fill('1234')
        await page.getByLabel('codex runtime root', { exact: true }).fill('/tmp/spark-cr-native-home')
        await expect(page.getByText(/Effective:.*Requires restart/).first()).toBeVisible()
        await page.getByRole('button', { name: 'Save agent settings', exact: true }).click()
        await expect.poll(async () => (await read()).agents.stored.default_command_timeout_ms).toBe(1234)
        const saved = await read()
        expect(saved.agents.effective.native.codex_runtime_root).toEqual(original.agents.effective.native.codex_runtime_root)
        expect(saved.providers.stored.openai.api_key_env).toBe('CR_SETTINGS_TEST_REFERENCE')
        expect(typeof saved.providers.credential_status.openai).toBe('boolean')
        expect(JSON.stringify(saved)).not.toContain('SECRET')
        await page.reload()
        await expect(endpoint).toHaveValue('https://example.com/v1')
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
        await page.getByTestId('nav-mode-settings').click()
        const width = page.getByLabel('Editor sidebar width (pixels)', { exact: true })
        await width.fill('420')
        await page.getByRole('button', { name: 'Save preferences', exact: true }).click()
        await expect(page.getByText('Client preferences saved.', { exact: true })).toBeVisible()
        const nativeHome = path.join(home, 'saved-agent-home')
        await page.getByLabel('codex runtime root', { exact: true }).fill(nativeHome)
        await page.getByRole('button', { name: 'Save agent settings', exact: true }).click()
        await expect.poll(async () => (await (await fetch(`${first}/workspace/api/settings`)).json()).agents.stored.native.codex_runtime_root).toBe(nativeHome)
        await page.goto('about:blank')
        await stop()
        const second = await start()
        expect(second).not.toBe(first)
        await page.goto(second)
        await page.getByTestId('nav-mode-settings').click()
        await expect(page.getByLabel('Editor sidebar width (pixels)', { exact: true })).toHaveValue('420')
        await expect(page.getByLabel('codex runtime root', { exact: true })).toHaveValue(nativeHome)
        const settings = await (await fetch(`${second}/workspace/api/settings`)).json()
        expect(settings.agents.effective.native.codex_runtime_root).toBe(nativeHome)
        expect(await page.evaluate(() => localStorage.getItem('spark.client_id'))).toBeNull()
    } finally { await page.goto('about:blank'); await stop() }
})

for (const transition of ['switch', 'clear']) {
    test(`project ${transition} confirms draft loss and cancellation preserves the editor`, async ({ page }, testInfo) => {
        const projects = [testInfo.outputPath('project-one'), testInfo.outputPath('project-two')]
        for (const project of projects) {
            mkdirSync(project, { recursive: true })
            expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
        }
        await page.goto('/')
        const switcher = page.getByTestId('top-nav-project-switcher')
        const selectProject = async (project: string) => {
            await switcher.click()
            await page.getByRole('option').filter({ hasText: project }).click()
        }
        await selectProject(projects[0])
        await page.getByTestId('nav-mode-settings').click()
        await page.getByRole('switch', { name: 'Override workspace model settings' }).click()
        const model = page.getByLabel('Project model', { exact: true })
        await model.fill('unsaved-project-model')
        const navigate = async () => {
            if (transition === 'clear') await page.getByTestId('top-nav-project-clear-button').click()
            else await selectProject(projects[1])
        }
        await navigate()
        await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
        await expect(model).toHaveValue('unsaved-project-model')
        await expect(switcher).toHaveAttribute('title', projects[0])
        const read = async () => (await page.request.get(`/workspace/api/settings?project_path=${encodeURIComponent(projects[0])}`)).json()
        expect((await read()).models.stored).toBeNull()
        await navigate()
        await page.getByRole('button', { name: 'Discard and leave', exact: true }).click()
        await expect(switcher).toHaveAttribute('title', transition === 'clear' ? 'No active project' : projects[1])
        await expect(model).toHaveCount(0)
        expect((await read()).models.stored).toBeNull()
    })
}

test('project execution dialog protects dismissal and handles clean and dirty external updates', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('execution-project')
    mkdirSync(project, { recursive: true })
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
    await page.goto('/')
    await page.getByTestId('top-nav-project-switcher').click()
    await page.getByRole('option').filter({ hasText: project }).click()
    await page.getByTestId('top-nav-project-settings-button').click()
    const dialog = page.getByTestId('project-settings-dialog')
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
    await page.keyboard.press('Escape')
    await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
    await expect(select).toContainText('Use workspace default')
    await page.mouse.click(10, 10)
    await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
    await expect(dialog).toBeVisible()
    await external(null)
    await expect(page.getByText('Project execution settings changed elsewhere. Your draft is retained; Discard reloads the latest values.', { exact: true })).toBeVisible()
    await expect(select).toContainText('Use workspace default')
    await page.getByTestId('project-settings-save-button').click()
    await expect(page.getByTestId('project-settings-save-error')).toContainText('Settings changed')
    await expect(select).toContainText('Use workspace default')
    await page.getByRole('button', { name: 'Discard and reload', exact: true }).click()
    await expect(page.getByTestId('project-settings-save-error')).toHaveCount(0)
    await expect(page.getByTestId('project-settings-save-button')).toBeDisabled()
    await select.click()
    await page.getByRole('option').filter({ hasText: /native/i }).click()
    await page.keyboard.press('Escape')
    await page.getByRole('button', { name: 'Discard and leave', exact: true }).click()
    await expect(dialog).toHaveCount(0)
    expect((await read()).execution.stored).toBeNull()
})

test('scoped configuration links open existing flow and trigger editors with navigation protection', async ({ page }) => {
    await page.goto('/')
    await page.getByTestId('nav-mode-settings').click()
    const draft = page.getByLabel('Flows directory', { exact: true })
    await draft.fill('/tmp/scoped-link-draft')
    await page.getByRole('button', { name: 'Edit flow policies', exact: true }).click()
    await page.getByRole('button', { name: 'Keep editing', exact: true }).click()
    await expect(draft).toHaveValue('/tmp/scoped-link-draft')
    await page.getByRole('button', { name: 'Edit flow policies', exact: true }).click()
    await page.getByRole('button', { name: 'Discard and leave', exact: true }).click()
    await expect(page.getByTestId('settings-panel')).toHaveCount(0)
    await expect(page.getByTestId('editor-no-flow-state')).toBeVisible()
    await page.getByTestId('nav-mode-settings').click()
    await page.getByRole('button', { name: 'Edit triggers', exact: true }).click()
    await expect(page.getByTestId('triggers-panel')).toBeVisible()
})
