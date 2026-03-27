import { RunsPanel } from '@/features/runs/RunsPanel'
import { useStore } from '@/store'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const jsonResponse = (payload: unknown) =>
  new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  })

const resolveRequestUrl = (input: RequestInfo | URL): string => {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.toString()
  return input.url
}

const resetRunsState = () => {
  useStore.setState({
    viewMode: 'runs',
    activeProjectPath: null,
    selectedRunId: null,
    executionFlow: null,
    projectRegistry: {},
    projectSessionsByPath: {},
    recentProjectPaths: [],
  })
}

const makeRun = (overrides: Partial<Record<string, unknown>> = {}) => ({
  run_id: String(overrides.run_id ?? 'run-1'),
  flow_name: String(overrides.flow_name ?? 'review.dot'),
  status: String(overrides.status ?? 'completed'),
  outcome: (overrides.outcome as 'success' | 'failure' | null | undefined) ?? 'success',
  outcome_reason_code: null,
  outcome_reason_message: null,
  working_directory: String(overrides.working_directory ?? '/tmp/workdir'),
  project_path: String(overrides.project_path ?? '/tmp/project-one'),
  git_branch: 'main',
  git_commit: 'abcdef0',
  spec_id: null,
  plan_id: null,
  model: 'gpt-5.3-codex-spark',
  started_at: '2026-03-22T00:00:00Z',
  ended_at: '2026-03-22T00:05:00Z',
  last_error: null,
  token_usage: 1234,
})

describe('RunsPanel', () => {
  beforeEach(() => {
    resetRunsState()
    vi.stubGlobal('fetch', vi.fn())
    class MockEventSource {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 2
      readonly url: string
      readyState = MockEventSource.OPEN
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent<string>) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string | URL) {
        this.url = String(url)
      }

      close() {
        this.readyState = MockEventSource.CLOSED
      }
    }
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('defaults to the active project scope and can switch to all projects', async () => {
    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({
          runs: [
            makeRun({
              run_id: 'run-project-one',
              flow_name: 'project-one.dot',
              project_path: '/tmp/project-one',
            }),
          ],
        })
      }
      if (url.endsWith('/attractor/runs')) {
        return jsonResponse({
          runs: [
            makeRun({
              run_id: 'run-project-one',
              flow_name: 'project-one.dot',
              project_path: '/tmp/project-one',
            }),
            makeRun({
              run_id: 'run-project-two',
              flow_name: 'project-two.dot',
              project_path: '/tmp/project-two',
            }),
          ],
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
    })

    const user = userEvent.setup()
    render(<RunsPanel />)

    await waitFor(() => {
      expect(screen.getByText('project-one.dot')).toBeVisible()
    })
    expect(
      fetchMock.mock.calls.some(([request]) =>
        resolveRequestUrl(request as RequestInfo | URL).includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one'),
      ),
    ).toBe(true)
    expect(screen.getByText('Run history for the active project.')).toBeVisible()

    await user.click(screen.getByTestId('runs-scope-all-projects'))

    await waitFor(() => {
      expect(screen.getByText('project-two.dot')).toBeVisible()
    })
    expect(
      fetchMock.mock.calls.some(([request]) => {
        const url = resolveRequestUrl(request as RequestInfo | URL)
        return url.endsWith('/attractor/runs') && !url.includes('project_path=')
      }),
    ).toBe(true)
    expect(screen.getByText('Run history across all projects.')).toBeVisible()
  })

  it('shows an explicit no-project notice before fetching all-project runs', async () => {
    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (url.endsWith('/attractor/runs') && method === 'GET') {
        return jsonResponse({
          runs: [
            makeRun({
              run_id: 'run-global',
              flow_name: 'global.dot',
              project_path: '/tmp/project-two',
            }),
          ],
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    const user = userEvent.setup()
    render(<RunsPanel />)

    expect(screen.getByText('Choose an active project or switch to all projects to view run history.')).toBeVisible()
    expect(fetchMock).not.toHaveBeenCalled()

    await user.click(screen.getByTestId('runs-scope-all-projects'))

    await waitFor(() => {
      expect(screen.getByText('global.dot')).toBeVisible()
    })
    expect(fetchMock).toHaveBeenCalled()
  })

  it('keeps the run selector ahead of the detail stack and constrains it to a scroll region', async () => {
    window.innerWidth = 1400
    window.dispatchEvent(new Event('resize'))

    const selectedRun = makeRun({
      run_id: 'run-selected',
      flow_name: 'selected.dot',
      status: 'running',
      ended_at: null,
      project_path: '/tmp/project-one',
      spec_id: 'spec-123',
      plan_id: 'plan-123',
    })
    const secondaryRun = makeRun({
      run_id: 'run-secondary',
      flow_name: 'secondary.dot',
      project_path: '/tmp/project-one',
    })

    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({
          runs: [selectedRun, secondaryRun],
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-selected',
          checkpoint: {
            completed_nodes: ['prepare'],
            current_node: 'validate',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/context')) {
        return jsonResponse({
          pipeline_id: 'run-selected',
          context: {
            active_item: 'REQ-001',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-selected',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/graph')) {
        return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
          status: 200,
          headers: { 'Content-Type': 'image/svg+xml' },
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/questions')) {
        return jsonResponse({
          pipeline_id: 'run-selected',
          questions: [],
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
    })

    const user = userEvent.setup()
    render(<RunsPanel />)

    await waitFor(() => {
      expect(screen.getByTestId('run-list-panel')).toBeVisible()
      expect(screen.getAllByTestId('run-history-row')).toHaveLength(2)
    })

    expect(screen.getByTestId('runs-panel')).toHaveAttribute('data-responsive-layout', 'split')

    const runListPanel = screen.getByTestId('run-list-panel')
    expect(runListPanel).toHaveClass('border-r')

    const scrollRegion = screen.getByTestId('run-list-scroll-region')
    expect(scrollRegion).toHaveClass('flex-1')
    expect(scrollRegion).toHaveClass('overflow-y-auto')

    expect(screen.getAllByRole('button', { name: 'Open' })).toHaveLength(2)
    expect(screen.getAllByRole('button', { name: 'Cancel' }).some((button) => !button.hasAttribute('disabled'))).toBe(true)
    expect(screen.queryByText('history table')).not.toBeInTheDocument()

    await user.click(screen.getAllByTestId('run-history-row')[0]!)

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-panel')).toBeVisible()
    })

    const runSummaryPanel = screen.getByTestId('run-summary-panel')
    expect(
      runListPanel.compareDocumentPosition(runSummaryPanel) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
  })

  it('selects a run in place when clicking the card without leaving the runs tab', async () => {
    const primaryRun = makeRun({
      run_id: 'run-primary',
      flow_name: 'primary.dot',
      project_path: '/tmp/project-one',
    })
    const selectedRun = makeRun({
      run_id: 'run-selected',
      flow_name: 'selected.dot',
      status: 'running',
      ended_at: null,
      project_path: '/tmp/project-one',
    })

    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({
          runs: [primaryRun, selectedRun],
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-selected',
          checkpoint: {
            completed_nodes: ['prepare'],
            current_node: 'validate',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/context')) {
        return jsonResponse({
          pipeline_id: 'run-selected',
          context: {
            active_item: 'REQ-001',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-selected',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/graph')) {
        return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
          status: 200,
          headers: { 'Content-Type': 'image/svg+xml' },
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/questions')) {
        return jsonResponse({
          pipeline_id: 'run-selected',
          questions: [],
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
    })

    const user = userEvent.setup()
    render(<RunsPanel />)

    await waitFor(() => {
      expect(screen.getByText('selected.dot')).toBeVisible()
    })

    const runCards = screen.getAllByTestId('run-history-row')
    await user.click(runCards[1]!)

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-panel')).toBeVisible()
      expect(screen.getByTestId('run-summary-flow-name')).toHaveTextContent('selected.dot')
    })

    expect(useStore.getState().viewMode).toBe('runs')
    expect(useStore.getState().selectedRunId).toBe('run-selected')
  })
})
