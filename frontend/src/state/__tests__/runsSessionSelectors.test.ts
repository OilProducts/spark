import { beforeEach, expect, it } from 'vitest'
import { useStore } from '@/store'
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
