import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useRunsList } from '../hooks/useRunsList'
import { requestRunsTransportReconnect } from '../services/runsTransportReconnect'
import { useStore } from '@/store'

const pending: { url: string; signal?: AbortSignal | null; resolve: (value: Response) => void; reject: (error: Error) => void }[] = []
const notice = (projectPath = '/one') => act(() => {
  window.dispatchEvent(new CustomEvent('spark:runs-overview-resync-required', { detail: { projectPath } }))
})
const complete = async (index: number, runId = 'run') => {
  await act(async () => pending[index].resolve(new Response(JSON.stringify({ runs: [
    { run_id: runId, project_path: new URL(pending[index].url, 'http://localhost').searchParams.get('project_path'), status: 'running' },
  ] }), { status: 200, headers: { 'Content-Type': 'application/json' } })))
}
const mount = () => renderHook(({ project, enabled }) => useRunsList({
  activeProjectPath: project, scopeMode: 'active', selectedRunId: null, manageSync: enabled,
}), { initialProps: { project: '/one', enabled: true } })

beforeEach(() => {
  pending.length = 0
  useStore.setState({ viewMode: 'runs', runsListSession: {
    scopeMode: 'active', selectedRunIdByScopeKey: {}, runs: [], status: 'idle', error: null,
    streamStatus: 'idle', streamError: null,
  } })
  vi.stubGlobal('fetch', vi.fn((url: string, init?: RequestInit) => new Promise<Response>((resolve, reject) => {
    pending.push({ url, signal: init?.signal, resolve, reject })
  })))
})
afterEach(() => vi.unstubAllGlobals())

it('coalesces initial, manual, reconnect and recovery bursts into one trailing request', async () => {
  const { result } = mount()
  notice('/other')
  expect(pending).toHaveLength(1)
  for (let i = 0; i < 20; i++) notice()
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
  notice('/other')
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

it('aborts on scope change, rejects stale responses and cleanup, and loads the new scope', async () => {
  const { result, rerender } = mount()
  await complete(0)
  notice()
  notice()
  rerender({ project: '/two', enabled: true })
  expect(pending[1].signal?.aborted).toBe(true)
  expect(pending).toHaveLength(3)
  expect(new URL(pending[2].url, 'http://localhost').searchParams.get('project_path')).toBe('/two')
  await complete(1, 'stale')
  expect(result.current.scopedRuns[0].run_id).not.toBe('stale')
  expect(result.current.status).toBe('loading')
  notice('/one')
  expect(pending).toHaveLength(3)
  notice('/two')
  await complete(2, 'new-scope')
  expect(result.current.scopedRuns[0].run_id).toBe('new-scope')
  expect(result.current.scopedRuns[0].project_path).toBe('/two')
  expect(pending).toHaveLength(4)
  await complete(3, 'new-scope')
  expect(pending).toHaveLength(4)
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
