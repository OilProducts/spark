import { create } from 'zustand'
import { createEmptyTriggerForm } from '@/features/triggers/model/triggerForm'
import { isRemoteEnvironmentActive } from './lib/activeEnvironment'
import { createEditorSlice } from './state/editorSlice'
import { createHomeSessionSlice } from './state/homeSessionSlice'
import { createRunsSessionSlice } from './state/runsSessionSlice'
import {
    DEFAULT_UI_DEFAULTS,
    DEFAULT_WORKING_DIRECTORY,
    loadRouteState,
    resolveProjectSessionState,
    saveRouteState,
    saveSessionViewMode,
} from './state/store-helpers'
import type { AppState, RouteState } from './state/store-types'
import { createTriggersSessionSlice } from './state/triggersSessionSlice'
import { createWorkspaceSlice } from './state/workspaceSlice'

export * from './state/store-types'
export * from './state/viewSessionTypes'

export const useStore = create<AppState>()((...args) => ({
    ...createWorkspaceSlice(...args),
    ...createHomeSessionSlice(...args),
    ...createRunsSessionSlice(...args),
    ...createTriggersSessionSlice(...args),
    ...createEditorSlice(...args),
}))

export const selectRouteState = (state: AppState): RouteState => ({
    activeProjectPath: state.activeProjectPath,
    projectPagePath: state.projectPagePath,
    selectedRunId: state.runsListSession.selectedRunId,
    selectedTriggerId: state.triggersSession.selectedTriggerId,
    selectedMission: state.selectedMission,
    activeFlow: state.activeFlow,
    settingsCategory: state.settingsCategory,
})

// The app opens on the Overview, or the view you reloaded on; each view reopens on its last selection.
const restoredRouteState = loadRouteState()
if (restoredRouteState.selectedRunId) {
    useStore.getState().setRunsSelectedRunId(restoredRouteState.selectedRunId)
}
if (restoredRouteState.selectedTriggerId) {
    useStore.getState().updateTriggersSession({ selectedTriggerId: restoredRouteState.selectedTriggerId })
}
let savedRouteState = JSON.stringify(selectRouteState(useStore.getState()))
let suppressRouteSave = false
useStore.subscribe((state, previous) => {
    if (state.viewMode !== previous.viewMode) saveSessionViewMode(state.viewMode)
    // Never overwrite local route persistence during switches or while viewing a remote environment.
    if (suppressRouteSave || isRemoteEnvironmentActive()) return
    const next = selectRouteState(state)
    const serialized = JSON.stringify(next)
    if (serialized !== savedRouteState) {
        savedRouteState = serialized
        saveRouteState(next)
    }
})

/** Clears environment-scoped session state so switching environments starts clean without leaking data. */
export function resetStoreForEnvironmentSwitch(restoreLocalRoute: boolean) {
    const route = restoreLocalRoute ? loadRouteState() : null
    const activeProjectPath = route?.activeProjectPath ?? null
    const projectSessionsByPath = activeProjectPath
        ? { [activeProjectPath]: resolveProjectSessionState({}, activeProjectPath) }
        : {}
    const workingDir = activeProjectPath
        ? projectSessionsByPath[activeProjectPath].workingDir
        : DEFAULT_WORKING_DIRECTORY
    suppressRouteSave = true
    try {
        useStore.setState({
        activeProjectPath,
        projectPagePath: route?.projectPagePath ?? null,
        projectPageSettingsRequest: null,
        selectedMission: route?.selectedMission ?? null,
        selectedRole: null,
        chatOriginProjectPath: null,
        missionBoard: [],
        missionBoardLoaded: false,
        projectRegistry: {},
        recentProjectPaths: activeProjectPath ? [activeProjectPath] : [],
        projectSessionsByPath,
        projectRegistrationError: null,
        activeFlow: route?.activeFlow ?? null,
        flowLoadFailure: null,
        workingDir,
        homeConversationCache: {
            conversationsById: {},
            summariesByProjectPath: {},
        },
        homeThreadSummariesStatusByProjectPath: {},
        homeThreadSummariesErrorByProjectPath: {},
        homeProjectSessionsByPath: {},
        homeConversationSessionsById: {},
        homeProjectGitMetadataByPath: {},
        sendingChats: {},
        runsListSession: {
            selectedRunId: null,
            projectFilter: null,
            status: 'idle',
            error: null,
            runs: [],
        },
        runDetailSessionsByRunId: {},
        triggersSession: {
            status: 'idle',
            error: null,
            triggers: [],
            selectedTriggerId: route?.selectedTriggerId ?? null,
            revealedWebhookSecrets: {},
            createFormOpen: false,
            newTriggerDraft: {
                form: createEmptyTriggerForm(null),
            },
            editTriggerDraftsByTriggerId: {},
        },
        clientPreferencesLoaded: false,
        clientFlowNodePositions: {},
        clientFlowEdgePorts: {},
        chatProjectOrder: [],
        clientRunPresentation: {},
        selectedNodeId: null,
        selectedEdgeId: null,
        pendingEditorNodeSelection: null,
        flowMetadata: {},
        flowMetadataErrors: {},
        graphAttrs: {},
        graphAttrErrors: {},
        diagnostics: [],
        nodeDiagnostics: {},
        edgeDiagnostics: {},
        hasValidationErrors: false,
        saveState: 'idle',
        saveErrorMessage: null,
        saveErrorKind: null,
        saveErrorProblems: null,
        uiDefaults: { ...DEFAULT_UI_DEFAULTS },
        editorGraphSettingsPanelOpenByFlow: {},
        editorExpandChildFlowsByFlow: {},
        editorShowAdvancedFlowMetadataByFlow: {},
        editorShowAdvancedGraphAttrsByFlow: {},
        editorLaunchInputDraftsByFlow: {},
        editorLaunchInputDraftErrorByFlow: {},
        editorNodeInspectorSessionsByNodeId: {},
        })
        if (route?.selectedRunId) {
            useStore.getState().setRunsSelectedRunId(route.selectedRunId)
        }
        savedRouteState = JSON.stringify(selectRouteState(useStore.getState()))
    } finally {
        suppressRouteSave = false
    }
}
