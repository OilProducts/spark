import { RunsSessionController } from '@/app/AppSessionControllers'
import { RunsPanel } from '@/features/runs/RunsPanel'
import { RunStream } from '@/features/runs/RunStream'
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
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle',
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
    runRecordOverrides: {},
    executionFlow: null,
    executionContinuation: null,
    workingDir: '',
    model: '',
    projectRegistry: {},
    projectSessionsByPath: {},
    recentProjectPaths: [],
    runsListSession: {
      scopeMode: 'active',
      selectedRunIdByScopeKey: {},
      status: 'idle',
      error: null,
      runs: [],
      lastFetchedAtMs: null,
      nowMs: Date.now(),
      metadataStaleAfterMs: 15000,
    },
    runDetailSessionsByRunId: {},
  })
}

const renderRunsPanel = () =>
  render(
    <DialogProvider>
      <RunsSessionController />
      <RunsPanel />
    </DialogProvider>,
  )

const renderRunsWorkspace = () =>
  render(
    <DialogProvider>
      <RunsSessionController />
      <RunStream />
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

  it('keeps the run selector ahead of the detail stack, uses the new activity-first hierarchy, and collapses the graph by default', async () => {
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
    const runActivityPanel = screen.getByTestId('run-activity-panel')
    const runTimelinePanel = screen.getByTestId('run-event-timeline-panel')
    const runCheckpointPanel = screen.getByTestId('run-checkpoint-panel')
    const runGraphPanel = screen.getByTestId('run-graph-panel')
    expect(screen.getByTestId('run-summary-spec-artifact-link')).toBeVisible()
    expect(screen.getByTestId('run-summary-plan-artifact-link')).toBeVisible()
    expect(screen.getByTestId('run-summary-cancel-button')).toBeEnabled()
    expect(runActivityPanel).toBeVisible()
    expect(runTimelinePanel).toBeVisible()
    expect(runCheckpointPanel).toBeVisible()
    expect(runGraphPanel).toBeVisible()
    expect(screen.getByTestId('run-summary-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-activity-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-graph-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-activity-logs-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-checkpoint-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-context-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-artifact-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-event-timeline-toggle-button')).toBeVisible()
    expect(screen.queryByTestId('run-graph-canvas')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-activity-logs-panel')).not.toBeInTheDocument()

    expect(
      runSummaryPanel.compareDocumentPosition(runActivityPanel) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
    expect(
      runActivityPanel.compareDocumentPosition(runTimelinePanel) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
    expect(
      runTimelinePanel.compareDocumentPosition(runCheckpointPanel) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
    expect(
      runCheckpointPanel.compareDocumentPosition(runGraphPanel) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()

    await user.click(screen.getByTestId('run-graph-toggle-button'))
    await waitFor(() => {
      expect(screen.getByTestId('run-graph-canvas')).toBeVisible()
    })

    await user.click(screen.getByTestId('run-activity-logs-toggle-button'))
    expect(screen.getByTestId('run-activity-logs-panel')).toBeVisible()

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

  it('hands off continuation from the selected run into execution mode', async () => {
    const selectedRun = makeRun({
      run_id: 'run-to-continue',
      flow_name: 'selected.dot',
      status: 'failed',
      project_path: '/tmp/project-one',
      working_directory: '/tmp/project-one/worktree',
      model: 'codex default (config/profile)',
    })

    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({ runs: [selectedRun] })
      }
      if (url.includes('/attractor/pipelines/run-to-continue/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-to-continue',
          checkpoint: {
            completed_nodes: ['prepare'],
            current_node: 'failed_node',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-to-continue/context')) {
        return jsonResponse({
          pipeline_id: 'run-to-continue',
          context: {
            active_item: 'REQ-001',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-to-continue/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-to-continue',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-to-continue/graph-preview')) {
        return jsonResponse({
          status: 'ok',
          graph: {
            graph_attrs: {},
            nodes: [
              { id: 'start', label: 'Start', shape: 'Mdiamond' },
              { id: 'failed_node', label: 'Failed Node', shape: 'box' },
              { id: 'done', label: 'Done', shape: 'Msquare' },
            ],
            edges: [
              { from: 'start', to: 'failed_node', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              { from: 'failed_node', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
            ],
          },
          diagnostics: [],
          errors: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-to-continue/questions')) {
        return jsonResponse({
          pipeline_id: 'run-to-continue',
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

    await user.click(screen.getByTestId('run-history-row'))

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-continue-button')).toBeVisible()
    })

    await user.click(screen.getByTestId('run-summary-continue-button'))

    expect(useStore.getState().viewMode).toBe('execution')
    expect(useStore.getState().activeProjectPath).toBe('/tmp/project-one')
    expect(useStore.getState().executionFlow).toBe('selected.dot')
    expect(useStore.getState().workingDir).toBe('/tmp/project-one/worktree')
    expect(useStore.getState().model).toBe('')
    expect(useStore.getState().executionContinuation).toEqual({
      sourceRunId: 'run-to-continue',
      sourceFlowName: 'selected.dot',
      sourceWorkingDirectory: '/tmp/project-one/worktree',
      sourceModel: 'codex default (config/profile)',
      flowSourceMode: 'snapshot',
      startNodeId: null,
    })
  })

  it('only shows continuation for inactive runs', async () => {
    const runningRun = makeRun({
      run_id: 'run-running',
      flow_name: 'selected.dot',
      status: 'running',
      ended_at: null,
    })

    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({ runs: [runningRun] })
      }
      if (url.includes('/attractor/pipelines/run-running/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-running',
          checkpoint: {
            completed_nodes: ['prepare'],
            current_node: 'work',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-running/context')) {
        return jsonResponse({
          pipeline_id: 'run-running',
          context: {},
        })
      }
      if (url.includes('/attractor/pipelines/run-running/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-running',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-running/graph-preview')) {
        return jsonResponse({
          status: 'ok',
          graph: {
            graph_attrs: {},
            nodes: [
              { id: 'start', label: 'Start', shape: 'Mdiamond' },
              { id: 'work', label: 'Work', shape: 'box' },
              { id: 'done', label: 'Done', shape: 'Msquare' },
            ],
            edges: [
              { from: 'start', to: 'work', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              { from: 'work', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
            ],
          },
          diagnostics: [],
          errors: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-running/questions')) {
        return jsonResponse({
          pipeline_id: 'run-running',
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

    await user.click(screen.getByTestId('run-history-row'))

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-panel')).toBeVisible()
    })

    expect(screen.queryByTestId('run-summary-continue-button')).not.toBeInTheDocument()
    expect(screen.getByTestId('run-summary-cancel-button')).toBeVisible()
  })

  it('converges the selected run summary, activity surface, and list row on authoritative run detail state', async () => {
    const staleRun = makeRun({
      run_id: 'run-stale-status',
      flow_name: 'selected.dot',
      status: 'running',
      outcome: null,
      ended_at: null,
      last_error: '',
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
        return jsonResponse({ runs: [staleRun] })
      }
      if (url.includes('/attractor/pipelines/run-stale-status/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-stale-status',
          checkpoint: {
            completed_nodes: ['prepare'],
            current_node: 'done',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-stale-status/context')) {
        return jsonResponse({
          pipeline_id: 'run-stale-status',
          context: { active_item: 'REQ-001' },
        })
      }
      if (url.includes('/attractor/pipelines/run-stale-status/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-stale-status',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-stale-status/graph-preview')) {
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
      if (url.includes('/attractor/pipelines/run-stale-status/questions')) {
        return jsonResponse({
          pipeline_id: 'run-stale-status',
          questions: [],
        })
      }
        if (url.endsWith('/attractor/pipelines/run-stale-status')) {
          return jsonResponse({
            pipeline_id: 'run-stale-status',
            run_id: 'run-stale-status',
            status: 'completed',
          outcome: 'success',
          outcome_reason_code: null,
          outcome_reason_message: null,
          flow_name: 'selected.dot',
          working_directory: '/tmp/project-one/workdir',
          project_path: '/tmp/project-one',
          git_branch: 'main',
          git_commit: 'abcdef0',
          spec_id: null,
          plan_id: null,
          model: 'gpt-5.3-codex-spark',
          started_at: '2026-03-22T00:00:00Z',
            ended_at: '2026-03-22T00:05:00Z',
            last_error: '',
            token_usage: 1234,
            current_node: 'done',
            completed_nodes: ['start', 'done'],
            progress: {
              current_node: 'done',
              completed_nodes: ['start', 'done'],
            },
            continued_from_run_id: null,
            continued_from_node: null,
            continued_from_flow_mode: null,
          continued_from_flow_name: null,
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
    })

    const user = userEvent.setup()
    renderRunsWorkspace()

    await waitFor(() => {
      expect(screen.getByText('selected.dot')).toBeVisible()
    })

    await user.click(screen.getByTestId('run-history-row'))

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-status')).toHaveTextContent('Completed')
    })

    expect(screen.getByTestId('run-activity-status')).toHaveTextContent('Completed')
    expect(screen.getByTestId('run-activity-headline')).toHaveTextContent('Completed successfully')
    expect(screen.getByTestId('run-history-row')).toHaveTextContent('Completed')
  })

  it('does not open a third explainability stream for the activity card', async () => {
    const selectedRun = makeRun({
      run_id: 'run-stream-count',
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
        return jsonResponse({ runs: [selectedRun] })
      }
      if (url.includes('/attractor/pipelines/run-stream-count/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-stream-count',
          checkpoint: {
            completed_nodes: ['prepare'],
            current_node: 'validate',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-stream-count/context')) {
        return jsonResponse({
          pipeline_id: 'run-stream-count',
          context: { active_item: 'REQ-001' },
        })
      }
      if (url.includes('/attractor/pipelines/run-stream-count/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-stream-count',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-stream-count/graph-preview')) {
        return jsonResponse({
          status: 'ok',
          graph: {
            graph_attrs: {},
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
      if (url.includes('/attractor/pipelines/run-stream-count/questions')) {
        return jsonResponse({
          pipeline_id: 'run-stream-count',
          questions: [],
        })
      }
      if (url.endsWith('/attractor/pipelines/run-stream-count')) {
        return jsonResponse({
          pipeline_id: 'run-stream-count',
          run_id: 'run-stream-count',
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
          token_usage: 1234,
          current_node: 'validate',
          completed_nodes: ['prepare'],
          progress: {
            current_node: 'validate',
            completed_nodes: ['prepare'],
          },
          continued_from_run_id: null,
          continued_from_node: null,
          continued_from_flow_mode: null,
          continued_from_flow_name: null,
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    const openedUrls: string[] = []
    class CountingEventSource {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 2
      readonly url: string
      readyState = CountingEventSource.OPEN
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent<string>) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string | URL) {
        this.url = String(url)
        openedUrls.push(this.url)
      }

      close() {
        this.readyState = CountingEventSource.CLOSED
      }
    }
    vi.stubGlobal('EventSource', CountingEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
    })

    const user = userEvent.setup()
    renderRunsWorkspace()

    await waitFor(() => {
      expect(screen.getByText('selected.dot')).toBeVisible()
    })

    await user.click(screen.getByTestId('run-history-row'))

    await waitFor(() => {
      expect(screen.getByTestId('run-activity-panel')).toBeVisible()
    })

    expect(openedUrls).toHaveLength(2)
  })
})
