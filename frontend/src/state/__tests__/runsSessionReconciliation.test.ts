import { beforeEach, expect, it } from 'vitest'
import { useStore } from '@/store'
import type { RunRecord } from '@/features/runs/model/shared'
import { selectSelectedRunId, selectSelectedRunSession } from '../runsSessionSelectors'
import { buildRunsScopeKey } from '../runsSessionScope'

const record = (runId = 'a', patch: Partial<RunRecord> = {}): RunRecord => ({
    run_id: runId, flow_name: 'flow.yaml', status: 'running', working_directory: '/a',
    project_path: '/a', model: '', started_at: '', last_error: '', ...patch,
})

beforeEach(() => {
    useStore.setState(useStore.getInitialState(), true)
    useStore.getState().setRunsSelectedRunIdForScope('all', 'a')
    useStore.getState().updateRunsListSession({ scopeMode: 'all' })
})

it('seeds summaries before status, then preserves details while accepting available list telemetry', () => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record('a', { token_usage: 2 })] })
    expect(selectSelectedRunSession(useStore.getState())?.record?.token_usage).toBe(2)
    state.updateRunsListSession({ runs: [record('a', { current_node: 'summary' })] })
    expect(selectSelectedRunSession(useStore.getState())?.record?.current_node).toBe('summary')
    state.reconcileRunRecord('a', 'status', record('a', { status: 'completed', current_node: 'done', token_usage: 9 }), ['start', 'done'])
    const fetchedAt = selectSelectedRunSession(useStore.getState())?.statusFetchedAtMs
    state.updateRunsListSession({ runs: [record('a', { token_usage: null })] })
    expect(selectSelectedRunSession(useStore.getState())?.record).toMatchObject({ status: 'completed', current_node: 'done', token_usage: 9 })
    state.updateRunsListSession({ runs: [record('a', { token_usage: 10 })] })
    expect(selectSelectedRunSession(useStore.getState())).toMatchObject({ completedNodesSnapshot: ['start', 'done'], statusFetchedAtMs: fetchedAt, record: { token_usage: 10 } })
})

it('honors omitted versus null live fields and never manufactures records from partial events', () => {
    const state = useStore.getState()
    state.reconcileRunRecord('a', 'journal', { status: 'running' })
    expect(selectSelectedRunSession(useStore.getState())?.record).toBeNull()
    state.reconcileRunRecord('a', 'status', record('a', { outcome: 'failure', token_usage: 10, current_node: 'review' }))
    state.reconcileRunRecord('a', 'live', record('a', { status: 'completed', outcome: null, token_usage: undefined }))
    expect(selectSelectedRunSession(useStore.getState())?.record).toMatchObject({ status: 'completed', outcome: null, token_usage: 10, current_node: 'review' })
    state.reconcileRunRecord('a', 'live', record('a', { token_usage: null }))
    expect(selectSelectedRunSession(useStore.getState())?.record?.token_usage).toBeNull()
    state.reconcileRunRecord('a', 'journal', { current_node: 'done' })
    expect(selectSelectedRunSession(useStore.getState())?.record?.current_node).toBe('done')
})

it('rolls back only unchanged optimistic fields in both snapshots', () => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record('a', { status: 'failed', last_error: 'summary error' })] })
    state.reconcileRunRecord('a', 'status', record('a', { status: 'failed', last_error: 'detail error' }))
    const rollback = state.optimisticallyPatchRun('a', { status: 'running', last_error: '' })
    expect(useStore.getState().runsListSession.runs[0]).toMatchObject({ status: 'running', last_error: '' })
    expect(selectSelectedRunSession(useStore.getState())?.record).toMatchObject({ status: 'running', last_error: '' })
    state.reconcileRunRecord('a', 'journal', { status: 'completed', token_usage: 99 })
    rollback()
    expect(selectSelectedRunSession(useStore.getState())?.record).toMatchObject({ status: 'completed', last_error: 'detail error', token_usage: 99 })
    expect(useStore.getState().runsListSession.runs[0]).toMatchObject({ status: 'failed', last_error: 'summary error' })
})

it('optimistically cancels an uninspected list row without creating a session', () => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record('b')] })
    const rollback = state.optimisticallyPatchRun('b', { status: 'cancel_requested' })
    expect(useStore.getState().runsListSession.runs[0].status).toBe('cancel_requested')
    expect(useStore.getState().runDetailSessionsByRunId.b).toBeUndefined()
    rollback()
    expect(useStore.getState().runsListSession.runs[0].status).toBe('running')
})

it('clears references to a missing run without deselecting a newer run or accepting late writes', () => {
    const state = useStore.getState()
    state.setRunsSelectedRunIdForScope('project:/a', 'a')
    state.setRunsSelectedRunIdForScope('all', 'b')
    state.clearRunDetailSession('a')
    state.updateRunDetailSession('a', { statusError: 'late failure' })
    state.reconcileRunRecord('a', 'status', record())
    expect(useStore.getState().runDetailSessionsByRunId.a).toBeUndefined()
    expect(useStore.getState().runsListSession.selectedRunIdByScopeKey['project:/a']).toBeNull()
    expect(selectSelectedRunId(useStore.getState())).toBe('b')
})

it('prunes removed project sessions, including a selected run whose status has not arrived', () => {
    const state = useStore.getState()
    state.registerProject('/a')
    state.registerProject('/b')
    state.setRunsSelectedRunIdForScope('project:/a', 'unknown-a')
    state.setRunsSelectedRunIdForScope('project:/b', 'b')
    state.reconcileRunRecord('a', 'status', record())
    const rollback = state.optimisticallyPatchRun('a', { status: 'cancel_requested' })
    state.removeProject('/a', '/b')
    rollback()
    state.updateRunDetailSession('unknown-a', { graphError: 'late graph' })
    expect(Object.keys(useStore.getState().runDetailSessionsByRunId)).toEqual(['b'])
})

it('invalidates question confirmation on project and all-project transitions while retaining cached content', () => {
    const state = useStore.getState()
    state.registerProject('/a')
    state.registerProject('/b')
    state.setRunsSelectedRunIdForScope(buildRunsScopeKey('active', '/a'), 'a')
    state.setRunsSelectedRunIdForScope(buildRunsScopeKey('active', '/b'), 'b')
    state.updateRunDetailSession('a', { record: record(), questionsStatus: 'ready', resourceRequestIds: { questions: 1 } })
    state.updateRunsListSession({ scopeMode: 'active' })
    state.setActiveProjectPath('/b')
    state.setActiveProjectPath('/a')
    expect(selectSelectedRunSession(useStore.getState())).toMatchObject({ questionsStatus: 'idle', resourceRequestIds: { questions: -1 }, record: { run_id: 'a' } })
    state.updateRunDetailSession('b', { questionsStatus: 'ready' })
    state.setRunsSelectedRunIdForScope('all', 'b')
    state.updateRunsListSession({ scopeMode: 'all' })
    expect(selectSelectedRunSession(useStore.getState())?.questionsStatus).toBe('idle')
})

it('rolls back an optimistic list change when that run is first inspected while the action is pending', () => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record('b')] })
    const rollback = state.optimisticallyPatchRun('b', { status: 'cancel_requested' })
    state.setRunsSelectedRunIdForScope('all', 'b')
    expect(selectSelectedRunSession(useStore.getState())?.record?.status).toBe('cancel_requested')
    rollback()
    expect(useStore.getState().runsListSession.runs[0].status).toBe('running')
    expect(selectSelectedRunSession(useStore.getState())?.record?.status).toBe('running')
})


it.each(['live', 'journal', 'optimistic'] as const)('preserves newer %s fields through list summaries and status hydration', (source) => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record()] })
    const requestUpdates = useStore.getState().runDetailSessionsByRunId.a.recordUpdates
    const patch = { status: 'cancel_requested', last_error: 'newer' }
    if (source === 'optimistic') state.optimisticallyPatchRun('a', patch)
    else state.reconcileRunRecord('a', source, patch)
    state.updateRunsListSession({ runs: [record('a', { flow_name: 'stale summary' })] })
    state.reconcileRunRecord('a', 'status', record('a', { git_commit: 'durable' }), ['done'], requestUpdates)
    expect(selectSelectedRunSession(useStore.getState())).toMatchObject({
        completedNodesSnapshot: ['done'], record: { ...patch, flow_name: 'flow.yaml', git_commit: 'durable' },
    })
})

it('preserves rollback during status hydration but does not retain updates from before the request', () => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record()] })
    const rollback = state.optimisticallyPatchRun('a', { status: 'cancel_requested' })
    const requestUpdates = useStore.getState().runDetailSessionsByRunId.a.recordUpdates
    rollback()
    state.reconcileRunRecord('a', 'status', record('a', { status: 'cancel_requested' }), [], requestUpdates)
    expect(selectSelectedRunSession(useStore.getState())?.record?.status).toBe('running')
    const nextRequest = useStore.getState().runDetailSessionsByRunId.a.recordUpdates
    state.reconcileRunRecord('a', 'status', record('a', { status: 'completed' }), [], nextRequest)
    expect(selectSelectedRunSession(useStore.getState())?.record?.status).toBe('completed')
})

it.each(['live', 'journal'] as const)('tracks the latest %s write even when it matches the intervening summary', (source) => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record()] })
    const requestUpdates = useStore.getState().runDetailSessionsByRunId.a.recordUpdates
    state.reconcileRunRecord('a', source, { status: 'completed' })
    state.updateRunsListSession({ runs: [record()] })
    state.reconcileRunRecord('a', source, { status: 'running' })
    state.reconcileRunRecord('a', 'status', record(), [], requestUpdates)
    expect(selectSelectedRunSession(useStore.getState())?.record?.status).toBe('running')
})

it('gives only applied optimistic and rollback fields authority during hydration', () => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record('a', { flow_name: 'stale summary' })] })
    const requestUpdates = useStore.getState().runDetailSessionsByRunId.a.recordUpdates
    const rollback = state.optimisticallyPatchRun('a', { status: 'cancel_requested', last_error: '' })
    state.reconcileRunRecord('a', 'journal', { status: 'completed' })
    const completedUpdate = useStore.getState().runDetailSessionsByRunId.a.recordUpdates.status
    rollback()
    expect(useStore.getState().runDetailSessionsByRunId.a.recordUpdates.status).toBe(completedUpdate)
    state.reconcileRunRecord('a', 'status', record('a', { flow_name: 'durable', last_error: 'old error' }), [], requestUpdates)
    expect(selectSelectedRunSession(useStore.getState())?.record).toMatchObject({ status: 'completed', flow_name: 'durable', last_error: '' })
})

it('refreshes an unselected cached run from list status without losing detailed data', () => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record()] })
    state.reconcileRunRecord('a', 'status', record('a', { execution_profile_id: 'native' }), ['start'])
    state.setRunsSelectedRunIdForScope('all', 'b')
    state.updateRunsListSession({ runs: [record('a', { status: 'completed', outcome: 'success', current_node: 'done' })] })
    const session = useStore.getState().runDetailSessionsByRunId.a
    expect(session.record).toMatchObject({ status: 'completed', outcome: 'success', current_node: 'done', execution_profile_id: 'native' })
    expect(session.completedNodesSnapshot).toEqual(['start'])
})

it.each(['live', 'journal', 'status'] as const)('does not roll back a newer %s confirmation of the optimistic value', (source) => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record('a', { status: 'failed' })] })
    state.reconcileRunRecord('a', 'status', record('a', { status: 'failed' }))
    const rollback = state.optimisticallyPatchRun('a', { status: 'running' })
    state.reconcileRunRecord('a', source, source === 'journal' ? { status: 'running' } : record('a'))
    rollback()
    expect(useStore.getState().runDetailSessionsByRunId.a.record?.status).toBe('running')
})

it('preserves a newer list confirmation when rolling back an uninspected run', () => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record('b', { status: 'failed' })] })
    const rollback = state.optimisticallyPatchRun('b', { status: 'running' })
    state.updateRunsListSession({ runs: [record('b')] })
    rollback()
    expect(useStore.getState().runsListSession.runs[0].status).toBe('running')
})

it('preserves a fresh list confirmation for a cached run after switching away', () => {
    const state = useStore.getState()
    state.updateRunsListSession({ runs: [record('a', { status: 'failed' })] })
    state.reconcileRunRecord('a', 'status', record('a', { status: 'failed' }))
    const rollback = state.optimisticallyPatchRun('a', { status: 'running' })
    state.setRunsSelectedRunIdForScope('all', 'b')
    state.updateRunsListSession({ runs: [record('a')] })
    rollback()
    expect(useStore.getState().runDetailSessionsByRunId.a.record?.status).toBe('running')
    expect(useStore.getState().runsListSession.runs[0].status).toBe('running')
})
