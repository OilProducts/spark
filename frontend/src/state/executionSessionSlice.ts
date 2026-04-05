import { type StateCreator } from 'zustand'
import type { AppState } from './store-types'
import type { ExecutionSessionSlice } from './viewSessionTypes'

export const createExecutionSessionSlice: StateCreator<AppState, [], [], ExecutionSessionSlice> = (set) => ({
    executionLaunchInputValues: {},
    executionLaunchError: null,
    executionLastLaunchFailure: null,
    executionRunStartGitPolicyWarning: null,
    executionCollapsedLaunchInputsByFlow: {},
    executionGraphCollapsed: false,
    executionOpenRunsAfterLaunch: false,
    executionLaunchSuccessRunId: null,
    updateExecutionSession: (patch) => set(() => patch),
})
