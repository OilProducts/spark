import { beforeEach, expect, it } from 'vitest'
import { useStore } from '@/store'
import { buildRunsScopeKey } from '../runsSessionScope'
import { selectSelectedRunId, selectSelectedRunSession } from '../runsSessionSelectors'

beforeEach(() => useStore.setState(useStore.getInitialState(), true))

it('remembers independent project and all-project selections, including explicit empty selections', () => {
    const state = useStore.getState()
    state.setRunsSelectedRunIdForScope(buildRunsScopeKey('active', '/a'), 'a')
    state.setRunsSelectedRunIdForScope(buildRunsScopeKey('active', '/b'), 'b')
    state.setRunsSelectedRunIdForScope('all', null)
    state.updateRunDetailSession('a', { selectedNodeId: 'node-a', inspectorTab: 'artifacts' })
    state.updateRunDetailSession('b', { selectedNodeId: 'node-b' })
    useStore.setState({ activeProjectPath: '/a' })
    const cached = selectSelectedRunSession(useStore.getState())
    expect(selectSelectedRunId(useStore.getState())).toBe('a')
    useStore.setState({ activeProjectPath: '/b' })
    expect(selectSelectedRunSession(useStore.getState())?.selectedNodeId).toBe('node-b')
    useStore.setState({ activeProjectPath: '/a' })
    expect(selectSelectedRunSession(useStore.getState())).toBe(cached)
    state.updateRunsListSession({ scopeMode: 'all' })
    expect(selectSelectedRunId(useStore.getState())).toBeNull()
    expect(selectSelectedRunSession(useStore.getState())).toBeNull()
    state.updateRunsListSession({ scopeMode: 'active' })
    expect(selectSelectedRunSession(useStore.getState())).toBe(cached)
})
