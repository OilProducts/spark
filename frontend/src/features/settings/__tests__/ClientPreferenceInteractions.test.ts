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
        useStore.getState().setClientRunPresentation({ activity_mode: 'events', inspector_tab: 'context', timeline_severity: 'error', graph_height: 640, sort: 'oldest' })
        useStore.getState().setRunsSelectedRunIdForScope('all', 'first-presentation-run')
        const first = useStore.getState().runDetailSessionsByRunId['first-presentation-run']
        expect(first.activityMode).toBe('events')
        expect(first.inspectorTab).toBe('context')
        expect(first.timelineSeverityFilter).toBe('error')
        expect(first.graphPaneHeight).toBe(640)
        useStore.getState().setClientRunPresentation({ activity_mode: 'transcript' })
        useStore.getState().setRunsSelectedRunIdForScope('all', 'second-presentation-run')
        expect(useStore.getState().runDetailSessionsByRunId['second-presentation-run'].activityMode).toBe('transcript')
        expect(useStore.getState().runDetailSessionsByRunId['first-presentation-run'].activityMode).toBe('events')
        const payload = JSON.stringify(completed.mock.calls.map(([event]) => event.detail))
        expect(payload).not.toContain('first-presentation-run')
        expect(payload).not.toContain('record')
    } finally { window.removeEventListener('spark:preferences-completed', completed) }
})
