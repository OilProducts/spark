import { create } from 'zustand'
import { createEditorSlice } from './state/editorSlice'
import { createHomeSessionSlice } from './state/homeSessionSlice'
import { createRunInspectorSlice } from './state/runInspectorSlice'
import { createRunsSessionSlice } from './state/runsSessionSlice'
import type { AppState } from './state/store-types'
import { createTriggersSessionSlice } from './state/triggersSessionSlice'
import { createWorkspaceSlice } from './state/workspaceSlice'
import { createWorkflowEventLogSlice } from './state/workflowEventLogSlice'

export * from './state/store-types'
export * from './state/viewSessionTypes'

export const useStore = create<AppState>()((...args) => ({
    ...createWorkspaceSlice(...args),
    ...createWorkflowEventLogSlice(...args),
    ...createHomeSessionSlice(...args),
    ...createRunInspectorSlice(...args),
    ...createRunsSessionSlice(...args),
    ...createTriggersSessionSlice(...args),
    ...createEditorSlice(...args),
}))
