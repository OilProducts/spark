import { create } from 'zustand'
import { createEditorSlice } from './state/editorSlice'
import { createExecutionLaunchSlice } from './state/executionLaunchSlice'
import { createExecutionSessionSlice } from './state/executionSessionSlice'
import { createHomeSessionSlice } from './state/homeSessionSlice'
import { createRunInspectorSlice } from './state/runInspectorSlice'
import { createRunsSessionSlice } from './state/runsSessionSlice'
import type { AppState } from './state/store-types'
import { createTriggersSessionSlice } from './state/triggersSessionSlice'
import { createWorkspaceSlice } from './state/workspaceSlice'

export * from './state/store-types'
export * from './state/viewSessionTypes'

export const useStore = create<AppState>()((...args) => ({
    ...createWorkspaceSlice(...args),
    ...createHomeSessionSlice(...args),
    ...createExecutionLaunchSlice(...args),
    ...createExecutionSessionSlice(...args),
    ...createRunInspectorSlice(...args),
    ...createRunsSessionSlice(...args),
    ...createTriggersSessionSlice(...args),
    ...createEditorSlice(...args),
}))
