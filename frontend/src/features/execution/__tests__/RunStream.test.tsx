import { RunStream } from '@/features/runs/RunStream'
import {
  flattenRunJournalSegments,
  useRunJournalStore,
} from '@/features/runs/state/runJournalStore'
import { useStore } from '@/store'
import { act, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const resetRunStreamState = () => {
  useRunJournalStore.setState({ byRunId: {} })
  useStore.setState((state) => ({
    ...state,
    selectedRunId: null,
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle',
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
    saveState: 'idle',
    saveStateVersion: 0,
    saveErrorMessage: null,
    saveErrorKind: null,
    humanGate: null,
    nodeStatuses: {},
    runtimeStatus: 'idle',
  }))
}

describe('RunStream save indicator', () => {
  beforeEach(() => {
    resetRunStreamState()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.runOnlyPendingTimers()
    vi.useRealTimers()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('shows a single saved toast and dismisses it after the fade window', () => {
    render(<RunStream />)

    act(() => {
      useStore.getState().markSaveSuccess()
    })

    expect(screen.getByTestId('global-save-state-indicator')).toHaveTextContent('Saved')

    act(() => {
      vi.advanceTimersByTime(1000)
    })

    expect(screen.getByTestId('global-save-state-indicator').className).toContain('opacity-0')

    act(() => {
      vi.advanceTimersByTime(1000)
    })

    expect(screen.queryByTestId('global-save-state-indicator')).not.toBeInTheDocument()
    expect(useStore.getState().saveState).toBe('idle')
  })

  it('updates the selected run to a terminal state from the streamed runtime event', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      if (url.endsWith('/attractor/pipelines/run-reconcile')) {
        return new Response(JSON.stringify({
          pipeline_id: 'run-reconcile',
          run_id: 'run-reconcile',
          status: 'running',
          outcome: null,
          outcome_reason_code: null,
          outcome_reason_message: null,
          flow_name: 'selected.dot',
          working_directory: '/tmp/project-one/workdir',
          project_path: '/tmp/project-one',
          git_branch: 'main',
          git_commit: 'abcdef0',
          spec_id: null,
          plan_id: null,
          model: 'gpt-5.4',
          started_at: '2026-03-22T00:00:00Z',
          ended_at: null,
          last_error: '',
          token_usage: 10,
          current_node: 'review',
          completed_nodes: ['start'],
          progress: {
            current_node: 'review',
            completed_nodes: ['start'],
          },
          continued_from_run_id: null,
          continued_from_node: null,
          continued_from_flow_mode: null,
          continued_from_flow_name: null,
        }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      throw new Error(`Unhandled request: ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    const eventSources: MockEventSource[] = []
    class MockEventSource {
      static readonly OPEN = 1
      static readonly CLOSED = 2
      readonly url: string
      readyState = MockEventSource.OPEN
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent<string>) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string | URL) {
        this.url = String(url)
        eventSources.push(this)
      }

      emit(payload: unknown) {
        this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(payload) }))
      }

      open() {
        this.onopen?.(new Event('open'))
      }

      close() {
        this.readyState = MockEventSource.CLOSED
      }
    }
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.getState().setSelectedRunId('run-reconcile')
      useStore.getState().setSelectedRunSnapshot({
        record: {
          run_id: 'run-reconcile',
          flow_name: 'selected.dot',
          status: 'running',
          outcome: null,
          outcome_reason_code: null,
          outcome_reason_message: null,
          working_directory: '/tmp/project-one/workdir',
          project_path: '/tmp/project-one',
          git_branch: 'main',
          git_commit: 'abcdef0',
          spec_id: null,
          plan_id: null,
          model: 'gpt-5.4',
          started_at: '2026-03-22T00:00:00Z',
          ended_at: null,
          last_error: '',
          token_usage: 10,
          continued_from_run_id: null,
          continued_from_node: null,
          continued_from_flow_mode: null,
          continued_from_flow_name: null,
        },
        completedNodes: ['start'],
      })
    })

    render(<RunStream />)

    expect(useStore.getState().runtimeStatus).toBe('idle')

    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(useStore.getState().runtimeStatus).toBe('running')
    expect(eventSources).toHaveLength(1)

    await act(async () => {
      eventSources[0]?.open()
      eventSources[0]?.emit({
        type: 'runtime',
        status: 'completed',
        outcome: 'success',
        outcome_reason_code: null,
        outcome_reason_message: null,
      })
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(useStore.getState().runtimeStatus).toBe('completed')
    expect(useStore.getState().selectedRunRecord?.status).toBe('completed')
    expect(useStore.getState().selectedRunRecord?.outcome).toBe('success')
  })

  it('keeps the selected run stream open from cached state when the status refresh is unavailable', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      if (url.endsWith('/attractor/pipelines/run-cached-fallback')) {
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }
      throw new Error(`Unhandled request: ${url}`)
    })
    vi.stubGlobal('fetch', fetchMock)

    const eventSources: MockEventSource[] = []
    class MockEventSource {
      static readonly OPEN = 1
      static readonly CLOSED = 2
      readonly url: string
      readyState = MockEventSource.OPEN
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent<string>) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string | URL) {
        this.url = String(url)
        eventSources.push(this)
      }

      emit(payload: unknown) {
        this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(payload) }))
      }

      close() {
        this.readyState = MockEventSource.CLOSED
      }
    }
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.getState().setSelectedRunId('run-cached-fallback')
      useStore.getState().setSelectedRunSnapshot({
        record: {
          run_id: 'run-cached-fallback',
          flow_name: 'cached.dot',
          status: 'running',
          outcome: null,
          outcome_reason_code: null,
          outcome_reason_message: null,
          working_directory: '/tmp/project-one/workdir',
          project_path: '/tmp/project-one',
          git_branch: 'main',
          git_commit: 'abcdef0',
          spec_id: null,
          plan_id: null,
          model: 'gpt-5.4',
          started_at: '2026-03-22T00:00:00Z',
          ended_at: null,
          last_error: '',
          token_usage: 10,
          continued_from_run_id: null,
          continued_from_node: null,
          continued_from_flow_mode: null,
          continued_from_flow_name: null,
        },
        completedNodes: ['start'],
      })
    })

    render(<RunStream />)

    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(eventSources).toHaveLength(1)
    expect(eventSources[0]?.url).toContain('/attractor/pipelines/run-cached-fallback/events')
    expect(eventSources[0]?.url).not.toContain('after_sequence=0')

    await act(async () => {
      eventSources[0]?.emit({
        type: 'StageStarted',
        sequence: 1,
        emitted_at: '2026-03-22T00:02:00Z',
        node_id: 'review',
        index: 2,
      })
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(useStore.getState().selectedRunRecord?.current_node).toBe('review')
    expect(
      flattenRunJournalSegments(useRunJournalStore.getState().byRunId['run-cached-fallback']?.segments ?? []),
    ).toHaveLength(1)
  })
})
