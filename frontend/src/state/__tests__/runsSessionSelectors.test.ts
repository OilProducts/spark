import { beforeEach, expect, it } from 'vitest'
import { useStore } from '@/store'
import type { RunRecord } from '@/features/runs/model/shared'
import { selectSelectedRunId, selectSelectedRunSession } from '../runsSessionSelectors'

beforeEach(() => useStore.setState(useStore.getInitialState(), true))

it('keeps one run selection, whichever chat project is open', () => {
    const state = useStore.getState()
    state.setRunsSelectedRunId('a')
    state.updateRunDetailSession('a', { selectedNodeId: 'node-a' })
    const cached = selectSelectedRunSession(useStore.getState())
    useStore.setState({ activeProjectPath: '/b' })
    expect(selectSelectedRunId(useStore.getState())).toBe('a')
    expect(selectSelectedRunSession(useStore.getState())).toBe(cached)
    state.setRunsSelectedRunId(null)
    expect(selectSelectedRunId(useStore.getState())).toBeNull()
    expect(selectSelectedRunSession(useStore.getState())).toBeNull()
})

it('clears a search that would hide a run being opened, even the one already open', () => {
    const run = (run_id: string, title: string, parent_run_id: string | null = null) => ({ run_id, title, parent_run_id, flow_name: 'flow.yaml', status: 'completed', started_at: '', working_directory: '', model: '' }) as RunRecord
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [run('launch', 'November launch'), run('hotfix', 'Hotfix'), run('parent', 'Release train'), run('child', 'Step', 'parent')] })
    state.setRunsSelectedRunId('hotfix')
    state.updateRunsListSession({ searchQuery: 'November' })
    state.setRunsSelectedRunId('launch')
    expect(useStore.getState().runsListSession.searchQuery).toBe('November')
    // The child of a matching parent stays listed under it.
    state.updateRunsListSession({ searchQuery: 'release' })
    state.setRunsSelectedRunId('child')
    expect(useStore.getState().runsListSession.searchQuery).toBe('release')
    // Reopening the open run (from the Inbox, say) still clears a search that hides it.
    state.setRunsSelectedRunId('hotfix')
    state.updateRunsListSession({ searchQuery: 'November' })
    state.setRunsSelectedRunId('hotfix')
    expect(useStore.getState().runsListSession.searchQuery).toBe('')
})
