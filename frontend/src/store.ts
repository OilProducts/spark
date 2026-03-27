import { create } from 'zustand'
import { createEditorSlice } from './state/editorSlice'
import { createExecutionLaunchSlice } from './state/executionLaunchSlice'
import { createRunInspectorSlice } from './state/runInspectorSlice'
import type { AppState } from './state/store-types'
import { createWorkspaceSlice } from './state/workspaceSlice'

export * from './state/store-types'

export const useStore = create<AppState>()((...args) => ({
    ...createWorkspaceSlice(...args),
    ...createExecutionLaunchSlice(...args),
    ...createRunInspectorSlice(...args),
    ...createEditorSlice(...args),
}))
