import { create } from 'zustand'
import { getActiveEnvironmentId, isRemoteEnvironmentActive } from './lib/activeEnvironment'
import { createEditorSlice, initialEditorSessionState } from './state/editorSlice'
import { createHomeSessionSlice, initialHomeSessionState } from './state/homeSessionSlice'
import { createRunsSessionSlice, initialRunsSessionState } from './state/runsSessionSlice'
import {
    DEFAULT_ROUTE_STATE,
    loadRouteState,
    saveRouteState,
    saveSessionViewMode,
} from './state/store-helpers'
import type { AppState, RouteState } from './state/store-types'
import { createTriggersSessionSlice, initialTriggersSessionState } from './state/triggersSessionSlice'
import { createWorkspaceSlice, initialWorkspaceState } from './state/workspaceSlice'

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

/**
 * Captures the environment and project shown; the check it returns says whether both still are, so
 * work that finishes later writes nothing into what replaced them.
 */
export function captureShownWorkspace(): () => boolean {
    const environmentId = getActiveEnvironmentId()
    const projectPath = useStore.getState().activeProjectPath
    return () => getActiveEnvironmentId() === environmentId && useStore.getState().activeProjectPath === projectPath
}

/** Reopens the route's run and trigger, which their sessions hold. */
function restoreRouteSelections(route: RouteState) {
    if (route.selectedRunId) {
        useStore.getState().setRunsSelectedRunId(route.selectedRunId)
    }
    if (route.selectedTriggerId) {
        useStore.getState().updateTriggersSession({ selectedTriggerId: route.selectedTriggerId })
    }
}

// The app opens on the Overview, or the view you reloaded on; each view reopens on its last selection.
restoreRouteSelections(loadRouteState())
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

/**
 * Puts every environment-scoped slice back to its fresh state, so switching environments starts
 * clean without leaking data. App-wide preferences, such as the appearance, are kept.
 */
export function resetStoreForEnvironmentSwitch(restoreLocalRoute: boolean) {
    const route = restoreLocalRoute ? loadRouteState() : DEFAULT_ROUTE_STATE
    suppressRouteSave = true
    try {
        useStore.setState({
            ...initialWorkspaceState(route),
            ...initialHomeSessionState(),
            ...initialRunsSessionState(),
            ...initialTriggersSessionState(),
            ...initialEditorSessionState(route),
        })
        restoreRouteSelections(route)
        savedRouteState = JSON.stringify(selectRouteState(useStore.getState()))
    } finally {
        suppressRouteSave = false
    }
}
