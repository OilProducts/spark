import { afterEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'

const initial = useStore.getState()
afterEach(() => { useStore.setState(initial, true); vi.restoreAllMocks() })

it('persists completed presentation toggles but leaves node drafts and hydration outside settings', () => {
    const completed = vi.fn()
    window.addEventListener('spark:preferences-completed', completed)
    try {
        useStore.setState({ preferredAdvancedControls: true, preferredExpandChildFlows: true })
        useStore.getState().updateEditorNodeInspectorSession('node-a', { readsContextDraft: 'unsaved context' })
        expect(completed).not.toHaveBeenCalled()
        expect(useStore.getState().editorNodeInspectorSessionsByNodeId['node-a'].showAdvanced).toBe(true)
        useStore.getState().setEditorExpandChildFlows('flow-a', false)
        useStore.getState().setEditorGraphSettingsPanelOpen('flow-a', true)
        useStore.getState().setEditorShowAdvancedFlowMetadata('flow-a', false)
        useStore.getState().updateEditorNodeInspectorSession('node-a', { showAdvanced: true })
        expect(completed.mock.calls.map(([event]) => (event as CustomEvent).detail)).toEqual([
            { expand_child_flows: false }, { graph_settings_open: true },
            { show_advanced_controls: false }, { show_advanced_controls: true },
        ])
        expect(useStore.getState().editorNodeInspectorSessionsByNodeId['node-a'].readsContextDraft).toBe('unsaved context')
    } finally { window.removeEventListener('spark:preferences-completed', completed) }
})

it('reuses presentation defaults in new run sessions while keeping record and cache state separate', () => {
    const completed = vi.fn()
    window.addEventListener('spark:preferences-completed', completed)
    try {
        useStore.getState().setClientRunPresentation({ graph_height: 640, sort: 'oldest' })
        useStore.getState().setRunsSelectedRunId('first-presentation-run')
        const first = useStore.getState().runDetailSessionsByRunId['first-presentation-run']
        expect(first.graphPaneHeight).toBe(640)
        useStore.getState().setClientRunPresentation({ graph_height: 720 })
        useStore.getState().setRunsSelectedRunId('second-presentation-run')
        expect(useStore.getState().runDetailSessionsByRunId['second-presentation-run'].graphPaneHeight).toBe(720)
        expect(useStore.getState().runDetailSessionsByRunId['first-presentation-run'].graphPaneHeight).toBe(640)
        const payload = JSON.stringify(completed.mock.calls.map(([event]) => event.detail))
        expect(payload).not.toContain('first-presentation-run')
        expect(payload).not.toContain('record')
    } finally { window.removeEventListener('spark:preferences-completed', completed) }
})
