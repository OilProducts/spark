import { RunsPanel } from '@/features/runs/RunsPanel'
import { useStore } from '@/store'
import { DialogProvider } from '@/ui'
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

const renderRunsPanel = () =>
  render(
    <DialogProvider>
      <RunsPanel />
    </DialogProvider>,
  )

const makeRun = (overrides: Partial<Record<string, unknown>> = {}) => ({
  run_id: String(overrides.run_id ?? 'run-1'),
  flow_name: String(overrides.flow_name ?? 'review.dot'),
  status: String(overrides.status ?? 'completed'),
  outcome: (overrides.outcome as 'success' | 'failure' | null | undefined) ?? 'success',
  outcome_reason_code: null,
  outcome_reason_message: null,
  working_directory: String(overrides.working_directory ?? '/tmp/workdir'),
  project_path: String(overrides.project_path ?? '/tmp/project-one'),
  git_branch: (overrides.git_branch as string | null | undefined) ?? 'main',
  git_commit: (overrides.git_commit as string | null | undefined) ?? 'abcdef0',
  spec_id: (overrides.spec_id as string | null | undefined) ?? null,
  plan_id: (overrides.plan_id as string | null | undefined) ?? null,
  model: String(overrides.model ?? 'gpt-5.3-codex-spark'),
  started_at: String(overrides.started_at ?? '2026-03-22T00:00:00Z'),
  ended_at: (overrides.ended_at as string | null | undefined) ?? '2026-03-22T00:05:00Z',
  last_error: (overrides.last_error as string | null | undefined) ?? null,
  token_usage: (overrides.token_usage as number | null | undefined) ?? 1234,
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
    renderRunsPanel()

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
    renderRunsPanel()

    expect(screen.getByText('Choose an active project or switch to all projects to view run history.')).toBeVisible()
    expect(fetchMock).not.toHaveBeenCalled()

    await user.click(screen.getByTestId('runs-scope-all-projects'))

    await waitFor(() => {
      expect(screen.getByText('global.dot')).toBeVisible()
    })
    expect(fetchMock).toHaveBeenCalled()
  })

  it('keeps the run selector ahead of the detail stack, removes row actions, and shows graph plus console in the detail pane', async () => {
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
      if (url.includes('/attractor/pipelines/run-selected/graph-preview')) {
        return jsonResponse({
          status: 'ok',
          graph: {
            graph_attrs: {
              label: 'Selected graph',
            },
            nodes: [
              { id: 'start', label: 'Start', shape: 'Mdiamond' },
              { id: 'validate', label: 'Validate', shape: 'box' },
              { id: 'done', label: 'Done', shape: 'Msquare' },
            ],
            edges: [
              { from: 'start', to: 'validate', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              { from: 'validate', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
            ],
          },
          diagnostics: [],
          errors: [],
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
    renderRunsPanel()

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

    expect(screen.queryByRole('button', { name: 'Open' })).not.toBeInTheDocument()
    expect(screen.queryAllByRole('button', { name: 'Cancel' })).toHaveLength(0)
    expect(screen.queryByText('history table')).not.toBeInTheDocument()

    const selectedRunCard = screen.getByText('selected.dot').closest('[data-testid="run-history-row"]')
    expect(selectedRunCard).not.toBeNull()
    await user.click(selectedRunCard!)

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-panel')).toBeVisible()
    })

    const runSummaryPanel = screen.getByTestId('run-summary-panel')
    expect(screen.getByTestId('run-summary-spec-artifact-link')).toBeVisible()
    expect(screen.getByTestId('run-summary-plan-artifact-link')).toBeVisible()
    expect(screen.getByTestId('run-summary-cancel-button')).toBeEnabled()
    expect(screen.getByTestId('run-graph-panel')).toBeVisible()
    expect(screen.getByTestId('run-console-panel')).toBeVisible()
    expect(screen.getByTestId('run-summary-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-graph-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-console-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-checkpoint-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-context-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-artifact-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-event-timeline-toggle-button')).toBeVisible()

    await user.click(screen.getByTestId('run-graph-toggle-button'))
    expect(screen.queryByTestId('run-graph-canvas')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('run-console-toggle-button'))
    expect(screen.queryByTestId('run-console-output')).not.toBeInTheDocument()

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
      if (url.includes('/attractor/pipelines/run-selected/graph-preview')) {
        return jsonResponse({
          status: 'ok',
          graph: {
            graph_attrs: {},
            nodes: [
              { id: 'start', label: 'Start', shape: 'Mdiamond' },
              { id: 'done', label: 'Done', shape: 'Msquare' },
            ],
            edges: [
              { from: 'start', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
            ],
          },
          diagnostics: [],
          errors: [],
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
    renderRunsPanel()

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
