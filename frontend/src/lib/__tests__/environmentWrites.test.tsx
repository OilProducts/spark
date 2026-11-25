import { render, renderHook, screen, waitFor, act } from '@testing-library/react'
import type { ReactNode } from 'react'
import { DialogProvider } from '@/components/app/dialog-controller'
import { HomeSessionController } from '@/app/AppSessionControllers'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ClientPreferencesController } from '@/features/settings/ClientPreferencesController'
import { completePreferenceInteraction } from '@/features/settings/services/clientPreferences'
import { useConversationComposer } from '@/features/projects/hooks/useConversationComposer'
import { markProjectUsed } from '@/features/projects/hooks/usePersistProjectState'
import { defaultProjectChoice } from '@/features/projects/model/projectChoices'
import { useProjectConversationCache } from '@/features/projects/hooks/useProjectConversationCache'
import { useProjectsHomeController } from '@/features/projects/hooks/useProjectsHomeController'
import { useProjectRegistryBootstrap } from '@/features/projects/hooks/useProjectRegistryBootstrap'
import { useProjectsHomeInteractionState } from '@/features/projects/hooks/useProjectsHomeInteractionState'
import { applyConversationPageToCache, applySummarySnapshotToCache, type NormalizedConversationRecord } from '@/features/projects/model/projectsHomeState'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { cachedAppearance, applyTheme } from '@/lib/theme'
import { requestNavigation } from '@/state/workspaceSlice'
import { ATTENTION_CHANGED_EVENT } from '@/lib/api/attentionApi'
import { reviewProposedPlanValidated } from '@/lib/api/conversationsApi'
import { closeFlowOpening, primeFlowSaveBaseline, refusedFlowSaves, refusedFlowSaveStatus, resetFlowSaveBaselines, retryLastSaveContent, saveFlowContent, sendPendingFlowSaves, setInvalidDraft, waitForFlowSaves } from '@/lib/flowPersistence'
import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { parseConversationPageResponse, parseConversationSummarySnapshotResponse } from '@/lib/workspaceClient'
import { conversationAcceptance, conversationBatch, conversationMutationAck, conversationPage, conversationRow, summarySnapshot } from '@/test/conversationWire'
import { deferred, switchEnvironment } from '@/test/environments'

// Every write started in a remote environment reaches it, through its proxy, after a switch to local.
const REMOTE = '/workspace/api/environments/links/env-r/proxy'
const PROJECT = '/tmp/project'
const flow = (title: string) => ({ flow: { schema_version: '2', id: 'demo', title, nodes: {} } })

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
        primeFlowSaveBaseline('demo', flow('Local'), PROJECT)
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

    it('keeps a remote save refused after a switch away for that environment, for review only once the flow opens there, sending nothing', async () => {
        await switchEnvironment('env-r')
        primeFlowSaveBaseline('demo', flow('Base'), PROJECT)
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
        expect(refused).toMatchObject({ name: 'demo', projectPath: PROJECT, environmentId: 'env-r', message: 'disk full', kind: 'http' })
        expect(refusedFlowSaveStatus(refused)).toBe('hidden')

        // Back there, nothing retries it through the save state shown; opened anew, even as the edits found
        // it, it stays for review, and nothing is sent.
        await switchEnvironment('env-r')
        resetStoreForEnvironmentSwitch(false)
        sent = []
        await expect(retryLastSaveContent()).resolves.toBe(false)
        primeFlowSaveBaseline('demo', flow('Base'), PROJECT)
        expect(refusedFlowSaveStatus(refused)).toBe('hidden')
        await expect(retryLastSaveContent()).resolves.toBe(false)
        expect(sent).toEqual([])
    })

    it('never retries a save refused in its flow once the flow opens anew', async () => {
        useStore.setState({ activeFlow: 'demo', activeProjectPath: PROJECT, viewMode: 'editor' })
        primeFlowSaveBaseline('demo', flow('Base'), PROJECT)
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : json({ detail: 'unavailable' }, 503))
        await expect(saveFlowContent('demo', flow('Refused'), PROJECT)).resolves.toBe(false)
        expect(useStore.getState()).toMatchObject({ saveState: 'error', saveErrorMessage: 'unavailable' })
        primeFlowSaveBaseline('demo', flow('Elsewhere'), PROJECT)
        sent = []
        await expect(retryLastSaveContent()).resolves.toBe(false)
        expect(sent).toEqual([])
    })

    it('never sends an autosave whose preview answers after its flow opened anew with a newer version', async () => {
        useStore.setState({ activeFlow: 'demo', activeProjectPath: PROJECT })
        primeFlowSaveBaseline('demo', flow('Base'), PROJECT)
        const preview = deferred<unknown>()
        answer = (url) => (url.endsWith('/preview') ? preview.promise : { status: 'saved' })
        const saving = saveFlowContent('demo', flow('Old'), PROJECT)
        await waitFor(() => expect(sent).toEqual(['POST /attractor/preview']))
        // The editor leaves the flow and opens it again, now as saved elsewhere, while the preview answers.
        closeFlowOpening('demo', PROJECT, null)
        primeFlowSaveBaseline('demo', flow('Newer'), PROJECT)

        preview.resolve({ status: 'ok' })
        await expect(saving).resolves.toBe(false)
        expect(sent).toEqual(['POST /attractor/preview'])
        expect(refusedFlowSaves()).toMatchObject([{ content: flow('Old'), message: 'These edits weren’t sent: the flow opened again first.' }])
    })

    it('keeps the baseline a newer opening loaded when an older save’s acknowledgment lands after it', async () => {
        useStore.setState({ activeFlow: 'demo', activeProjectPath: PROJECT })
        primeFlowSaveBaseline('demo', flow('Base'), PROJECT)
        const saved = deferred<unknown>()
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : saved.promise)
        const saving = saveFlowContent('demo', flow('Old'), PROJECT)
        await waitFor(() => expect(sent).toHaveLength(2))
        closeFlowOpening('demo', PROJECT, null)
        primeFlowSaveBaseline('demo', flow('Newer'), PROJECT)
        saved.resolve({ status: 'saved' })
        await expect(saving).resolves.toBe(true)

        // An edit back to the old snapshot is a change over the version this opening loaded.
        sent = []
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : { status: 'saved' })
        await expect(saveFlowContent('demo', flow('Old'), PROJECT)).resolves.toBe(true)
        expect(sent).toEqual(['POST /attractor/preview', 'POST /attractor/api/flows'])
    })

    it('never sends a save queued before its flow opened anew, keeping it for the flow’s notice', async () => {
        await switchEnvironment('env-r')
        useStore.setState({ activeFlow: 'demo', activeProjectPath: PROJECT })
        primeFlowSaveBaseline('demo', flow('Base'), PROJECT)
        const firstSaved = deferred<unknown>()
        const flowSaves = () => sent.filter((request) => request.endsWith('/api/flows'))
        answer = (url) => (url.endsWith('/preview') ? { status: 'ok' } : flowSaves().length === 1 ? firstSaved.promise : { status: 'saved' })
        const first = saveFlowContent('demo', flow('One'), PROJECT)
        await waitFor(() => expect(flowSaves()).toHaveLength(1))
        const queued = saveFlowContent('demo', flow('Two'), PROJECT)
        // The editor leaves the flow and opens it again, now as saved elsewhere, before the queued save goes.
        closeFlowOpening('demo', PROJECT, 'env-r')
        useStore.getState().resetSaveState()
        primeFlowSaveBaseline('demo', flow('Elsewhere'), PROJECT)

        firstSaved.resolve(json({ detail: 'unavailable' }, 503))
        await expect(first).resolves.toBe(false)
        await expect(queued).resolves.toBe(false)
        expect(flowSaves()).toHaveLength(1)
        // The flow open now shows neither, nor retries them; its notice keeps the later edits for review.
        expect(useStore.getState().saveState).toBe('idle')
        await expect(retryLastSaveContent()).resolves.toBe(false)
        expect(flowSaves()).toHaveLength(1)
        const [refused] = refusedFlowSaves()
        expect(refused).toMatchObject({ content: flow('Two'), message: 'These edits weren’t sent: the flow opened again first.', kind: 'unknown' })
        expect(refusedFlowSaveStatus(refused)).toBe('hidden')
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

    // Back in the remote environment, once its project shows, the chat, which no list names here, opens again
    // with the refused message as its draft, and the error.
    await switchEnvironment('env-r')
    resetStoreForEnvironmentSwitch(false)
    useStore.getState().registerProject(PROJECT)
    expect(useStore.getState().projectSessionsByPath[PROJECT]?.conversationId).toBe('conversation-1')
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

it('leaves a newer send in the same chat pending and sending when a retired workspace\'s acknowledgment lands', async () => {
    const thread = (revision: number) => ({
        conversation_id: 'conversation-1', project_path: PROJECT, title: 'Thread', created_at: '2026-09-01T00:00:00Z',
        updated_at: '2026-09-01T00:00:00Z', revision, chat_mode: 'chat', turns: [], segments: [],
    })
    const turns = [deferred<unknown>(), deferred<unknown>()]
    const posts = () => sent.filter((request) => request.startsWith('POST') && request.endsWith('/conversations/conversation-1/turns'))
    answer = (url, method) => {
        if (url.includes('/projects/chat-models')) return { providers: { codex: { status: 'available', error: null, label: 'Codex' } }, models: [
            { provider: 'codex', id: 'gpt-5.4', display: 'GPT-5.4', is_default: true, supported_reasoning_efforts: ['medium'], default_reasoning_effort: 'medium' },
        ] }
        if (url.endsWith('/workspace/api/settings')) return { models: { scope: 'project', source: 'workspace', revision: '1', stored: null,
            effective: { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null } } }
        if (method === 'POST' && url.endsWith('/turns')) return turns[posts().length - 1].promise
        return {}
    }
    // The chat at revision 3, open in a freshly mounted workspace, which sends `text` as the `nth` turn.
    const send = async (text: string, nth: number) => {
        act(() => {
            useStore.getState().registerProject(PROJECT)
            useStore.getState().setActiveProjectPath(PROJECT)
            useStore.getState().updateProjectSessionState(PROJECT, { conversationId: 'conversation-1' })
            useStore.getState().commitHomeConversationCache((cache) => applyConversationPageToCache(cache, parseConversationPageResponse(conversationPage(thread(3))), 'window'))
        })
        const workspace = renderHook(() => useProjectsHomeController())
        await waitFor(() => expect(workspace.result.current.surfaceProps.isChatSendDisabled).toBe(false))
        act(() => workspace.result.current.surfaceProps.onChatDraftChange(text))
        act(() => workspace.result.current.surfaceProps.onChatComposerSubmit())
        await waitFor(() => expect(posts()).toHaveLength(nth))
        return workspace
    }
    const pendingTurn = () => useStore.getState().homeProjectSessionsByPath[PROJECT]?.pendingConversationTurn
    const accept = (index: number) => conversationAcceptance(thread(4 + index), (bodies[sent.indexOf(posts()[index])] as { client_turn_id: string }).client_turn_id)

    const retired = await send('First', 1)
    await switchEnvironment('env-r')
    resetStoreForEnvironmentSwitch(false)
    retired.unmount()
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    const shown = await send('Second', 2)
    expect(pendingTurn()).toMatchObject({ conversationId: 'conversation-1', afterRevision: '3' })

    // The retired workspace's answer, at a later revision, neither merges nor ends the newer send.
    turns[0].resolve(accept(0))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(useStore.getState().homeConversationCache.conversationsById['conversation-1']?.latestRevision).toBe('3')
    expect(pendingTurn()).toMatchObject({ conversationId: 'conversation-1', afterRevision: '3' })
    expect(useStore.getState().sendingChats).toEqual({ 'conversation-1': true })
    expect(shown.result.current.surfaceProps.isChatSendDisabled).toBe(true)

    turns[1].resolve(accept(1))
    await waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
    expect(pendingTurn()).toBeNull()
    shown.unmount()
})

it('keeps a refused send\'s text, files and error for the chat it was sent in, through a return and leave before the projects load', async () => {
    const project = (path: string, active: string | null) => ({ project_id: path, project_path: path, display_name: path, is_favorite: false, active_conversation_id: active })
    let serverChat = 'conversation-1'
    let projectsHeld: ReturnType<typeof deferred<unknown>> | null = null
    const turn = deferred<unknown>()
    const uploads = () => sent.filter((request) => request.startsWith('POST') && request.includes('/attachments'))
    const projectLoads = () => sent.filter((request) => request === 'GET /workspace/api/projects')
    answer = (url, method) => {
        if (url.includes('/projects/chat-models')) return { providers: { codex: { status: 'available', error: null, label: 'Codex' } }, models: [
            { provider: 'codex', id: 'gpt-5.4', display: 'GPT-5.4', is_default: true, supported_reasoning_efforts: ['medium'], default_reasoning_effort: 'medium' },
        ] }
        if (url.endsWith('/workspace/api/settings')) return { models: { scope: 'project', source: 'workspace', revision: '1', stored: null,
            effective: { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null } } }
        if (method === 'GET' && url.endsWith('/workspace/api/projects')) return projectsHeld?.promise ?? [project(PROJECT, serverChat), project('/tmp/other', null)]
        if (method === 'PATCH' && url.endsWith('/projects/state')) {
            serverChat = (bodies.at(-1) as { active_conversation_id?: string }).active_conversation_id ?? serverChat
            return project(PROJECT, serverChat)
        }
        if (method === 'POST' && url.includes('/attachments')) return { id: `attachment-${uploads().length}`, filename: 'shot.png', media_type: 'image/png', size_bytes: 5 }
        if (method === 'POST' && url.endsWith('/turns')) return turn.promise
        return {}
    }
    // The Chats view's workspace: it loads every project while it has none.
    const useWorkspace = () => {
        const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
        useProjectRegistryBootstrap({ hydrateProjectRegistry, enabled: useStore((state) => Object.keys(state.projectRegistry).length === 0) })
        return useProjectsHomeController()
    }
    const loaded = (workspace: { result: { current: ReturnType<typeof useWorkspace> } }) => waitFor(() => {
        expect(Object.keys(useStore.getState().projectRegistry)).toEqual([PROJECT, '/tmp/other'])
        expect(workspace.result.current.surfaceProps.isChatSendDisabled).toBe(false)
    })
    const show = async (environmentId: string | null) => {
        await switchEnvironment(environmentId)
        resetStoreForEnvironmentSwitch(environmentId === null)
    }
    useStore.setState({ activeProjectPath: PROJECT })
    const left = renderHook(useWorkspace, { wrapper: DialogProvider })
    await loaded(left)
    // The project's chat list names both chats, so either opens again from it.
    act(() => useStore.getState().commitHomeConversationCache((cache) => applySummarySnapshotToCache(cache, parseConversationSummarySnapshotResponse(summarySnapshot(
        ['conversation-1', 'conversation-2'].map((conversation_id) => ({ conversation_id, project_path: PROJECT, title: 'Thread', created_at: '2026-09-01T00:00:00Z', updated_at: '2026-09-01T00:00:00Z', revision: 3, turns: [], segments: [] })),
        { project_path: PROJECT },
    )), 'http', PROJECT)))
    act(() => left.result.current.surfaceProps.onChatDraftChange('Hello there'))
    act(() => left.result.current.surfaceProps.onAddAttachments([new File(['image'], 'shot.png', { type: 'image/png' })]))
    await waitFor(() => expect(left.result.current.surfaceProps.attachments).toMatchObject([{ status: 'uploaded' }]))
    act(() => left.result.current.surfaceProps.onChatComposerSubmit())
    await waitFor(() => expect(sent).toContain(`POST /workspace/api/conversations/conversation-1/turns`))
    // Another chat opens, then another environment shows; the send is refused meanwhile.
    act(() => left.result.current.sidebarProps.onSelectConversationThread(PROJECT, 'conversation-2'))
    await waitFor(() => expect(serverChat).toBe('conversation-2'))
    await show('env-r')
    left.unmount()
    turn.resolve(json({ detail: 'unavailable' }, 503))
    await new Promise((resolve) => setTimeout(resolve, 0))

    // Back briefly while this machine's projects are still loading, then away again before they answer.
    projectsHeld = deferred<unknown>()
    await show(null)
    const brief = renderHook(useWorkspace, { wrapper: DialogProvider })
    await waitFor(() => expect(projectLoads()).toHaveLength(2))
    expect(useStore.getState().homeProjectSessionsByPath).toEqual({})
    await show('env-r')
    brief.unmount()
    projectsHeld.resolve([project(PROJECT, serverChat), project('/tmp/other', null)])
    projectsHeld = null
    await new Promise((resolve) => setTimeout(resolve, 0))

    // Back again with the projects loaded: the chat opened since stays open, its draft its own.
    await show(null)
    const shown = renderHook(useWorkspace, { wrapper: DialogProvider })
    await loaded(shown)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(useStore.getState().projectSessionsByPath[PROJECT]?.conversationId).toBe('conversation-2')
    expect(shown.result.current.surfaceProps.panelError).toBeNull()
    expect(shown.result.current.surfaceProps.chatDraft).toBe('')
    expect(shown.result.current.surfaceProps.attachments).toEqual([])

    // The chat it was sent in, opened again, has its text, files and error back, uploaded once.
    act(() => shown.result.current.sidebarProps.onSelectConversationThread(PROJECT, 'conversation-1'))
    await waitFor(() => expect(shown.result.current.surfaceProps.attachments).toMatchObject([{ status: 'uploaded', attachment: { id: 'attachment-1' } }]))
    expect(shown.result.current.surfaceProps.chatDraft).toBe('Hello there')
    expect(shown.result.current.surfaceProps.panelError).toBe('unavailable')
    expect(uploads()).toHaveLength(1)
    expect(uploads()[0]).toContain('/workspace/api/conversations/conversation-1/attachments?')
    shown.unmount()
})

it('opens a new chat\'s late-refused first send again once its project, with no chat saved, opens its latest by itself on a return', async () => {
    const project = (path: string, active: string | null) => ({ project_id: path, project_path: path, display_name: path, is_favorite: false, active_conversation_id: active })
    let serverChat: string | null = 'conversation-1'
    const turn = deferred<unknown>()
    answer = (url, method) => {
        if (url.includes('/projects/chat-models')) return { providers: { codex: { status: 'available', error: null, label: 'Codex' } }, models: [
            { provider: 'codex', id: 'gpt-5.4', display: 'GPT-5.4', is_default: true, supported_reasoning_efforts: ['medium'], default_reasoning_effort: 'medium' },
        ] }
        if (url.endsWith('/workspace/api/settings')) return { models: { scope: 'project', source: 'workspace', revision: '1', stored: null,
            effective: { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null } } }
        if (method === 'GET' && url.endsWith('/workspace/api/projects')) return [project(PROJECT, serverChat), project('/tmp/other', null)]
        if (method === 'GET' && url.includes('/workspace/api/projects/conversations')) return summarySnapshot([
            { conversation_id: 'conversation-1', project_path: PROJECT, title: 'Thread', created_at: '2026-09-01T00:00:00Z', updated_at: '2026-09-01T00:00:00Z', revision: 3, turns: [], segments: [] },
        ], { project_path: PROJECT })
        if (method === 'GET' && url.includes('/workspace/api/conversations/')) return json({ detail: 'missing' }, 404)
        if (method === 'PATCH' && url.endsWith('/projects/state')) {
            // New chat saves as the project's chat; the chat its first message opens fails to.
            const chosen = (bodies.at(-1) as { active_conversation_id?: string | null }).active_conversation_id ?? null
            if (chosen !== null && chosen !== 'conversation-1') return json({ detail: 'unavailable' }, 503)
            serverChat = chosen
            return project(PROJECT, serverChat)
        }
        if (method === 'POST' && url.includes('/attachments')) return { id: 'attachment-1', filename: 'shot.png', media_type: 'image/png', size_bytes: 5 }
        if (method === 'POST' && url.endsWith('/turns')) return turn.promise
        return {}
    }
    // The Chats view's workspace, with the chat lists that open a project's latest chat when it has none saved.
    const useWorkspace = () => {
        const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
        useProjectRegistryBootstrap({ hydrateProjectRegistry, enabled: useStore((state) => Object.keys(state.projectRegistry).length === 0) })
        return useProjectsHomeController()
    }
    const wrapper = ({ children }: { children: ReactNode }) => <DialogProvider><HomeSessionController />{children}</DialogProvider>
    const openChat = () => useStore.getState().projectSessionsByPath[PROJECT]?.conversationId
    const show = async (environmentId: string | null) => {
        await switchEnvironment(environmentId)
        resetStoreForEnvironmentSwitch(environmentId === null)
        useStore.setState({ activeProjectPath: PROJECT, viewMode: 'home' })
    }
    useStore.setState({ activeProjectPath: PROJECT, viewMode: 'home' })
    const left = renderHook(useWorkspace, { wrapper })
    await waitFor(() => expect(openChat()).toBe('conversation-1'))
    await waitFor(() => expect(left.result.current.surfaceProps.isChatSendDisabled).toBe(false))
    act(() => left.result.current.sidebarProps.onCreateConversationThread(PROJECT))
    await waitFor(() => expect(serverChat).toBeNull())
    // Its first message, a file alone.
    act(() => left.result.current.surfaceProps.onAddAttachments([new File(['image'], 'shot.png', { type: 'image/png' })]))
    await waitFor(() => expect(left.result.current.surfaceProps.attachments).toMatchObject([{ status: 'uploaded' }]))
    const created = openChat()!
    act(() => left.result.current.surfaceProps.onChatComposerSubmit())
    await waitFor(() => expect(sent).toContain(`POST /workspace/api/conversations/${created}/turns`))
    await show('env-r')
    left.unmount()

    // Back: with no chat saved, the project opens its latest once its list loads; then the send is refused.
    await show(null)
    const shown = renderHook(useWorkspace, { wrapper })
    await waitFor(() => expect(openChat()).toBe('conversation-1'))
    turn.resolve(json({ detail: 'unavailable' }, 503))
    await waitFor(() => expect(openChat()).toBe(created))
    await waitFor(() => expect(shown.result.current.surfaceProps.attachments).toMatchObject([{ status: 'uploaded', attachment: { id: 'attachment-1' } }]))
    expect(shown.result.current.surfaceProps.chatDraft).toBe('')
    expect(shown.result.current.surfaceProps.panelError).toBe('unavailable')
    shown.unmount()
})

it('keeps a new chat\'s late-refused first send, opened again on a return, when the chat its project opened by itself, before the switch and on the return, saves after', async () => {
    const project = (path: string, active: string | null) => ({ project_id: path, project_path: path, display_name: path, is_favorite: false, active_conversation_id: active })
    let serverChat: string | null = null
    const turn = deferred<unknown>()
    // Each save of the project's latest, opened by itself, answers only when the test says.
    const latestSaves: ReturnType<typeof deferred<unknown>>[] = []
    const saves = () => sent.filter((request) => request.startsWith('PATCH') && request.endsWith('/projects/state'))
    answer = (url, method) => {
        if (url.includes('/projects/chat-models')) return { providers: { codex: { status: 'available', error: null, label: 'Codex' } }, models: [
            { provider: 'codex', id: 'gpt-5.4', display: 'GPT-5.4', is_default: true, supported_reasoning_efforts: ['medium'], default_reasoning_effort: 'medium' },
        ] }
        if (url.endsWith('/workspace/api/settings')) return { models: { scope: 'project', source: 'workspace', revision: '1', stored: null,
            effective: { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null } } }
        if (method === 'GET' && url.endsWith('/workspace/api/projects')) return [project(PROJECT, serverChat), project('/tmp/other', null)]
        if (method === 'GET' && url.includes('/workspace/api/projects/conversations')) return summarySnapshot([
            { conversation_id: 'conversation-1', project_path: PROJECT, title: 'Thread', created_at: '2026-09-01T00:00:00Z', updated_at: '2026-09-01T00:00:00Z', revision: 3, turns: [], segments: [] },
        ], { project_path: PROJECT })
        if (method === 'GET' && url.includes('/workspace/api/conversations/')) return json({ detail: 'missing' }, 404)
        if (method === 'PATCH' && url.endsWith('/projects/state')) {
            const chosen = (bodies.at(-1) as { active_conversation_id?: string | null }).active_conversation_id ?? null
            if (chosen === 'conversation-1') {
                latestSaves.push(deferred<unknown>())
                return latestSaves.at(-1)!.promise
            }
            // New chat saves as the project's chat; the chat its first message opens fails to.
            if (chosen !== null) return json({ detail: 'unavailable' }, 503)
            serverChat = chosen
            return project(PROJECT, serverChat)
        }
        if (method === 'POST' && url.includes('/attachments')) return { id: 'attachment-1', filename: 'shot.png', media_type: 'image/png', size_bytes: 5 }
        if (method === 'POST' && url.endsWith('/turns')) return turn.promise
        return {}
    }
    const useWorkspace = () => {
        const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
        useProjectRegistryBootstrap({ hydrateProjectRegistry, enabled: useStore((state) => Object.keys(state.projectRegistry).length === 0) })
        return useProjectsHomeController()
    }
    const wrapper = ({ children }: { children: ReactNode }) => <DialogProvider><HomeSessionController />{children}</DialogProvider>
    const openChat = () => useStore.getState().projectSessionsByPath[PROJECT]?.conversationId
    const show = async (environmentId: string | null) => {
        await switchEnvironment(environmentId)
        resetStoreForEnvironmentSwitch(environmentId === null)
        useStore.setState({ activeProjectPath: PROJECT, viewMode: 'home' })
    }
    // With no chat saved, the project opens its latest by itself, saving it; that save is slow.
    useStore.setState({ activeProjectPath: PROJECT, viewMode: 'home' })
    const left = renderHook(useWorkspace, { wrapper })
    await waitFor(() => expect(openChat()).toBe('conversation-1'))
    await waitFor(() => expect(saves()).toHaveLength(1))
    await waitFor(() => expect(left.result.current.surfaceProps.isChatSendDisabled).toBe(false))
    act(() => left.result.current.sidebarProps.onCreateConversationThread(PROJECT))
    await waitFor(() => expect(saves()).toHaveLength(2))
    // Its first message, with a file.
    act(() => left.result.current.surfaceProps.onChatDraftChange('Hello there'))
    act(() => left.result.current.surfaceProps.onAddAttachments([new File(['image'], 'shot.png', { type: 'image/png' })]))
    await waitFor(() => expect(left.result.current.surfaceProps.attachments).toMatchObject([{ status: 'uploaded' }]))
    const created = openChat()!
    act(() => left.result.current.surfaceProps.onChatComposerSubmit())
    await waitFor(() => expect(sent).toContain(`POST /workspace/api/conversations/${created}/turns`))
    await show('env-r')
    left.unmount()

    // Back, the project, still with no chat saved, opens its latest by itself again, saving it; then the send is refused.
    await show(null)
    const shown = renderHook(useWorkspace, { wrapper })
    await waitFor(() => expect(openChat()).toBe('conversation-1'))
    await waitFor(() => expect(latestSaves).toHaveLength(2))
    turn.resolve(json({ detail: 'unavailable' }, 503))
    await waitFor(() => expect(openChat()).toBe(created))
    const recovered = async () => {
        await waitFor(() => expect(shown.result.current.surfaceProps.attachments).toMatchObject([{ status: 'uploaded', attachment: { id: 'attachment-1' } }]))
        expect(openChat()).toBe(created)
        expect(shown.result.current.surfaceProps.chatDraft).toBe('Hello there')
        expect(shown.result.current.surfaceProps.panelError).toBe('unavailable')
    }
    await recovered()

    // The saves answer now, the return's first: neither moves the chat opened since.
    for (const save of [latestSaves[1], latestSaves[0]]) {
        await act(async () => save.resolve(project(PROJECT, 'conversation-1')))
        await new Promise((resolve) => setTimeout(resolve, 0))
        await recovered()
    }
    shown.unmount()
})

it.each([[0, 4, 5], [1, 5, 4], [0, 5, 6]])('keeps a restored chat open and sending, its newer send pending, until both its sends a live update let overlap settle, send %i answering first at revision %i, when an unlisted chat\'s send is refused late', async (first, early, late) => {
    const project = (path: string, active: string | null) => ({ project_id: path, project_path: path, display_name: path, is_favorite: false, active_conversation_id: active })
    const thread = (revision: number) => ({
        conversation_id: 'conversation-1', project_path: PROJECT, title: 'Thread', created_at: '2026-09-01T00:00:00Z',
        updated_at: '2026-09-01T00:00:00Z', revision, chat_mode: 'chat', turns: [], segments: [],
    })
    let serverChat: string | null = 'conversation-1'
    const refused = deferred<unknown>()
    const turns = [deferred<unknown>(), deferred<unknown>()]
    const posts = () => sent.filter((request) => request.startsWith('POST') && request.endsWith('/conversations/conversation-1/turns'))
    answer = (url, method) => {
        if (url.includes('/projects/chat-models')) return { providers: { codex: { status: 'available', error: null, label: 'Codex' } }, models: [
            { provider: 'codex', id: 'gpt-5.4', display: 'GPT-5.4', is_default: true, supported_reasoning_efforts: ['medium'], default_reasoning_effort: 'medium' },
        ] }
        if (url.endsWith('/workspace/api/settings')) return { models: { scope: 'project', source: 'workspace', revision: '1', stored: null,
            effective: { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null } } }
        if (method === 'GET' && url.endsWith('/workspace/api/projects')) return [project(PROJECT, serverChat), project('/tmp/other', null)]
        if (method === 'GET' && url.includes('/workspace/api/projects/conversations')) return summarySnapshot([thread(3)], { project_path: PROJECT })
        if (method === 'GET' && url.includes('/workspace/api/conversations/conversation-1?')) return conversationPage(thread(3))
        if (method === 'GET' && url.includes('/workspace/api/conversations/')) return json({ detail: 'missing' }, 404)
        if (method === 'PATCH' && url.endsWith('/projects/state')) {
            // New chat saves as the project's chat; the chat its first message opens fails to.
            const chosen = (bodies.at(-1) as { active_conversation_id?: string | null }).active_conversation_id ?? null
            if (chosen !== null && chosen !== 'conversation-1') return json({ detail: 'unavailable' }, 503)
            serverChat = chosen
            return project(PROJECT, serverChat)
        }
        if (method === 'POST' && url.includes('/attachments')) return { id: 'attachment-1', filename: 'shot.png', media_type: 'image/png', size_bytes: 5 }
        if (method === 'POST' && url.endsWith('/conversations/conversation-1/turns')) return turns[posts().length - 1].promise
        if (method === 'POST' && url.endsWith('/turns')) return refused.promise
        return {}
    }
    const useWorkspace = () => {
        const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
        useProjectRegistryBootstrap({ hydrateProjectRegistry, enabled: useStore((state) => Object.keys(state.projectRegistry).length === 0) })
        return useProjectsHomeController()
    }
    const wrapper = ({ children }: { children: ReactNode }) => <DialogProvider><HomeSessionController />{children}</DialogProvider>
    const openChat = () => useStore.getState().projectSessionsByPath[PROJECT]?.conversationId
    const pendingTurn = () => useStore.getState().homeProjectSessionsByPath[PROJECT]?.pendingConversationTurn
    const accept = (index: number, revision: number) => conversationAcceptance(thread(revision), (bodies[sent.indexOf(posts()[index])] as { client_turn_id: string }).client_turn_id)
    const show = async (environmentId: string | null) => {
        await switchEnvironment(environmentId)
        resetStoreForEnvironmentSwitch(environmentId === null)
        useStore.setState({ activeProjectPath: PROJECT, viewMode: 'home' })
    }
    useStore.setState({ activeProjectPath: PROJECT, viewMode: 'home' })
    const left = renderHook(useWorkspace, { wrapper })
    await waitFor(() => expect(openChat()).toBe('conversation-1'))
    await waitFor(() => expect(left.result.current.surfaceProps.isChatSendDisabled).toBe(false))
    act(() => left.result.current.sidebarProps.onCreateConversationThread(PROJECT))
    await waitFor(() => expect(serverChat).toBeNull())
    // Its first message, with a file.
    act(() => left.result.current.surfaceProps.onChatDraftChange('Hello there'))
    act(() => left.result.current.surfaceProps.onAddAttachments([new File(['image'], 'shot.png', { type: 'image/png' })]))
    await waitFor(() => expect(left.result.current.surfaceProps.attachments).toMatchObject([{ status: 'uploaded' }]))
    const created = openChat()!
    act(() => left.result.current.surfaceProps.onChatComposerSubmit())
    await waitFor(() => expect(sent).toContain(`POST /workspace/api/conversations/${created}/turns`))
    await show('env-r')
    left.unmount()

    // Back, the project opens its latest chat by itself, which sends a message.
    await show(null)
    const shown = renderHook(useWorkspace, { wrapper })
    await waitFor(() => expect(openChat()).toBe('conversation-1'))
    await waitFor(() => expect(useStore.getState().homeConversationCache.conversationsById['conversation-1']?.latestRevision).toBe('3'))
    const send = async (text: string, count: number) => {
        await waitFor(() => expect(shown.result.current.surfaceProps.isChatSendDisabled).toBe(false))
        act(() => shown.result.current.surfaceProps.onChatDraftChange(text))
        act(() => shown.result.current.surfaceProps.onChatComposerSubmit())
        await waitFor(() => expect(posts()).toHaveLength(count))
    }
    await send('First', 1)
    expect(pendingTurn()).toMatchObject({ conversationId: 'conversation-1', afterRevision: '3' })
    // A live update records it before its answer, so Send opens again, for a second message.
    act(() => {
        window.dispatchEvent(new CustomEvent('spark:conversation-live-event', { detail: {
            type: 'conversation.changes', conversationId: 'conversation-1', projectPath: PROJECT, payload: conversationBatch([conversationRow(thread(4))]),
        } }))
    })
    await send('Second', 2)
    expect(pendingTurn()).toMatchObject({ conversationId: 'conversation-1', afterRevision: '4' })

    // One send's answer lands while the other is in flight; then the unlisted chat's send is refused.
    // The second send's answer ends its pending turn; the first's leaves it, even past where it started.
    const pendingAfterFirst = first === 0 ? { conversationId: 'conversation-1', afterRevision: String(early) } : null
    await act(async () => turns[first].resolve(accept(first, early)))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(pendingTurn() ?? null).toEqual(pendingAfterFirst)
    expect(useStore.getState().sendingChats).toEqual({ 'conversation-1': true })
    await act(async () => refused.resolve(json({ detail: 'unavailable' }, 503)))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(openChat()).toBe('conversation-1')
    expect(pendingTurn() ?? null).toEqual(pendingAfterFirst)
    expect(useStore.getState().sendingChats).toEqual({ 'conversation-1': true })
    expect(shown.result.current.surfaceProps.panelError).toBeNull()
    await act(async () => turns[1 - first].resolve(accept(1 - first, late)))
    await waitFor(() => expect(useStore.getState().sendingChats).toEqual({}))
    expect(pendingTurn()).toBeNull()
    expect(openChat()).toBe('conversation-1')
    await show('env-r')
    shown.unmount()

    // The next return opens the refused chat with its text, file and error.
    await show(null)
    const next = renderHook(useWorkspace, { wrapper })
    await waitFor(() => expect(openChat()).toBe(created))
    await waitFor(() => expect(next.result.current.surfaceProps.attachments).toMatchObject([{ status: 'uploaded', attachment: { id: 'attachment-1' } }]))
    expect(next.result.current.surfaceProps.chatDraft).toBe('Hello there')
    expect(next.result.current.surfaceProps.panelError).toBe('unavailable')
    next.unmount()
})

it('makes a project the last used when its save answers after a switch away and back, keeping its open chat and any newer use or pin', async () => {
    const project = (path: string, usedAt: string, more: object = {}) => ({ project_id: path, project_path: path, display_name: path, is_favorite: false, active_conversation_id: null, last_accessed_at: usedAt, ...more })
    const saves: ReturnType<typeof deferred<unknown>>[] = []
    const listed = deferred<unknown>()
    answer = (url, method) => {
        if (method === 'GET' && url.endsWith('/workspace/api/projects')) return listed.promise
        if (method === 'PATCH' && url.endsWith('/projects/state')) {
            saves.push(deferred<unknown>())
            return saves.at(-1)!.promise
        }
        return {}
    }
    // The Chats view's workspace: it loads every project while it has none.
    const useWorkspace = () => {
        const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
        useProjectRegistryBootstrap({ hydrateProjectRegistry, enabled: useStore((state) => Object.keys(state.projectRegistry).length === 0) })
        return useProjectsHomeController()
    }
    const show = async (environmentId: string | null) => {
        await switchEnvironment(environmentId)
        resetStoreForEnvironmentSwitch(environmentId === null)
        useStore.setState({ activeProjectPath: PROJECT })
    }
    const registry = () => useStore.getState().projectRegistry
    // Runs start in the other project, then twice in this one, its chat then conversation-1.
    const used = [markProjectUsed('/tmp/other'), markProjectUsed(PROJECT), markProjectUsed(PROJECT)]
    await waitFor(() => expect(saves).toHaveLength(3))
    const answered = async (index: number, record: unknown) => act(async () => {
        saves[index].resolve(record)
        await used[index]
    })
    // Another environment shows, then this one again, its projects loading; the other project's save answers meanwhile.
    await show('env-r')
    await show(null)
    const shown = renderHook(useWorkspace, { wrapper: DialogProvider })
    await waitFor(() => expect(sent).toContain('GET /workspace/api/projects'))
    await answered(0, project('/tmp/other', '2026-10-15T00:00:00Z'))

    // The list, from before the saves, loads in full: the other project was used last, this one pinned, conversation-2 open.
    await act(async () => listed.resolve([
        project(PROJECT, '2026-09-01T00:00:00Z', { is_favorite: true, active_conversation_id: 'conversation-2' }),
        project('/tmp/other', '2026-10-01T00:00:00Z'),
    ]))
    await waitFor(() => expect(Object.keys(registry())).toEqual([PROJECT, '/tmp/other']))
    expect(registry()['/tmp/other']?.lastAccessedAt).toBe('2026-10-15T00:00:00Z')
    expect(defaultProjectChoice(registry())).toBe('/tmp/other')
    expect(shown.result.current.sidebarProps.activeConversationId).toBe('conversation-2')

    // This project's later save answers: it becomes the last used, its open chat still selected.
    await answered(2, project(PROJECT, '2026-11-02T00:00:00Z', { active_conversation_id: 'conversation-1' }))
    expect(defaultProjectChoice(registry())).toBe(PROJECT)
    expect(useStore.getState().activeProjectPath).toBe(PROJECT)
    expect(shown.result.current.sidebarProps.activeConversationId).toBe('conversation-2')

    // Its earlier save answers last, with an older use.
    await answered(1, project(PROJECT, '2026-11-01T00:00:00Z', { active_conversation_id: 'conversation-1' }))
    expect(defaultProjectChoice(registry())).toBe(PROJECT)
    expect(registry()[PROJECT]).toMatchObject({ lastAccessedAt: '2026-11-02T00:00:00Z', isFavorite: true })
    expect(shown.result.current.sidebarProps.activeConversationId).toBe('conversation-2')
    shown.unmount()
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
