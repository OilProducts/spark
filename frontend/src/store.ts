import { create } from 'zustand'
import { createEditorSlice } from './state/editorSlice'
import { createHomeSessionSlice } from './state/homeSessionSlice'
import { createRunsSessionSlice } from './state/runsSessionSlice'
import { loadRouteState, saveRouteState } from './state/store-helpers'
import type { AppState, RouteState } from './state/store-types'
import { createTriggersSessionSlice } from './state/triggersSessionSlice'
import { createWorkspaceSlice } from './state/workspaceSlice'
import { createWorkflowEventLogSlice } from './state/workflowEventLogSlice'

export * from './state/store-types'
export * from './state/viewSessionTypes'

export const useStore = create<AppState>()((...args) => ({
    ...createWorkspaceSlice(...args),
    ...createWorkflowEventLogSlice(...args),
    ...createHomeSessionSlice(...args),
    ...createRunsSessionSlice(...args),
    ...createTriggersSessionSlice(...args),
    ...createEditorSlice(...args),
}))

export const selectRouteState = (state: AppState): RouteState => ({
    viewMode: state.viewMode,
    activeProjectPath: state.activeProjectPath,
    projectPagePath: state.projectPagePath,
    selectedRunId: state.runsListSession.selectedRunId,
    selectedTriggerId: state.triggersSession.selectedTriggerId,
    selectedMission: state.selectedMission,
    activeFlow: state.activeFlow,
    settingsCategory: state.settingsCategory,
})

// The app reopens where you were: the view and each view's last selection.
const restoredRouteState = loadRouteState()
if (restoredRouteState.selectedRunId) {
    useStore.getState().setRunsSelectedRunId(restoredRouteState.selectedRunId)
}
if (restoredRouteState.selectedTriggerId) {
    useStore.getState().updateTriggersSession({ selectedTriggerId: restoredRouteState.selectedTriggerId })
}
let savedRouteState = JSON.stringify(selectRouteState(useStore.getState()))
useStore.subscribe((state) => {
    const next = selectRouteState(state)
    const serialized = JSON.stringify(next)
    if (serialized !== savedRouteState) {
        savedRouteState = serialized
        saveRouteState(next)
    }
})
