import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useRunsList } from '../hooks/useRunsList'
import { useRunDetailResources } from '../hooks/useRunDetailResources'
import type { RunRecord } from '../model/shared'
import { runTitle } from '../model/runOverviewModel'
import { requestRunsTransportReconnect } from '../services/runsTransportReconnect'
import { useStore } from '@/store'
import { selectSelectedRunId } from '@/state/runsSessionSelectors'

const pending: { url: string; signal?: AbortSignal | null; resolve: (value: Response) => void; reject: (error: Error) => void }[] = []
const notice = (projectPath = '/one') => act(() => {
  window.dispatchEvent(new CustomEvent('spark:runs-overview-resync-required', { detail: { projectPath } }))
})
const complete = async (index: number, runId = 'run') => {
  await act(async () => pending[index].resolve(new Response(JSON.stringify({ runs: [
    { run_id: runId, project_path: new URL(pending[index].url, 'http://localhost').searchParams.get('project_path'), status: 'running' },
  ] }), { status: 200, headers: { 'Content-Type': 'application/json' } })))
}
const mount = () => renderHook(({ enabled }) => useRunsList({
  selectedRunId: null, manageSync: enabled,
}), { initialProps: { project: '/one', enabled: true } })

beforeEach(() => {
  pending.length = 0
  useStore.setState(useStore.getInitialState(), true)
  useStore.setState({ viewMode: 'runs', runsListSession: {
    selectedRunId: null, projectFilter: null, runs: [], status: 'idle', error: null,
    streamStatus: 'idle', streamError: null,
  } })
  vi.stubGlobal('fetch', vi.fn((url: string, init?: RequestInit) => new Promise<Response>((resolve, reject) => {
    pending.push({ url, signal: init?.signal, resolve, reject })
  })))
})
afterEach(() => vi.unstubAllGlobals())

it('coalesces initial, manual, reconnect and recovery bursts into one trailing request', async () => {
  const { result } = mount()
  expect(pending).toHaveLength(1)
  for (let i = 0; i < 20; i++) notice(i % 2 ? '/one' : '/other')
  act(() => { void result.current.fetchRuns(); requestRunsTransportReconnect() })
  expect(pending).toHaveLength(1)
  expect(pending[0].signal?.aborted).toBe(false)
  await complete(0)
  expect(pending).toHaveLength(2)
  for (let i = 0; i < 20; i++) notice()
  await complete(1)
  expect(pending).toHaveLength(3)
  await complete(2)
  expect(pending).toHaveLength(3)
  expect(result.current.status).toBe('ready')
})

it('clears pending refresh on failure and retries only when requested', async () => {
  const { result } = mount()
  notice()
  await act(async () => pending[0].resolve(new Response('', { status: 503 })))
  expect(pending).toHaveLength(1)
  expect(result.current.error).toBe('Unable to load runs')
  expect(result.current.streamStatus).toBe('degraded')
  notice()
  expect(pending).toHaveLength(2)
  await act(async () => pending[1].resolve(new Response('', { status: 503 })))
  expect(pending).toHaveLength(2)
  act(() => requestRunsTransportReconnect())
  expect(pending).toHaveLength(3)
  await complete(2)
  expect(result.current.error).toBeNull()
})

it('lists every project\'s runs', async () => {
  const { result } = mount()
  expect(new URL(pending[0].url, 'http://localhost').searchParams.has('project_path')).toBe(false)
  await complete(0, 'any-project')
  expect(result.current.scopedRuns[0].run_id).toBe('any-project')
})

it('aborts and discards pending work when synchronization stops without cancellation errors', async () => {
  const { result, rerender } = mount()
  notice()
  rerender({ project: '/one', enabled: false })
  expect(pending[0].signal?.aborted).toBe(true)
  await act(async () => pending[0].reject(new DOMException('Aborted', 'AbortError')))
  notice()
  expect(pending).toHaveLength(1)
  expect(result.current.error).toBeNull()
  rerender({ project: '/two', enabled: true })
  await complete(1, 'resumed')
  expect(result.current.scopedRuns[0].run_id).toBe('resumed')
})

it('reconciles only the live upsert row and still accepts fresh list telemetry', async () => {
  const record = (runId: string, telemetry: Partial<RunRecord> = {}): RunRecord => ({
    run_id: runId, project_path: '/one', working_directory: '/one', flow_name: 'flow.yaml',
    status: 'running', model: '', started_at: '', last_error: '', ...telemetry,
  })
  useStore.setState({ activeProjectPath: '/one' })
  const state = useStore.getState()
  state.setRunsSelectedRunId('b')
  state.setRunsSelectedRunId('a')
  const list = renderHook(() => useRunsList({ selectedRunId: 'a' }))
  const respond = async (index: number, runs: RunRecord[]) => {
    await act(async () => pending[index].resolve(new Response(JSON.stringify({ runs }), {
      status: 200, headers: { 'Content-Type': 'application/json' },
    })))
  }
  await respond(0, [record('a', { token_usage: 5 }), record('b', { token_usage: 2 })])
  act(() => state.reconcileRunRecord('a', 'status', record('a', { token_usage: 100 })))
  let renders = 0
  renderHook(() => {
    renders++
    return useRunDetailResources({ selectedRunId: 'a', manageSync: false })
  })
  expect(selectSelectedRunId(useStore.getState())).toBe('a')
  const before = useStore.getState().runDetailSessionsByRunId.a
  const beforeRenders = renders
  const upsert = (run: RunRecord) => act(() => {
    window.dispatchEvent(new CustomEvent('spark:run-upsert', { detail: { run } }))
  })
  upsert(record('b', { token_usage: 20 }))
  expect(useStore.getState().runDetailSessionsByRunId.b.record?.token_usage).toBe(20)
  expect(useStore.getState().runsListSession.runs.find((run) => run.run_id === 'b')?.token_usage).toBe(20)
  expect(useStore.getState().runDetailSessionsByRunId.a).toBe(before)
  expect(useStore.getState().runDetailSessionsByRunId.a.record).toBe(before.record)
  expect(before.record?.token_usage).toBe(100)
  expect(renders).toBe(beforeRenders)
  expect(list.result.current.scopedRuns.find((run) => run.run_id === 'a')?.token_usage).toBe(100)

  upsert(record('b'))
  expect(useStore.getState().runDetailSessionsByRunId.b.record?.token_usage).toBe(20)
  upsert(record('b', { token_usage: null }))
  expect(useStore.getState().runDetailSessionsByRunId.b.record?.token_usage).toBeNull()
  expect(renders).toBe(beforeRenders)

  for (const telemetry of [{ token_usage: 101 }, {}, { token_usage: null }]) {
    notice()
    await respond(pending.length - 1, [record('a', telemetry), record('b')])
    expect(useStore.getState().runDetailSessionsByRunId.a.record?.token_usage).toBe(101)
  }
})

it('keeps the launch-input title fallback through initial load, live updates and new live runs', async () => {
  const record = (runId: string, input: string, extra: Partial<RunRecord> = {}) => ({
    run_id: runId, project_path: '/one', working_directory: '/one', flow_name: 'implement-change.yaml',
    status: 'running', model: '', started_at: runId, last_error: '', first_launch_input: input, ...extra,
  })
  useStore.setState({ activeProjectPath: '/one' })
  renderHook(() => useRunsList({ selectedRunId: null }))
  await act(async () => pending[0].resolve(new Response(JSON.stringify({ runs: [record('a', 'changes/CR-1/request.md')] }), {
    status: 200, headers: { 'Content-Type': 'application/json' },
  })))
  const titles = () => useStore.getState().runsListSession.runs.map(runTitle)
  expect(titles()).toEqual(['changes/CR-1/request.md'])

  const upsert = (run: object) => act(() => {
    window.dispatchEvent(new CustomEvent('spark:run-upsert', { detail: { run } }))
  })
  upsert(record('a', 'changes/CR-1/request.md', { status: 'completed' }))
  expect(titles()).toEqual(['changes/CR-1/request.md'])
  upsert(record('b', 'changes/CR-2/request.md'))
  expect(titles()).toEqual(['changes/CR-2/request.md', 'changes/CR-1/request.md'])
})
