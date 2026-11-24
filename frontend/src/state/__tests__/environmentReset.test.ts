import { beforeEach, expect, it } from 'vitest'
import { applyTheme, cachedAppearance } from '@/lib/theme'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { switchEnvironment } from '@/test/environments'
import { initialEditorSessionState } from '../editorSlice'
import { initialHomeSessionState } from '../homeSessionSlice'
import { initialRunsSessionState } from '../runsSessionSlice'
import { initialTriggersSessionState } from '../triggersSessionSlice'
import { initialWorkspaceState } from '../workspaceSlice'

const freshEnvironmentState = () => ({
    ...initialWorkspaceState(),
    ...initialHomeSessionState(),
    ...initialRunsSessionState(),
    ...initialTriggersSessionState(),
    ...initialEditorSessionState(),
})

beforeEach(() => {
    useStore.setState(useStore.getInitialState(), true)
})

it('starts a switched-to environment from fresh state, keeping the app-wide preferences', async () => {
    const store = useStore.getState()
    applyTheme('dark')
    useStore.setState({
        viewMode: 'runs',
        activeProjectPath: '/local/project',
        projectPagePath: '/local/project',
        recentProjectPaths: ['/local/project'],
        activeFlow: 'local.yaml',
        settingsCategory: 'execution',
        missionBoardLoaded: true,
        graphAttrs: { title: 'Local' },
        hasValidationErrors: true,
        clientPreferencesLoaded: true,
        chatProjectOrder: ['/local/project'],
        editorSidebarWidth: 400,
        preferredEditorMode: 'files',
        preferredShowThinking: false,
    })
    store.updateHomeProjectSession('/local/project', { chatDraft: 'Local draft' })
    store.setSendingChat('conversation-1', true)
    store.setRunsSelectedRunId('run-1')
    store.updateTriggersSession({ selectedTriggerId: 'trigger-1' })
    store.markSaveFailure('Local failure', 'http')

    await switchEnvironment('env-r')
    resetStoreForEnvironmentSwitch(false)

    const state = useStore.getState()
    const fresh = freshEnvironmentState()
    expect(Object.fromEntries(Object.keys(fresh).map((key) => [key, state[key as keyof typeof state]]))).toEqual(fresh)
    expect(state).toMatchObject({ editorSidebarWidth: 400, preferredEditorMode: 'files', preferredShowThinking: false })
    expect(cachedAppearance()).toBe('dark')

    // Back on this machine, its own route and view come back.
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    expect(useStore.getState()).toMatchObject({
        viewMode: 'runs',
        activeProjectPath: '/local/project',
        activeFlow: 'local.yaml',
        settingsCategory: 'execution',
        runsListSession: { selectedRunId: 'run-1' },
        triggersSession: { selectedTriggerId: 'trigger-1' },
    })
})
