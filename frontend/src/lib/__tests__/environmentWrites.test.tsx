import { render, renderHook, screen, waitFor, act } from '@testing-library/react'
import { DialogProvider } from '@/components/app/dialog-controller'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ClientPreferencesController } from '@/features/settings/ClientPreferencesController'
import { completePreferenceInteraction } from '@/features/settings/services/clientPreferences'
import { useConversationComposer } from '@/features/projects/hooks/useConversationComposer'
import { useProjectConversationCache } from '@/features/projects/hooks/useProjectConversationCache'
import { useProjectsHomeController } from '@/features/projects/hooks/useProjectsHomeController'
import { useProjectsHomeInteractionState } from '@/features/projects/hooks/useProjectsHomeInteractionState'
import type { NormalizedConversationRecord } from '@/features/projects/model/projectsHomeState'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { cachedAppearance, applyTheme } from '@/lib/theme'
import { requestNavigation } from '@/state/workspaceSlice'
import { ATTENTION_CHANGED_EVENT } from '@/lib/api/attentionApi'
import { reviewProposedPlanValidated } from '@/lib/api/conversationsApi'
import { closeFlowOpening, primeFlowSaveBaseline, refusedFlowSaves, refusedFlowSaveStatus, resetFlowSaveBaselines, retryLastSaveContent, retryRefusedFlowSave, saveFlowContent, sendPendingFlowSaves, setInvalidDraft, waitForFlowSaves } from '@/lib/flowPersistence'
import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { conversationMutationAck } from '@/test/conversationWire'
import { deferred, switchEnvironment } from '@/test/environments'

// Every write started in a remote environment reaches it, through its proxy, after a switch to local.
const REMOTE = '/workspace/api/environments/links/env-r/proxy'
const PROJECT = '/tmp/project'
const flow = (title: string) => ({ flow: { schema_version: '2', id: 'demo', title, nodes: {} } })
// The flow's files as the server reads them back: what a flow opens from, and what recovery checks.
const files = (title: string) => [{ name: 'flow.md', content: `---\nid: demo\ntitle: ${title}\n---\n` }]
const flowRead = (title: string) => ({ name: 'demo', files: files(title), ...flow(title) })

let sent: string[] = []
let bodies: unknown[] = []
let answer: (url: string, method: string) => unknown = () => ({})

const json = (payload: unknown, status = 200) =>
    new Response(JSON.stringify(payload), { status, headers: { 'Content-Type': 'application/json' } })

beforeEach(() => {
    useStore.setState(useStore.getInitialState(), true)
    resetFlowSaveBaselines()
    sent = []
    bodies = []
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const method = init?.method ?? 'GET'
        sent.push(`${method} ${String(input)}`)
        bodies.push(typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body)
        const payload = await answer(String(input), method)
        return payload instanceof Response ? payload : json(payload)
    }))
})

afterEach(() => {
    vi.unstubAllGlobals()
})

describe('flow saves', () => {
    beforeEach(() => {
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : { status: 'saved' })
    })

    it('sends a scheduled save from the cleanup flush to the environment it was scheduled in', async () => {
        await switchEnvironment('env-r')
        const { result, unmount } = renderHook(() => useFlowSaveScheduler({
            flowName: 'demo',
            debounceMs: 60_000,
            buildContent: () => flow('Remote'),
        }))
        act(() => result.current.scheduleSave())
        await switchEnvironment(null)
        unmount()

        await waitFor(() => expect(sent).toEqual([
            `POST ${REMOTE}/attractor/preview`,
            `POST ${REMOTE}/attractor/api/flows`,
        ]))
    })

    it('saves after its preview in the environment the save started in, leaving the shown save state alone', async () => {
        await switchEnvironment('env-r')
        const preview = deferred<unknown>()
        answer = (url) => (url.endsWith('/preview') ? preview.promise : { status: 'saved' })
        const saving = saveFlowContent('demo', flow('Remote'), PROJECT)
        await waitFor(() => expect(sent).toHaveLength(1))
        await switchEnvironment(null)
        resetStoreForEnvironmentSwitch(true)

        preview.resolve({ status: 'ok' })
        await expect(saving).resolves.toBe(true)
        expect(sent).toEqual([`POST ${REMOTE}/attractor/preview`, `POST ${REMOTE}/attractor/api/flows`])
        expect(useStore.getState().saveState).toBe('idle')
    })

    it('keeps each environment its own baseline for the same project and flow', async () => {
        primeFlowSaveBaseline('demo', flow('Local'), files('Local'), PROJECT)
        await switchEnvironment('env-r')
        const saved = deferred<unknown>()
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : saved.promise)
        const remoteSave = saveFlowContent('demo', flow('Remote'), PROJECT)
        await waitFor(() => expect(sent).toHaveLength(2))
        await switchEnvironment(null)
        saved.resolve({ status: 'saved' })
        await expect(remoteSave).resolves.toBe(true)

        sent = []
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : { status: 'saved' })
        // The late remote ack left the local baseline as it was.
        await expect(saveFlowContent('demo', flow('Local'), PROJECT)).resolves.toBe(true)
        expect(sent).toEqual([])
        // Local content matching what remote saved is still a change here.
        await expect(saveFlowContent('demo', flow('Remote'), PROJECT)).resolves.toBe(true)
        expect(sent).toEqual(['POST /attractor/preview', 'POST /attractor/api/flows'])
    })

    it('sends a remote edit queued behind an autosave, whatever input is invalid in the environment switched to', async () => {
        await switchEnvironment('env-r')
        const firstSaved = deferred<unknown>()
        const flowSaves = () => sent.filter((request) => request.endsWith('/api/flows'))
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : flowSaves().length === 1 ? firstSaved.promise : { status: 'saved' })
        const first = saveFlowContent('demo', flow('Remote one'), PROJECT)
        await waitFor(() => expect(flowSaves()).toHaveLength(1))
        const queued = saveFlowContent('demo', flow('Remote two'), PROJECT)
        await switchEnvironment(null)
        resetStoreForEnvironmentSwitch(true)
        useStore.setState({ activeFlow: 'demo', activeProjectPath: PROJECT })
        setInvalidDraft('local-field', 'Routes: a line without a target.')
        try {
            firstSaved.resolve({ status: 'saved' })
            await expect(first).resolves.toBe(true)
            await expect(queued).resolves.toBe(true)
            expect(flowSaves()).toEqual([`POST ${REMOTE}/attractor/api/flows`, `POST ${REMOTE}/attractor/api/flows`])
            // Saves here still wait for it.
            await expect(saveFlowContent('demo', flow('Local'), PROJECT)).resolves.toBe(false)
            expect(useStore.getState().saveErrorMessage).toContain('Routes: a line without a target.')
        } finally { setInvalidDraft('local-field', null) }
    })

    it('lets this environment go on while another environment\'s save is in flight, and still guards closing the browser', async () => {
        await switchEnvironment('env-r')
        const remoteSaved = deferred<unknown>()
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : remoteSaved.promise)
        const remote = saveFlowContent('demo', flow('Remote'), PROJECT)
        await waitFor(() => expect(sent).toContain(`POST ${REMOTE}/attractor/api/flows`))
        await switchEnvironment(null)
        const { unmount } = renderHook(() => useFlowSaveScheduler({ flowName: 'demo', debounceMs: 60_000, buildContent: () => flow('Local') }))
        try {
            // Opening another flow, switching editor mode or launching here waits for nothing.
            expect(sendPendingFlowSaves(null)).toBe(false)
            const held = new Promise((resolve) => setTimeout(resolve, 0, 'held'))
            await expect(Promise.race([waitForFlowSaves().then(() => 'done'), held])).resolves.toBe('done')
            const unload = new Event('beforeunload', { cancelable: true })
            window.dispatchEvent(unload)
            expect(unload.defaultPrevented).toBe(true)
        } finally {
            remoteSaved.resolve({ status: 'saved' })
            await remote
            unmount()
        }
    })

    it('keeps a remote save refused after a switch away for that environment, retrying it only once the flow opens there unchanged', async () => {
        await switchEnvironment('env-r')
        primeFlowSaveBaseline('demo', flow('Base'), files('Base'), PROJECT)
        const remoteSaved = deferred<unknown>()
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : url.startsWith(REMOTE) ? remoteSaved.promise : { status: 'saved' })
        const remote = saveFlowContent('demo', flow('Remote'), PROJECT)
        await waitFor(() => expect(sent).toContain(`POST ${REMOTE}/attractor/api/flows`))
        await switchEnvironment(null)
        resetStoreForEnvironmentSwitch(true)
        closeFlowOpening('demo', PROJECT, 'env-r')
        useStore.setState({ activeFlow: 'demo', activeProjectPath: PROJECT })
        await expect(saveFlowContent('demo', flow('Local'), PROJECT)).resolves.toBe(true)
        const local = useStore.getState()

        remoteSaved.resolve(json({ detail: 'disk full' }, 500))
        await expect(remote).resolves.toBe(false)
        // This environment's flow of the same name is untouched; the refusal is kept for its own.
        expect(useStore.getState()).toBe(local)
        const [refused] = refusedFlowSaves()
        expect(refused).toMatchObject({ name: 'demo', projectPath: PROJECT, environmentId: 'env-r', message: 'disk full', retryable: true })
        expect(refusedFlowSaveStatus(refused)).toBe('hidden')

        // Back there, nothing retries it before the flow opens, nor through the save state shown.
        await switchEnvironment('env-r')
        resetStoreForEnvironmentSwitch(false)
        sent = []
        bodies = []
        answer = (url, method) => (method === 'GET' ? flowRead('Base') : url.endsWith('/preview') ? { status: 'ok' } : { status: 'saved' })
        await expect(retryRefusedFlowSave(refused)).resolves.toBe(false)
        await expect(retryLastSaveContent()).resolves.toBe(false)
        expect(sent).toEqual([])
        // Opened still as the edits found it, Retry reads the files there again just before sending the edits.
        primeFlowSaveBaseline('demo', flow('Base'), files('Base'), PROJECT)
        expect(refusedFlowSaveStatus(refused)).toBe('unchanged')
        await expect(retryRefusedFlowSave(refused)).resolves.toBe(true)
        expect(sent).toEqual([
            `POST ${REMOTE}/attractor/preview`,
            `GET ${REMOTE}/attractor/api/flows/demo?project_path=%2Ftmp%2Fproject`,
            `POST ${REMOTE}/attractor/api/flows`,
        ])
        expect(bodies[2]).toMatchObject({ flow: { title: 'Remote' } })
        expect(refusedFlowSaves()).toEqual([])
    })

    it('never retries a refused save over a version saved since', async () => {
        primeFlowSaveBaseline('demo', flow('Base'), files('Base'), PROJECT)
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : json({ detail: 'unavailable' }, 503))
        // Refused while another flow is open.
        useStore.setState({ activeFlow: 'other', activeProjectPath: PROJECT })
        await expect(saveFlowContent('demo', flow('Refused'), PROJECT)).resolves.toBe(false)
        expect(useStore.getState().saveState).toBe('idle')
        const [refused] = refusedFlowSaves()

        // It opens with newer content saved meanwhile.
        primeFlowSaveBaseline('demo', flow('Newer'), files('Newer'), PROJECT)
        expect(refusedFlowSaveStatus(refused)).toBe('changed')
        sent = []
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : { status: 'saved' })
        await expect(retryRefusedFlowSave(refused)).resolves.toBe(false)
        expect(sent).toEqual([])
    })

    it('never sends a save queued before its flow opened anew, keeping it for the flow’s notice', async () => {
        await switchEnvironment('env-r')
        useStore.setState({ activeFlow: 'demo', activeProjectPath: PROJECT })
        primeFlowSaveBaseline('demo', flow('Base'), files('Base'), PROJECT)
        const firstSaved = deferred<unknown>()
        const flowSaves = () => sent.filter((request) => request.endsWith('/api/flows'))
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : flowSaves().length === 1 ? firstSaved.promise : { status: 'saved' })
        const first = saveFlowContent('demo', flow('One'), PROJECT)
        await waitFor(() => expect(flowSaves()).toHaveLength(1))
        const queued = saveFlowContent('demo', flow('Two'), PROJECT)
        // The editor leaves the flow and opens it again, as loaded, before the queued save goes.
        closeFlowOpening('demo', PROJECT, 'env-r')
        useStore.getState().resetSaveState()
        primeFlowSaveBaseline('demo', flow('Base'), files('Base'), PROJECT)

        firstSaved.resolve(json({ detail: 'unavailable' }, 503))
        await expect(first).resolves.toBe(false)
        await expect(queued).resolves.toBe(false)
        expect(flowSaves()).toHaveLength(1)
        // The flow open now shows neither, nor retries them; its notice keeps the later edits.
        expect(useStore.getState().saveState).toBe('idle')
        await expect(retryLastSaveContent()).resolves.toBe(false)
        expect(flowSaves()).toHaveLength(1)
        expect(refusedFlowSaves()).toMatchObject([{ content: flow('Two'), message: 'These edits weren’t sent: the flow opened again first.', retryable: true }])
    })

    it.each([
        ['saved', 'Newer', { status: 'saved' }],
        ['refused', 'Base', json({ detail: 'unavailable' }, 503)],
    ])('sends edits still waiting to save before retrying a refused save, which then keeps them, %s', async (_, savedTitle, newerAnswer) => {
        primeFlowSaveBaseline('demo', flow('Base'), files('Base'), PROJECT)
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : json({ detail: 'unavailable' }, 503))
        useStore.setState({ activeFlow: 'other', activeProjectPath: PROJECT })
        await expect(saveFlowContent('demo', flow('Refused'), PROJECT)).resolves.toBe(false)
        const [refused] = refusedFlowSaves()
        // The flow opens unchanged, and an edit there waits to save when Retry is clicked.
        useStore.setState({ activeFlow: 'demo', viewMode: 'editor' })
        primeFlowSaveBaseline('demo', flow('Base'), files('Base'), PROJECT)
        expect(refusedFlowSaveStatus(refused)).toBe('unchanged')
        const { result, unmount } = renderHook(() => useFlowSaveScheduler({ flowName: 'demo', debounceMs: 60_000, buildContent: () => flow('Newer') }))
        try {
            act(() => result.current.scheduleSave())
            sent = []
            bodies = []
            // Refused, they are still newer than the refused save's, in the editor that holds them.
            answer = (url, method) => (method === 'GET' ? flowRead(savedTitle) : url.endsWith('/preview') ? { status: 'ok' } : newerAnswer)
            await expect(retryRefusedFlowSave(refused)).resolves.toBe(false)
            expect(sent.filter((request) => request.endsWith('/api/flows'))).toEqual(['POST /attractor/api/flows'])
            expect(bodies[sent.indexOf('POST /attractor/api/flows')]).toMatchObject({ flow: { title: 'Newer' } })
            expect(refusedFlowSaveStatus(refusedFlowSaves()[0])).toBe('changed')
        } finally { unmount() }
    })
})

it('saves a preference change to the environment it was made in, even once its load answers after a switch', async () => {
    const view = (revision: string) => ({ preferences: {
        scope: 'client',
        client_id: 'browser-one',
        revision,
        stored: { editor_mode: null, editor_sidebar_width: null },
        effective: { editor_mode: 'files', editor_sidebar_width: 400 },
    } })
    localStorage.setItem('spark.client_id', 'browser-one')
    const load = deferred<unknown>()
    answer = (_url, method) => (method === 'GET' ? load.promise : view('second'))
    await switchEnvironment('env-r')
    render(<ClientPreferencesController />)
    act(() => completePreferenceInteraction({ editor_sidebar_width: 450 }))
    await switchEnvironment(null)
    const local = useStore.getState()

    load.resolve(view('first'))
    await waitFor(() => expect(sent.filter((request) => request.startsWith('PATCH'))).toEqual([
        `PATCH ${REMOTE}/workspace/api/settings`,
    ]))
    expect(sent.every((request) => request.includes(REMOTE))).toBe(true)
    // The remote answers change nothing in the local view.
    expect(useStore.getState().editorSidebarWidth).toBe(local.editorSidebarWidth)
    expect(useStore.getState().preferredEditorMode).toBe(local.preferredEditorMode)
    expect(useStore.getState().clientPreferencesLoaded).toBe(false)
})

it('saves a change queued behind a save in flight after the workspace unmounts, telling only its environment, and keeps this browser\'s appearance', async () => {
    const view = (revision: string, width = 400) => ({ preferences: {
        scope: 'client',
        client_id: 'browser-one',
        revision,
        stored: { editor_mode: null, editor_sidebar_width: null },
        effective: { editor_mode: 'files', editor_sidebar_width: width, appearance: 'dark' },
    } })
    localStorage.setItem('spark.client_id', 'browser-one')
    applyTheme('light')
    const firstSave = deferred<unknown>()
    answer = (_url, method) => (method === 'GET' ? view('first') : sent.filter((request) => request.startsWith('PATCH')).length === 1 ? firstSave.promise : view('third', 500))
    await switchEnvironment('env-r')
    const { unmount } = render(<ClientPreferencesController />)
    await waitFor(() => expect(useStore.getState().clientPreferencesLoaded).toBe(true))
    // A remote environment's appearance leaves this browser's as it is.
    expect(cachedAppearance()).toBe('light')

    act(() => completePreferenceInteraction({ editor_sidebar_width: 450 }))
    await waitFor(() => expect(sent.filter((request) => request.startsWith('PATCH'))).toHaveLength(1))
    act(() => completePreferenceInteraction({ editor_sidebar_width: 500 }))
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    unmount()
    const live = vi.fn()
    window.addEventListener('spark:settings-live-event', live)
    try {
        firstSave.resolve(view('second', 450))
        await waitFor(() => expect(sent.filter((request) => request.startsWith('PATCH'))).toHaveLength(2))
        await new Promise((resolve) => setTimeout(resolve, 0))

        expect(sent.every((request) => request.includes(REMOTE))).toBe(true)
        expect(bodies.at(-1)).toMatchObject({ expected_revision: 'second', value: { preferences: { editor_sidebar_width: 500 } } })
        expect(live).not.toHaveBeenCalled()
    } finally { window.removeEventListener('spark:settings-live-event', live) }
})

it('sends a message and a mode switch delayed by a settings save to the environment they started in, putting a refused message back there', async () => {
    const thread = {
        conversation_id: 'conversation-2',
        project_path: PROJECT,
        title: 'Thread',
        created_at: '2026-09-01T00:00:00Z',
        updated_at: '2026-09-01T00:00:00Z',
        revision: 6,
        chat_mode: 'plan',
        turns: [],
        segments: [],
    }
    const settingsSaved = deferred<boolean | string>()
    // The send fails, so its draft restore would show if it reached the local view.
    answer = (url) => (url.endsWith('/turns') ? json({ detail: 'unavailable' }, 503) : conversationMutationAck(thread))
    const applyConversationBatch = vi.fn()
    const setPanelError = vi.fn()
    const setPendingConversationTurn = vi.fn()
    const useComposer = (conversationId: string) => useConversationComposer({
        activeProjectPath: PROJECT,
        chatDraft: 'Hello there',
        attachments: [],
        isUploadBlocking: false,
        isChatInputDisabled: false,
        waitForPendingSettingsSave: () => settingsSaved.promise,
        ensureConversationId: () => conversationId,
        getCurrentConversationId: () => conversationId,
        // The chat's record is in the remote cache until the switch resets it.
        getConversationRecord: () => (getActiveEnvironmentId() === 'env-r' ? { settingsRevision: '5' } as NormalizedConversationRecord : undefined),
        applyConversationBatch,
        formatErrorMessage: (_error, fallback) => fallback,
        setChatDraft: vi.fn(),
        setPanelError,
        setPendingConversationTurn,
    })

    await switchEnvironment('env-r')
    useStore.getState().updateHomeProjectSession(PROJECT, { chatDraft: 'Hello there' })
    useComposer('conversation-1').onChatComposerSubmit()
    const switching = useComposer('conversation-2').onSwitchChatMode('plan')
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    useStore.getState().updateHomeProjectSession(PROJECT, { chatDraft: 'Local draft' })

    // The pending settings save acknowledged revision 7, after the mode switch began.
    settingsSaved.resolve('7')
    await switching
    await waitFor(() => expect(sent).toHaveLength(2))
    // Let the failed send's handling finish.
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(sent).toEqual(expect.arrayContaining([
        `POST ${REMOTE}/workspace/api/conversations/conversation-1/turns`,
        `PUT ${REMOTE}/workspace/api/conversations/conversation-2/settings`,
    ]))
    expect(bodies[sent.indexOf(`PUT ${REMOTE}/workspace/api/conversations/conversation-2/settings`)]).toMatchObject({ expected_revision: '7' })
    expect(applyConversationBatch).not.toHaveBeenCalled()
    expect(setPanelError).not.toHaveBeenCalled()
    expect(setPendingConversationTurn).not.toHaveBeenCalled()
    const state = useStore.getState()
    expect(state.homeProjectSessionsByPath[PROJECT]?.chatDraft).toBe('Local draft')
    expect(state.homeConversationSessionsById).toEqual({})
    expect(state.sendingChats).toEqual({})

    // Back in the remote environment, the refused message is its draft again, with the error.
    await switchEnvironment('env-r')
    resetStoreForEnvironmentSwitch(false)
    expect(useStore.getState().homeProjectSessionsByPath[PROJECT]?.chatDraft).toBe('Hello there')
    expect(setPanelError).toHaveBeenCalledExactlyOnceWith('Unable to send the project chat turn.')
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    expect(useStore.getState().homeProjectSessionsByPath).toEqual({})
})

it('writes nothing from a chat reply or deferred navigation that lands after a switch', async () => {
    await switchEnvironment('env-r')
    const cache = renderHook(() => useProjectConversationCache({
        persistProjectState: vi.fn(async () => {}),
        projectSessionsByPath: {},
        updateProjectSessionState: vi.fn(),
    }))
    const interaction = renderHook(() => useProjectsHomeInteractionState({ activeConversationId: 'conversation-1', activeProjectPath: PROJECT }))
    // An open editor holds navigation until its saves finish.
    let release = () => {}
    const hold = (event: Event) => {
        event.preventDefault()
        release = (event as CustomEvent<{ proceed: () => void }>).detail.proceed
    }
    window.addEventListener('spark:before-navigation', hold)
    const open = vi.fn()
    try {
        requestNavigation(open)
        await switchEnvironment(null)
        resetStoreForEnvironmentSwitch(true)
        const local = useStore.getState()

        release()
        act(() => {
            cache.result.current.commitConversationCache((current) => ({ ...current, conversationsById: { remote: {} as NormalizedConversationRecord } }))
            interaction.result.current.setChatDraft('Remote draft')
            interaction.result.current.setPanelError('Remote error')
        })

        expect(open).not.toHaveBeenCalled()
        expect(useStore.getState().homeConversationCache).toBe(local.homeConversationCache)
        expect(useStore.getState().homeProjectSessionsByPath).toEqual({})
    } finally { window.removeEventListener('spark:before-navigation', hold) }
})

it('switches a new chat\'s mode at the revision a model save before it acknowledged, and shows neither answer after a switch', async () => {
    const thread = (revision: number) => ({
        conversation_id: 'conversation-1', project_path: PROJECT, title: 'Thread', created_at: '2026-09-01T00:00:00Z',
        updated_at: '2026-09-01T00:00:00Z', revision, chat_mode: 'chat', turns: [], segments: [],
    })
    const modelSaved = deferred<unknown>()
    const projectSaved = deferred<unknown>()
    const puts = () => sent.filter((request) => request.startsWith('PUT') && request.endsWith('/settings'))
    answer = (url, method) => {
        if (url.includes('/projects/chat-models')) return { providers: { codex: { status: 'available', error: null, label: 'Codex' } }, models: [
            { provider: 'codex', id: 'gpt-5.4', display: 'GPT-5.4', is_default: true, supported_reasoning_efforts: ['medium'], default_reasoning_effort: 'medium' },
        ] }
        if (url.endsWith('/workspace/api/settings')) return { models: { scope: 'project', source: 'workspace', revision: '1', stored: null,
            effective: { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null } } }
        if (method === 'PUT' && url.endsWith('/settings')) return puts().length === 1 ? modelSaved.promise : conversationMutationAck(thread(9))
        if (url.endsWith('/projects/state')) return projectSaved.promise
        return {}
    }
    await switchEnvironment('env-r')
    act(() => {
        useStore.getState().registerProject(PROJECT)
        useStore.getState().setActiveProjectPath(PROJECT)
    })
    const { result, unmount } = renderHook(() => useProjectsHomeController())
    await waitFor(() => expect(result.current.surfaceProps.onChatModeChange).toBeDefined())
    act(() => result.current.surfaceProps.onModelSettingsChange({ provider: 'codex', llm_profile: null, model: 'gpt-5.4', reasoning_effort: 'medium' }))
    await waitFor(() => expect(puts()).toHaveLength(1))
    act(() => result.current.surfaceProps.onChatModeChange?.('plan'))
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    unmount()
    // This machine has a project at the same path, with a chat of its own open.
    act(() => {
        useStore.getState().registerProject(PROJECT)
        useStore.getState().updateProjectSessionState(PROJECT, { conversationId: 'local-chat' })
    })
    const local = useStore.getState()

    modelSaved.resolve(conversationMutationAck(thread(7)))
    await waitFor(() => expect(puts()).toHaveLength(2))
    expect(puts().every((request) => request.includes(REMOTE))).toBe(true)
    expect(bodies[sent.lastIndexOf(puts()[1])]).toMatchObject({ chat_mode: 'plan', expected_revision: '7' })
    // The new chat's project state was sent remotely; its answer changes nothing here.
    expect(sent).toContain(`PATCH ${REMOTE}/workspace/api/projects/state`)
    projectSaved.resolve({ project_id: 'remote', project_path: PROJECT, display_name: 'Remote', active_conversation_id: 'conversation-1', is_favorite: true })
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(useStore.getState().projectRegistry).toEqual(local.projectRegistry)
    expect(useStore.getState().projectSessionsByPath).toEqual(local.projectSessionsByPath)
})

it('leaves this environment\'s chats and selection as they are when a chat delete\'s refresh finishes after a switch', async () => {
    const listed = deferred<unknown>()
    answer = (url, method) => {
        if (method === 'DELETE') return { status: 'deleted', conversation_id: 'conversation-1', project_path: PROJECT }
        if (url.includes('/projects/conversations') && sent.some((request) => request.startsWith('DELETE'))) return listed.promise
        return {}
    }
    await switchEnvironment('env-r')
    act(() => {
        useStore.getState().registerProject(PROJECT)
        useStore.getState().setActiveProjectPath(PROJECT)
        useStore.getState().updateProjectSessionState(PROJECT, { conversationId: 'conversation-1' })
    })
    const { result, unmount } = renderHook(() => useProjectsHomeController(), { wrapper: DialogProvider })
    let deleting!: Promise<void>
    act(() => { deleting = result.current.sidebarProps.onDeleteConversationThread(PROJECT, 'conversation-1', 'Thread') })
    await act(async () => screen.getByTestId('shared-dialog-confirm').click())
    await waitFor(() => expect(sent.filter((request) => request.startsWith(`GET ${REMOTE}/workspace/api/projects/conversations`)).length).toBeGreaterThan(0))
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    // This machine has a chat with the same ID open.
    act(() => {
        useStore.getState().registerProject(PROJECT)
        useStore.getState().updateProjectSessionState(PROJECT, { conversationId: 'conversation-1' })
    })
    const local = useStore.getState()
    sent = []

    listed.resolve(json({ detail: 'unavailable' }, 503))
    await act(async () => deleting)
    unmount()
    expect(sent).toEqual([])
    expect(useStore.getState().projectSessionsByPath).toEqual(local.projectSessionsByPath)
    expect(useStore.getState().homeConversationCache).toBe(local.homeConversationCache)
})

it('refreshes the Inbox only in the environment a plan approval was made in', async () => {
    const thread = { conversation_id: 'conversation-1', project_path: PROJECT, title: 'Thread', created_at: '2026-09-01T00:00:00Z',
        updated_at: '2026-09-01T00:00:00Z', revision: 3, chat_mode: 'plan', turns: [], segments: [] }
    const approved = deferred<unknown>()
    answer = () => approved.promise
    const refreshed = vi.fn()
    window.addEventListener(ATTENTION_CHANGED_EVENT, refreshed)
    try {
        await switchEnvironment('env-r')
        const approval = { project_path: PROJECT, disposition: 'approved' as const }
        const remote = reviewProposedPlanValidated('conversation-1', 'plan-1', approval)
        await switchEnvironment(null)
        approved.resolve(conversationMutationAck(thread))
        await remote
        expect(sent).toEqual([`POST ${REMOTE}/workspace/api/conversations/conversation-1/proposed-plans/plan-1/review`])
        expect(refreshed).not.toHaveBeenCalled()

        await reviewProposedPlanValidated('conversation-1', 'plan-1', approval)
        expect(refreshed).toHaveBeenCalledTimes(1)
    } finally { window.removeEventListener(ATTENTION_CHANGED_EVENT, refreshed) }
})
