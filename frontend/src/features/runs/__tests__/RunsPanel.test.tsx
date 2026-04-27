import { RunsSessionController } from '@/app/AppSessionControllers'
import { RunsPanel } from '@/features/runs/RunsPanel'
import { RunStream } from '@/features/runs/RunStream'
import {
  flattenRunJournalSegments,
  useRunJournalStore,
} from '@/features/runs/state/runJournalStore'
import { useStore } from '@/store'
import { DialogProvider } from '@/components/app/dialog-controller'
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
  useRunJournalStore.setState({ byRunId: {} })
  useStore.setState({
    viewMode: 'runs',
    activeProjectPath: null,
    selectedRunId: null,
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle',
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
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
      streamStatus: 'idle',
      streamError: null,
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
  outcome_reason_code: (overrides.outcome_reason_code as string | null | undefined) ?? null,
  outcome_reason_message: (overrides.outcome_reason_message as string | null | undefined) ?? null,
  working_directory: String(overrides.working_directory ?? '/tmp/workdir'),
  project_path: String(overrides.project_path ?? '/tmp/project-one'),
  git_branch: overrides.git_branch === undefined ? 'main' : (overrides.git_branch as string | null),
  git_commit: overrides.git_commit === undefined ? 'abcdef0' : (overrides.git_commit as string | null),
  spec_id: (overrides.spec_id as string | null | undefined) ?? null,
  plan_id: (overrides.plan_id as string | null | undefined) ?? null,
  model: String(overrides.model ?? 'gpt-5.3-codex-spark'),
  started_at: String(overrides.started_at ?? '2026-03-22T00:00:00Z'),
  ended_at: (overrides.ended_at as string | null | undefined) ?? '2026-03-22T00:05:00Z',
  last_error: (overrides.last_error as string | null | undefined) ?? null,
  token_usage: (overrides.token_usage as number | null | undefined) ?? 1234,
  token_usage_breakdown: (overrides.token_usage_breakdown as Record<string, unknown> | null | undefined) ?? null,
  estimated_model_cost: (overrides.estimated_model_cost as Record<string, unknown> | null | undefined) ?? null,
  current_node: (overrides.current_node as string | null | undefined) ?? null,
  continued_from_run_id: (overrides.continued_from_run_id as string | null | undefined) ?? null,
  continued_from_node: (overrides.continued_from_node as string | null | undefined) ?? null,
  continued_from_flow_mode: (overrides.continued_from_flow_mode as string | null | undefined) ?? null,
  continued_from_flow_name: (overrides.continued_from_flow_name as string | null | undefined) ?? null,
  parent_run_id: (overrides.parent_run_id as string | null | undefined) ?? null,
  parent_node_id: (overrides.parent_node_id as string | null | undefined) ?? null,
  root_run_id: (overrides.root_run_id as string | null | undefined) ?? null,
  child_invocation_index: (overrides.child_invocation_index as number | null | undefined) ?? null,
})

const makeJournalEntry = (
  sequence: number,
  payload: Record<string, unknown> & { type: string },
  overrides: Partial<Record<string, unknown>> = {},
) => {
  const rawType = String(payload.type)
  const defaultKind = rawType === 'log'
    ? 'log'
    : rawType === 'runtime'
      ? 'runtime'
      : rawType.startsWith('Interview')
        ? 'interview'
        : rawType.startsWith('Stage')
          ? 'stage'
          : 'other'
  return {
    id: `journal-${sequence}`,
    sequence,
    emitted_at: String(overrides.emitted_at ?? payload.emitted_at ?? '2026-03-22T00:00:00Z'),
    kind: String(overrides.kind ?? defaultKind),
    raw_type: rawType,
    severity: String(overrides.severity ?? 'info'),
    summary: String(overrides.summary ?? rawType),
    node_id: (overrides.node_id as string | null | undefined)
      ?? (payload.node_id as string | null | undefined)
      ?? (payload.stage as string | null | undefined)
      ?? null,
    stage_index: (overrides.stage_index as number | null | undefined)
      ?? (payload.index as number | null | undefined)
      ?? null,
    source_scope: (overrides.source_scope as string | null | undefined)
      ?? (payload.source_scope as string | null | undefined)
      ?? null,
    source_parent_node_id: (overrides.source_parent_node_id as string | null | undefined)
      ?? (payload.source_parent_node_id as string | null | undefined)
      ?? null,
    source_flow_name: (overrides.source_flow_name as string | null | undefined)
      ?? (payload.source_flow_name as string | null | undefined)
      ?? null,
    question_id: (overrides.question_id as string | null | undefined)
      ?? (payload.question_id as string | null | undefined)
      ?? null,
    payload,
  }
}

const makeJournalPage = (
  pipelineId: string,
  entries: ReturnType<typeof makeJournalEntry>[],
  hasOlder = false,
) => ({
  pipeline_id: pipelineId,
  entries,
  oldest_sequence: entries.at(-1)?.sequence ?? null,
  newest_sequence: entries[0]?.sequence ?? null,
  has_older: hasOlder,
})

const installControllableEventSource = () => {
  const eventSources: ControllableEventSource[] = []

  class ControllableEventSource {
    static readonly CONNECTING = 0
    static readonly OPEN = 1
    static readonly CLOSED = 2
    readonly url: string
    readyState = ControllableEventSource.OPEN
    onopen: ((event: Event) => void) | null = null
    onmessage: ((event: MessageEvent<string>) => void) | null = null
    onerror: ((event: Event) => void) | null = null

    constructor(url: string | URL) {
      this.url = String(url)
      eventSources.push(this)
    }

    emit(payload: unknown) {
      if (this.readyState === ControllableEventSource.CLOSED) {
        return
      }
      this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(payload) }))
    }

    open() {
      if (this.readyState === ControllableEventSource.CLOSED) {
        return
      }
      this.onopen?.(new Event('open'))
    }

    fail() {
      if (this.readyState === ControllableEventSource.CLOSED) {
        return
      }
      this.onerror?.(new Event('error'))
    }

    close() {
      this.readyState = ControllableEventSource.CLOSED
      this.onopen = null
      this.onmessage = null
      this.onerror = null
    }
  }

  vi.stubGlobal('EventSource', ControllableEventSource as unknown as typeof EventSource)

  return {
    eventSources,
    latestSourceMatching: (pattern: string) => (
      eventSources.filter((source) => source.url.includes(pattern)).at(-1) ?? null
    ),
    sourcesMatching: (pattern: string) => (
      eventSources.filter((source) => source.url.includes(pattern))
    ),
    CLOSED: ControllableEventSource.CLOSED,
  }
}

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
    renderRunsWorkspace()

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

  it('switches between active and all scopes by replacing the scoped runs stream', async () => {
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

    const { CLOSED, latestSourceMatching } = installControllableEventSource()

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
    })

    const user = userEvent.setup()
    renderRunsWorkspace()

    await waitFor(() => {
      expect(screen.getByText('project-one.dot')).toBeVisible()
      expect(latestSourceMatching('/attractor/runs/events')).toBeTruthy()
    })

    const activeScopeSource = latestSourceMatching('/attractor/runs/events')
    expect(activeScopeSource?.url).toContain('/attractor/runs/events?project_path=%2Ftmp%2Fproject-one')

    act(() => {
      activeScopeSource?.emit({
        type: 'run_upsert',
        run: makeRun({
          run_id: 'run-streamed-active',
          flow_name: 'streamed-active.dot',
          project_path: '/tmp/project-one',
          status: 'running',
          outcome: null,
          ended_at: null,
          started_at: '2026-03-22T00:06:00Z',
        }),
      })
    })

    await waitFor(() => {
      expect(screen.getByText('streamed-active.dot')).toBeVisible()
    })

    await user.click(screen.getByTestId('runs-scope-all-projects'))

    await waitFor(() => {
      expect(screen.getByText('project-two.dot')).toBeVisible()
    })

    const allProjectsSource = latestSourceMatching('/attractor/runs/events')
    expect(allProjectsSource).not.toBe(activeScopeSource)
    expect(activeScopeSource?.readyState).toBe(CLOSED)
    expect(allProjectsSource?.url).toMatch(/\/attractor\/runs\/events$/)
    expect(allProjectsSource?.url).not.toContain('project_path=')

    act(() => {
      activeScopeSource?.emit({
        type: 'run_upsert',
        run: makeRun({
          run_id: 'run-closed-source',
          flow_name: 'closed-source-update.dot',
          project_path: '/tmp/project-one',
        }),
      })
      allProjectsSource?.emit({
        type: 'run_upsert',
        run: makeRun({
          run_id: 'run-streamed-all',
          flow_name: 'streamed-all.dot',
          project_path: '/tmp/project-three',
          started_at: '2026-03-22T00:07:00Z',
        }),
      })
    })

    await waitFor(() => {
      expect(screen.getByText('streamed-all.dot')).toBeVisible()
    })
    expect(screen.queryByText('closed-source-update.dot')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('runs-scope-active-project'))

    await waitFor(() => {
      expect(screen.getByText('project-one.dot')).toBeVisible()
      expect(screen.queryByText('project-two.dot')).not.toBeInTheDocument()
    })

    const restoredActiveScopeSource = latestSourceMatching('/attractor/runs/events')
    expect(restoredActiveScopeSource).not.toBe(allProjectsSource)
    expect(allProjectsSource?.readyState).toBe(CLOSED)
    expect(restoredActiveScopeSource?.url).toContain('/attractor/runs/events?project_path=%2Ftmp%2Fproject-one')
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
    renderRunsWorkspace()

    expect(screen.getByText('Choose an active project or switch to all projects to view run history.')).toBeVisible()
    expect(fetchMock).not.toHaveBeenCalled()

    await user.click(screen.getByTestId('runs-scope-all-projects'))

    await waitFor(() => {
      expect(screen.getByText('global.dot')).toBeVisible()
    })
    expect(fetchMock).toHaveBeenCalled()
  })

  it('keeps the run selector ahead of the detail stack, folds monitoring into the summary, and collapses advanced evidence by default', async () => {
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
          questions: [{
            question_id: 'approve-1',
            node_id: 'validate',
            prompt: 'Approve the validation result?',
            type: 'CONFIRMATION',
          }],
        })
      }
      if (url.includes('/attractor/pipelines/run-selected/journal')) {
        return jsonResponse(makeJournalPage('run-selected', [
          makeJournalEntry(1, { type: 'StageStarted', node_id: 'validate' }, { summary: 'Stage validate started' }),
        ]))
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
      expect(screen.getByTestId('run-list-panel')).toBeVisible()
      expect(screen.getAllByTestId('run-history-row')).toHaveLength(2)
    })

    expect(screen.getByTestId('runs-panel')).toHaveAttribute('data-responsive-layout', 'split')
    expect(screen.getByTestId('runs-panel')).toHaveClass('h-full')

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

    const detailScrollRegion = screen.getByTestId('run-details-scroll-region')
    expect(detailScrollRegion).toHaveClass('min-h-0')
    expect(detailScrollRegion).toHaveClass('flex-1')
    expect(detailScrollRegion).toHaveClass('overflow-auto')

    const runSummaryPanel = screen.getByTestId('run-summary-panel')
    const nowSection = screen.getByTestId('run-summary-section-now')
    const outcomeSection = screen.getByTestId('run-summary-section-outcome')
    const scopeSection = screen.getByTestId('run-summary-section-scope')
    const usageSection = screen.getByTestId('run-summary-section-usage')
    const runActivityPanel = screen.getByTestId('run-activity-panel')
    const runTimelinePanel = screen.getByTestId('run-event-timeline-panel')
    const runAdvancedPanel = screen.getByTestId('run-advanced-panel')
    expect(screen.getByTestId('run-summary-spec-artifact-link')).toBeVisible()
    expect(screen.getByTestId('run-summary-plan-artifact-link')).toBeVisible()
    expect(screen.getByTestId('run-summary-cancel-button')).toBeEnabled()
    expect(nowSection).toHaveTextContent('Now')
    expect(outcomeSection).toHaveTextContent('Outcome')
    expect(scopeSection).toHaveTextContent('Scope')
    expect(usageSection).toHaveTextContent('Usage')
    expect(screen.getByTestId('run-summary-now-current-node')).toHaveTextContent('validate')
    expect(screen.getByTestId('run-summary-now-completed-nodes')).toHaveTextContent('1')
    await waitFor(() => {
      expect(screen.getByTestId('run-summary-now-pending-questions')).toHaveTextContent('1')
    })
    expect(screen.getByTestId('run-summary-now-waiting-state')).toHaveTextContent('Operator input')
    await waitFor(() => {
      expect(screen.getByTestId('run-summary-now-latest-journal')).toHaveTextContent('Stage validate started')
    })
    expect(runActivityPanel).toBeVisible()
    expect(runTimelinePanel).toBeVisible()
    expect(runAdvancedPanel).toBeVisible()
    expect(screen.queryByTestId('run-checkpoint-panel')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-graph-panel')).not.toBeInTheDocument()
    expect(screen.getByTestId('run-summary-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-advanced-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-event-timeline-toggle-button')).toBeVisible()
    expect(screen.queryByTestId('run-graph-canvas')).not.toBeInTheDocument()
    expect(runSummaryPanel).toContainElement(runActivityPanel)
    expect(
      runSummaryPanel.compareDocumentPosition(runTimelinePanel) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
    expect(
      nowSection.compareDocumentPosition(outcomeSection) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
    expect(
      outcomeSection.compareDocumentPosition(scopeSection) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
    expect(
      scopeSection.compareDocumentPosition(usageSection) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
    expect(
      runTimelinePanel.compareDocumentPosition(runAdvancedPanel) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()

    await user.click(screen.getByTestId('run-advanced-toggle-button'))

    await waitFor(() => {
      expect(screen.getByTestId('run-checkpoint-panel')).toBeVisible()
      expect(screen.getByTestId('run-graph-panel')).toBeVisible()
    })
    expect(screen.getByTestId('run-graph-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-checkpoint-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-context-toggle-button')).toBeVisible()
    expect(screen.getByTestId('run-artifact-toggle-button')).toBeVisible()
    expect(screen.queryByTestId('run-graph-canvas')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('run-graph-toggle-button'))
    await waitFor(() => {
      expect(screen.getByTestId('run-graph-canvas')).toBeVisible()
    })

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
    renderRunsWorkspace()

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
    renderRunsWorkspace()

    await waitFor(() => {
      expect(screen.getByText('selected.dot')).toBeVisible()
    })

    await user.click(screen.getByTestId('run-history-row'))

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-continue-button')).toBeVisible()
    })

    expect(screen.getByTestId('run-summary-section-scope')).toHaveTextContent('/tmp/project-one')

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
    renderRunsWorkspace()

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

  it('keeps outcome, working directory, and lineage rows compact and conditional', async () => {
    const failedRun = makeRun({
      run_id: 'run-business-failure',
      flow_name: 'failure.dot',
      status: 'completed',
      outcome: 'failure',
      outcome_reason_message: 'Release gate rejected',
      last_error: '',
      working_directory: '/tmp/project-one',
      project_path: '/tmp/project-one',
      git_branch: null,
      git_commit: null,
    })
    const lineageRun = makeRun({
      run_id: 'run-lineage',
      flow_name: 'lineage.dot',
      status: 'completed',
      outcome: 'success',
      working_directory: '/srv/spark/worktrees/run-lineage',
      project_path: '/tmp/project-one',
      continued_from_run_id: 'run-source',
      continued_from_node: 'review',
      parent_run_id: 'run-parent',
      parent_node_id: 'child_flow',
      root_run_id: 'run-root',
      child_invocation_index: 2,
    })
    const runsById = {
      [failedRun.run_id]: failedRun,
      [lineageRun.run_id]: lineageRun,
    }

    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({ runs: [failedRun, lineageRun] })
      }
      const pipelineMatch = url.match(/\/attractor\/pipelines\/([^/]+)\/([^/?#]+)/)
      const runId = pipelineMatch?.[1] ? decodeURIComponent(pipelineMatch[1]) : null
      const resource = pipelineMatch?.[2] ?? null
      if (runId && runId in runsById) {
        if (resource === 'checkpoint') {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              completed_nodes: ['prepare', 'execute'],
              current_node: 'done',
            },
          })
        }
        if (resource === 'context') {
          return jsonResponse({ pipeline_id: runId, context: {} })
        }
        if (resource === 'artifacts') {
          return jsonResponse({ pipeline_id: runId, artifacts: [] })
        }
        if (resource === 'questions') {
          return jsonResponse({ pipeline_id: runId, questions: [] })
        }
        if (resource === 'journal') {
          return jsonResponse(makeJournalPage(runId, []))
        }
        if (resource === 'graph-preview') {
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
      expect(screen.getByText('failure.dot')).toBeVisible()
      expect(screen.getByText('lineage.dot')).toBeVisible()
    })

    await user.click(screen.getAllByTestId('run-history-row')[0]!)

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-outcome')).toHaveTextContent('Failure')
    })
    expect(screen.getByTestId('run-summary-status')).toHaveTextContent('Completed')
    expect(screen.getByTestId('run-summary-outcome-reason')).toHaveTextContent('Release gate rejected')
    expect(screen.queryByTestId('run-summary-working-directory')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-summary-working-directory-note')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-summary-git-ref')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-summary-artifacts')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-summary-lineage')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-summary-last-error')).not.toBeInTheDocument()

    await user.click(screen.getAllByTestId('run-history-row')[1]!)

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-flow-name')).toHaveTextContent('lineage.dot')
    })
    expect(screen.getByTestId('run-summary-working-directory-note')).toHaveTextContent('Working dir differs')
    expect(screen.getByTestId('run-summary-working-directory-note')).toHaveTextContent('/srv/spark/worktrees/run-lineage')
    expect(screen.getByTestId('run-summary-lineage')).toHaveTextContent('Continued from run-source @ review')
    expect(screen.getByTestId('run-summary-lineage')).toHaveTextContent('Parent run-parent @ child_flow')
    expect(screen.getByTestId('run-summary-lineage')).toHaveTextContent('Root run-root')
    expect(screen.getByTestId('run-summary-lineage')).toHaveTextContent('Child invocation #2')
    expect(screen.queryByTestId('run-summary-continued-from')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-summary-parent-run')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-summary-root-run')).not.toBeInTheDocument()
    expect(screen.queryByTestId('run-summary-child-invocation')).not.toBeInTheDocument()
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
      if (url.includes('/attractor/pipelines/run-stale-status/journal')) {
        return jsonResponse(makeJournalPage('run-stale-status', []))
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

    class ConvergingEventSource {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 2
      readonly url: string
      readyState = ConvergingEventSource.OPEN
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
        this.readyState = ConvergingEventSource.CLOSED
      }
    }
    const eventSources: ConvergingEventSource[] = []
    const runsListSource = () => (
      eventSources.find((source) => source.url.includes('/attractor/runs/events')) ?? null
    )
    vi.stubGlobal('EventSource', ConvergingEventSource as unknown as typeof EventSource)

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

    act(() => {
      runsListSource()?.emit({
        type: 'run_upsert',
        run: {
          ...staleRun,
          status: 'completed',
          outcome: 'success',
          ended_at: '2026-03-22T00:05:00Z',
        },
      })
    })

    await waitFor(() => {
      expect(screen.getByTestId('run-history-row')).toHaveTextContent('Completed')
    })
  })

  it('opens one runs-list stream and one selected-run stream while a run is selected', async () => {
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

    expect(openedUrls.filter((url) => url.includes('/attractor/runs/events'))).toHaveLength(1)
    expect(openedUrls.filter((url) => url.includes('/attractor/pipelines/run-stream-count/events'))).toHaveLength(1)
  })

  it('applies live token telemetry from runs-list upserts to the selected run summary', async () => {
    const selectedRun = makeRun({
      run_id: 'run-live-usage',
      flow_name: 'selected.dot',
      status: 'running',
      outcome: null,
      ended_at: null,
      token_usage: null,
      token_usage_breakdown: null,
      estimated_model_cost: null,
      project_path: '/tmp/project-one',
      current_node: 'review',
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
      if (url.includes('/attractor/pipelines/run-live-usage/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-live-usage',
          checkpoint: {
            completed_nodes: ['start'],
            current_node: 'review',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-live-usage/context')) {
        return jsonResponse({
          pipeline_id: 'run-live-usage',
          context: {},
        })
      }
      if (url.includes('/attractor/pipelines/run-live-usage/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-live-usage',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-live-usage/graph-preview')) {
        return jsonResponse({
          status: 'ok',
          graph: {
            graph_attrs: {},
            nodes: [
              { id: 'start', label: 'Start', shape: 'Mdiamond' },
              { id: 'review', label: 'Review', shape: 'box' },
              { id: 'done', label: 'Done', shape: 'Msquare' },
            ],
            edges: [
              { from: 'start', to: 'review', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              { from: 'review', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
            ],
          },
          diagnostics: [],
          errors: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-live-usage/questions')) {
        return jsonResponse({
          pipeline_id: 'run-live-usage',
          questions: [],
        })
      }
      if (url.endsWith('/attractor/pipelines/run-live-usage')) {
        return jsonResponse({
          pipeline_id: 'run-live-usage',
          ...selectedRun,
          completed_nodes: ['start'],
          progress: {
            current_node: 'review',
            completed_nodes: ['start'],
          },
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    const { latestSourceMatching } = installControllableEventSource()

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
      expect(screen.getByTestId('run-summary-estimated-model-cost')).toHaveTextContent('—')
    })

    act(() => {
      latestSourceMatching('/attractor/runs/events')?.emit({
        type: 'run_upsert',
        run: {
          ...selectedRun,
          token_usage: 36,
          token_usage_breakdown: {
            input_tokens: 23,
            cached_input_tokens: 3,
            output_tokens: 13,
            total_tokens: 36,
            by_model: {
              'gpt-5.4': {
                input_tokens: 15,
                cached_input_tokens: 3,
                output_tokens: 9,
                total_tokens: 24,
              },
              'gpt-5.3-codex-spark': {
                input_tokens: 8,
                cached_input_tokens: 0,
                output_tokens: 4,
                total_tokens: 12,
              },
            },
          },
          estimated_model_cost: {
            currency: 'USD',
            amount: 0.000166,
            status: 'partial_unpriced',
            unpriced_models: ['gpt-5.3-codex-spark'],
            by_model: {
              'gpt-5.4': {
                currency: 'USD',
                amount: 0.000166,
                status: 'estimated',
              },
              'gpt-5.3-codex-spark': {
                currency: 'USD',
                amount: null,
                status: 'unpriced',
              },
            },
          },
        },
      })
    })

    await waitFor(() => {
      expect(screen.getByTestId('run-summary-estimated-model-cost')).toHaveTextContent('$0.000166')
    })
    expect(screen.getByTestId('run-summary-estimated-model-cost-note')).toHaveTextContent(
      'Unpriced models excluded from the subtotal: gpt-5.3-codex-spark',
    )
    expect(screen.getByTestId('run-summary-token-usage')).toHaveTextContent('36')
    expect(screen.getAllByTestId('run-summary-model-row')).toHaveLength(2)
    expect(screen.getByTestId('run-summary-model-breakdown')).toHaveTextContent('gpt-5.4')
    expect(screen.getByTestId('run-summary-model-breakdown')).toHaveTextContent('gpt-5.3-codex-spark')
  })

  it('keeps selected-run detail fetches scoped to run id changes instead of same-run stream updates', async () => {
    const selectedRun = makeRun({
      run_id: 'run-refetch-selected',
      flow_name: 'selected.dot',
      status: 'running',
      outcome: null,
      ended_at: null,
      project_path: '/tmp/project-one',
    })
    const otherRun = makeRun({
      run_id: 'run-refetch-other',
      flow_name: 'other.dot',
      status: 'running',
      outcome: null,
      ended_at: null,
      project_path: '/tmp/project-one',
    })
    const runsById = {
      [selectedRun.run_id]: selectedRun,
      [otherRun.run_id]: otherRun,
    }
    const currentNodeByRunId = {
      [selectedRun.run_id]: 'validate',
      [otherRun.run_id]: 'review',
    }
    const detailResources = ['checkpoint', 'context', 'artifacts', 'questions'] as const

    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({ runs: [selectedRun, otherRun] })
      }
      const pipelineStatusMatch = url.match(/\/attractor\/pipelines\/([^/?#]+)$/)
      const pipelineStatusRunId = pipelineStatusMatch?.[1] ? decodeURIComponent(pipelineStatusMatch[1]) : null
      if (pipelineStatusRunId && pipelineStatusRunId in runsById) {
        const run = runsById[pipelineStatusRunId as keyof typeof runsById]
        const currentNode = currentNodeByRunId[pipelineStatusRunId as keyof typeof currentNodeByRunId]
        return jsonResponse({
          pipeline_id: run.run_id,
          run_id: run.run_id,
          flow_name: run.flow_name,
          status: run.status,
          outcome: run.outcome,
          outcome_reason_code: null,
          outcome_reason_message: null,
          working_directory: run.working_directory,
          project_path: run.project_path,
          git_branch: run.git_branch,
          git_commit: run.git_commit,
          spec_id: null,
          plan_id: null,
          model: run.model,
          started_at: run.started_at,
          ended_at: run.ended_at,
          last_error: run.last_error ?? '',
          token_usage: run.token_usage,
          current_node: currentNode,
          completed_nodes: ['prepare'],
          progress: {
            current_node: currentNode,
            completed_nodes: ['prepare'],
          },
          continued_from_run_id: null,
          continued_from_node: null,
          continued_from_flow_mode: null,
          continued_from_flow_name: null,
        })
      }
      const pipelineMatch = url.match(/\/attractor\/pipelines\/([^/]+)\/([^/?#]+)/)
      const runId = pipelineMatch?.[1] ? decodeURIComponent(pipelineMatch[1]) : null
      const resource = pipelineMatch?.[2] ?? null
      if (runId && runId in runsById) {
        if (resource === 'checkpoint') {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              completed_nodes: ['prepare'],
              current_node: currentNodeByRunId[runId as keyof typeof currentNodeByRunId],
            },
          })
        }
        if (resource === 'context') {
          return jsonResponse({
            pipeline_id: runId,
            context: { active_item: `REQ-${runId}` },
          })
        }
        if (resource === 'artifacts') {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (resource === 'graph-preview') {
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
        if (resource === 'questions') {
          return jsonResponse({
            pipeline_id: runId,
            questions: [],
          })
        }
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    const { latestSourceMatching, sourcesMatching } = installControllableEventSource()
    const countDetailFetches = (runId: string, resource: typeof detailResources[number]) => (
      fetchMock.mock.calls.filter(([request, init]) => {
        const method = init?.method ?? 'GET'
        return method === 'GET'
          && resolveRequestUrl(request as RequestInfo | URL).includes(`/attractor/pipelines/${encodeURIComponent(runId)}/${resource}`)
      }).length
    )

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
    })

    const user = userEvent.setup()
    renderRunsWorkspace()

    await waitFor(() => {
      expect(screen.getByText('selected.dot')).toBeVisible()
    })

    const selectedRunCard = screen.getByText('selected.dot').closest('[data-testid="run-history-row"]')
    expect(selectedRunCard).toBeTruthy()
    await user.click(selectedRunCard!)

    await waitFor(() => {
      expect(screen.getByTestId('run-activity-panel')).toBeVisible()
      expect(sourcesMatching(`/attractor/pipelines/${selectedRun.run_id}/events`)).toHaveLength(1)
    })

    detailResources.forEach((resource) => {
      expect(countDetailFetches(selectedRun.run_id, resource)).toBe(1)
    })

    const selectedRunSource = latestSourceMatching(`/attractor/pipelines/${selectedRun.run_id}/events`)
    expect(selectedRunSource).toBeTruthy()

    act(() => {
      selectedRunSource?.open()
      selectedRunSource?.emit({
        type: 'StageStarted',
        sequence: 1,
        emitted_at: '2026-03-22T00:02:00Z',
        node_id: 'work',
        index: 2,
      })
      selectedRunSource?.emit({
        type: 'state',
        node: 'done',
        status: 'running',
      })
    })

    await waitFor(() => {
      expect(useStore.getState().selectedRunRecord?.current_node).toBe('done')
    })

    detailResources.forEach((resource) => {
      expect(countDetailFetches(selectedRun.run_id, resource)).toBe(1)
    })

    const otherRunCard = screen.getByText('other.dot').closest('[data-testid="run-history-row"]')
    expect(otherRunCard).toBeTruthy()
    await user.click(otherRunCard!)

    await waitFor(() => {
      expect(sourcesMatching(`/attractor/pipelines/${otherRun.run_id}/events`)).toHaveLength(1)
      expect(useStore.getState().selectedRunId).toBe(otherRun.run_id)
    })

    detailResources.forEach((resource) => {
      expect(countDetailFetches(otherRun.run_id, resource)).toBe(1)
    })
  })

  it('reconnects the runs list and selected run transports from the global reconnect control', async () => {
    const selectedRun = makeRun({
      run_id: 'run-reconnect',
      flow_name: 'selected.dot',
      status: 'running',
      outcome: null,
      ended_at: null,
      project_path: '/tmp/project-one',
    })
    const pipelineStatusUrl = '/attractor/pipelines/run-reconnect'
    const pipelineEventsUrl = '/attractor/pipelines/run-reconnect/events'
    const scopedRunsUrl = '/attractor/runs?project_path=%2Ftmp%2Fproject-one'
    const scopedRunsEventsUrl = '/attractor/runs/events?project_path=%2Ftmp%2Fproject-one'

    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes(scopedRunsUrl)) {
        return jsonResponse({ runs: [selectedRun] })
      }
      if (url.includes('/attractor/pipelines/run-reconnect/checkpoint')) {
        return jsonResponse({
          pipeline_id: 'run-reconnect',
          checkpoint: {
            completed_nodes: ['prepare'],
            current_node: 'validate',
          },
        })
      }
      if (url.includes('/attractor/pipelines/run-reconnect/context')) {
        return jsonResponse({
          pipeline_id: 'run-reconnect',
          context: { active_item: 'REQ-001' },
        })
      }
      if (url.includes('/attractor/pipelines/run-reconnect/artifacts')) {
        return jsonResponse({
          pipeline_id: 'run-reconnect',
          artifacts: [],
        })
      }
      if (url.includes('/attractor/pipelines/run-reconnect/graph-preview')) {
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
      if (url.includes('/attractor/pipelines/run-reconnect/questions')) {
        return jsonResponse({
          pipeline_id: 'run-reconnect',
          questions: [],
        })
      }
      if (url.endsWith(pipelineStatusUrl)) {
        return jsonResponse({
          pipeline_id: 'run-reconnect',
          run_id: 'run-reconnect',
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

    const { latestSourceMatching, sourcesMatching } = installControllableEventSource()

    const countGetRequests = (predicate: (url: string) => boolean) => (
      fetchMock.mock.calls.filter(([request, init]) => {
        const method = init?.method ?? 'GET'
        return method === 'GET' && predicate(resolveRequestUrl(request as RequestInfo | URL))
      }).length
    )

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
    })

    const user = userEvent.setup()
    renderRunsWorkspace()

    await waitFor(() => {
      expect(screen.getByText('selected.dot')).toBeVisible()
      expect(sourcesMatching('/attractor/runs/events')).toHaveLength(1)
    })

    await user.click(screen.getByTestId('run-history-row'))

    await waitFor(() => {
      expect(screen.getByTestId('run-activity-panel')).toBeVisible()
      expect(sourcesMatching(pipelineEventsUrl)).toHaveLength(1)
    })

    const initialRunsFetchCount = countGetRequests((url) => url.includes(scopedRunsUrl))
    const initialPipelineFetchCount = countGetRequests((url) => url.endsWith(pipelineStatusUrl))
    const initialRunsSource = latestSourceMatching(scopedRunsEventsUrl)
    const initialPipelineSource = latestSourceMatching(pipelineEventsUrl)

    act(() => {
      initialRunsSource?.fail()
      initialPipelineSource?.fail()
    })

    await waitFor(() => {
      expect(screen.getByTestId('runs-transport-reconnect-banner')).toBeVisible()
    })

    await user.click(screen.getByTestId('runs-transport-reconnect-button'))

    await waitFor(() => {
      expect(countGetRequests((url) => url.includes(scopedRunsUrl))).toBe(initialRunsFetchCount + 1)
      expect(countGetRequests((url) => url.endsWith(pipelineStatusUrl))).toBe(initialPipelineFetchCount + 1)
      expect(sourcesMatching(scopedRunsEventsUrl)).toHaveLength(2)
      expect(sourcesMatching(pipelineEventsUrl)).toHaveLength(2)
    })

    const reconnectedRunsSource = latestSourceMatching(scopedRunsEventsUrl)
    const reconnectedPipelineSource = latestSourceMatching(pipelineEventsUrl)
    expect(reconnectedRunsSource).not.toBe(initialRunsSource)
    expect(reconnectedPipelineSource).not.toBe(initialPipelineSource)

    act(() => {
      reconnectedRunsSource?.open()
      reconnectedPipelineSource?.open()
    })

    await waitFor(() => {
      expect(screen.queryByTestId('runs-transport-reconnect-banner')).not.toBeInTheDocument()
    })
  })

  it('hydrates durable journal history, shows child events inline, and deduplicates reconnects and reselects', async () => {
    const selectedRun = makeRun({
      run_id: 'run-history-replay',
      flow_name: 'selected.dot',
      status: 'completed',
      outcome: 'success',
      project_path: '/tmp/project-one',
      ended_at: '2026-03-22T00:05:00Z',
    })
    const otherRun = makeRun({
      run_id: 'run-history-secondary',
      flow_name: 'other.dot',
      status: 'completed',
      outcome: 'success',
      project_path: '/tmp/project-one',
      ended_at: '2026-03-22T00:06:00Z',
    })
    const runsById = {
      [selectedRun.run_id]: selectedRun,
      [otherRun.run_id]: otherRun,
    }
    const selectedRunJournalEntries = [
      makeJournalEntry(
        3,
        {
          type: 'StageCompleted',
          sequence: 3,
          emitted_at: '2026-03-22T00:04:00Z',
          node_id: 'plan_current',
          index: 2,
          source_scope: 'child',
          source_parent_node_id: 'run_milestone',
          source_flow_name: 'implement-milestone.dot',
        },
        {
          kind: 'stage',
          summary: 'Child flow implement-milestone.dot via run_milestone: Stage plan_current completed',
        },
      ),
      makeJournalEntry(
        2,
        {
          type: 'StageStarted',
          sequence: 2,
          emitted_at: '2026-03-22T00:03:00Z',
          node_id: 'plan_current',
          index: 2,
          source_scope: 'child',
          source_parent_node_id: 'run_milestone',
          source_flow_name: 'implement-milestone.dot',
        },
        {
          kind: 'stage',
          summary: 'Child flow implement-milestone.dot via run_milestone: Stage plan_current started',
        },
      ),
      makeJournalEntry(
        1,
        {
          type: 'StageCompleted',
          sequence: 1,
          emitted_at: '2026-03-22T00:02:00Z',
          node_id: 'prepare',
          index: 1,
          source_scope: 'root',
        },
        {
          kind: 'stage',
          summary: 'Stage prepare completed',
        },
      ),
    ]

    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({ runs: [selectedRun, otherRun] })
      }
      const pipelineStatusMatch = url.match(/\/attractor\/pipelines\/([^/?#]+)$/)
      const pipelineStatusRunId = pipelineStatusMatch?.[1] ? decodeURIComponent(pipelineStatusMatch[1]) : null
      if (pipelineStatusRunId && pipelineStatusRunId in runsById) {
        const run = runsById[pipelineStatusRunId as keyof typeof runsById]
        return jsonResponse({
          pipeline_id: run.run_id,
          run_id: run.run_id,
          flow_name: run.flow_name,
          status: run.status,
          outcome: run.outcome,
          outcome_reason_code: null,
          outcome_reason_message: null,
          working_directory: run.working_directory,
          project_path: run.project_path,
          git_branch: run.git_branch,
          git_commit: run.git_commit,
          spec_id: null,
          plan_id: null,
          model: run.model,
          started_at: run.started_at,
          ended_at: run.ended_at,
          last_error: run.last_error ?? '',
          token_usage: run.token_usage,
          current_node: 'done',
          completed_nodes: ['prepare', 'done'],
          progress: {
            current_node: 'done',
            completed_nodes: ['prepare', 'done'],
          },
          continued_from_run_id: null,
          continued_from_node: null,
          continued_from_flow_mode: null,
          continued_from_flow_name: null,
        })
      }
      const pipelineMatch = url.match(/\/attractor\/pipelines\/([^/]+)\/([^/?#]+)/)
      const runId = pipelineMatch?.[1] ? decodeURIComponent(pipelineMatch[1]) : null
      const resource = pipelineMatch?.[2] ?? null
      if (runId && runId in runsById) {
        if (resource === 'checkpoint') {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              completed_nodes: ['prepare'],
              current_node: 'done',
            },
          })
        }
        if (resource === 'context') {
          return jsonResponse({
            pipeline_id: runId,
            context: {},
          })
        }
        if (resource === 'artifacts') {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (resource === 'graph-preview') {
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
        if (resource === 'questions') {
          return jsonResponse({
            pipeline_id: runId,
            questions: [],
          })
        }
        if (resource === 'journal') {
          return jsonResponse(makeJournalPage(
            runId,
            runId === selectedRun.run_id ? selectedRunJournalEntries : [],
          ))
        }
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    class ReplayEventSource {
      static readonly CONNECTING = 0
      static readonly OPEN = 1
      static readonly CLOSED = 2
      readonly url: string
      readyState = ReplayEventSource.OPEN
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
        this.readyState = ReplayEventSource.CLOSED
      }
    }
    const eventSources: ReplayEventSource[] = []
    const runEventSources = () => (
      eventSources.filter((source) => source.url.includes('/attractor/pipelines/'))
    )
    const latestEventSourceForRun = (runId: string) => (
      runEventSources()
        .filter((source) => source.url.includes(`/attractor/pipelines/${encodeURIComponent(runId)}/events`))
        .at(-1) ?? null
    )
    vi.stubGlobal('EventSource', ReplayEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.getState().registerProject('/tmp/project-one')
      useStore.getState().setActiveProjectPath('/tmp/project-one')
    })

    const user = userEvent.setup()
    renderRunsWorkspace()

    await waitFor(() => {
      expect(screen.getByText('selected.dot')).toBeVisible()
    })

    const selectedRunCard = screen.getByText('selected.dot').closest('[data-testid="run-history-row"]')
    expect(selectedRunCard).toBeTruthy()
    await user.click(selectedRunCard!)

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-panel')).toBeVisible()
      expect(runEventSources()).toHaveLength(1)
    })

    const initialReplaySource = latestEventSourceForRun(selectedRun.run_id)
    expect(initialReplaySource).toBeTruthy()
    expect(initialReplaySource?.url).toContain('after_sequence=3')

    await waitFor(() => {
      expect(screen.getAllByTestId('run-event-timeline-row')).toHaveLength(3)
    })
    expect(screen.getAllByTestId('run-event-timeline-row-summary')).toHaveLength(3)
    expect(screen.getAllByTestId('run-event-timeline-row-source')).toHaveLength(2)
    expect(screen.getAllByTestId('run-event-timeline-row-source')[0]).toHaveTextContent(
      'Source: Child flow implement-milestone.dot via run_milestone',
    )

    const otherRunCard = screen.getByText('other.dot').closest('[data-testid="run-history-row"]')
    expect(otherRunCard).toBeTruthy()
    await user.click(otherRunCard!)

    await waitFor(() => {
      expect(runEventSources()).toHaveLength(2)
      expect(initialReplaySource?.readyState).toBe(ReplayEventSource.CLOSED)
    })

    const reselectedRunCard = screen.getByText('selected.dot').closest('[data-testid="run-history-row"]')
    expect(reselectedRunCard).toBeTruthy()
    await user.click(reselectedRunCard!)

    await waitFor(() => {
      expect(runEventSources()).toHaveLength(3)
      expect(screen.getByTestId('run-event-timeline-panel')).toBeVisible()
    })

    const replayAfterReselect = latestEventSourceForRun(selectedRun.run_id)
    expect(replayAfterReselect).toBeTruthy()
    expect(replayAfterReselect).not.toBe(initialReplaySource)
    expect(replayAfterReselect?.url).toContain('after_sequence=3')

    act(() => {
      replayAfterReselect!.open()
      replayAfterReselect!.emit({
        type: 'StageCompleted',
        sequence: 3,
        emitted_at: '2026-03-22T00:04:00Z',
        node_id: 'plan_current',
        index: 2,
        source_scope: 'child',
        source_parent_node_id: 'run_milestone',
        source_flow_name: 'implement-milestone.dot',
      })
      replayAfterReselect!.emit({
        type: 'StageCompleted',
        sequence: 4,
        emitted_at: '2026-03-22T00:05:00Z',
        node_id: 'done',
        index: 3,
      })
    })

    await waitFor(() => {
      expect(screen.getAllByTestId('run-event-timeline-row')).toHaveLength(4)
    })

    expect(
      flattenRunJournalSegments(useRunJournalStore.getState().byRunId[selectedRun.run_id]?.segments ?? [])
        .map(({ sequence }) => sequence),
    ).toEqual([4, 3, 2, 1])

    const timelineSummaries = screen.getAllByTestId('run-event-timeline-row-summary').map((node) => node.textContent ?? '')
    expect(timelineSummaries.filter((summary) => summary.includes('Stage plan_current started'))).toHaveLength(1)
    expect(timelineSummaries.filter((summary) => summary.includes('Stage plan_current completed'))).toHaveLength(1)
    expect(timelineSummaries).toEqual([
      'Stage done completed',
      'Child flow implement-milestone.dot via run_milestone: Stage plan_current completed',
      'Child flow implement-milestone.dot via run_milestone: Stage plan_current started',
      'Stage prepare completed',
    ])
  })

  it('keeps Load older available when filters empty the loaded journal and reveals older matching history', async () => {
    const selectedRun = makeRun({
      run_id: 'run-history-filtered-empty',
      flow_name: 'selected.dot',
      status: 'completed',
      outcome: 'success',
      project_path: '/tmp/project-one',
      ended_at: '2026-03-22T00:05:00Z',
    })
    const latestEntries = [
      makeJournalEntry(
        4,
        {
          type: 'StageCompleted',
          sequence: 4,
          emitted_at: '2026-03-22T00:05:00Z',
          node_id: 'done',
          index: 3,
          source_scope: 'root',
        },
        {
          kind: 'stage',
          severity: 'info',
          summary: 'Stage done completed',
        },
      ),
      makeJournalEntry(
        3,
        {
          type: 'StageCompleted',
          sequence: 3,
          emitted_at: '2026-03-22T00:04:00Z',
          node_id: 'plan_current',
          index: 2,
          source_scope: 'root',
        },
        {
          kind: 'stage',
          severity: 'info',
          summary: 'Stage plan_current completed',
        },
      ),
    ]
    const olderEntries = [
      makeJournalEntry(
        2,
        {
          type: 'StageFailed',
          sequence: 2,
          emitted_at: '2026-03-22T00:03:00Z',
          node_id: 'legacy_validate',
          index: 2,
          source_scope: 'root',
          error: 'validation gate rejected',
        },
        {
          kind: 'stage',
          severity: 'error',
          summary: 'Stage legacy_validate failed: validation gate rejected',
        },
      ),
      makeJournalEntry(
        1,
        {
          type: 'StageStarted',
          sequence: 1,
          emitted_at: '2026-03-22T00:02:00Z',
          node_id: 'prepare',
          index: 1,
          source_scope: 'root',
        },
        {
          kind: 'stage',
          severity: 'info',
          summary: 'Stage prepare started',
        },
      ),
    ]
    const journalRequestUrls: string[] = []

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
      if (url.endsWith(`/attractor/pipelines/${selectedRun.run_id}`)) {
        return jsonResponse({
          pipeline_id: selectedRun.run_id,
          run_id: selectedRun.run_id,
          flow_name: selectedRun.flow_name,
          status: selectedRun.status,
          outcome: selectedRun.outcome,
          outcome_reason_code: null,
          outcome_reason_message: null,
          working_directory: selectedRun.working_directory,
          project_path: selectedRun.project_path,
          git_branch: selectedRun.git_branch,
          git_commit: selectedRun.git_commit,
          spec_id: null,
          plan_id: null,
          model: selectedRun.model,
          started_at: selectedRun.started_at,
          ended_at: selectedRun.ended_at,
          last_error: selectedRun.last_error ?? '',
          token_usage: selectedRun.token_usage,
          current_node: 'done',
          completed_nodes: ['prepare', 'done'],
          progress: {
            current_node: 'done',
            completed_nodes: ['prepare', 'done'],
          },
          continued_from_run_id: null,
          continued_from_node: null,
          continued_from_flow_mode: null,
          continued_from_flow_name: null,
        })
      }
      const pipelineMatch = url.match(/\/attractor\/pipelines\/([^/]+)\/([^/?#]+)/)
      const runId = pipelineMatch?.[1] ? decodeURIComponent(pipelineMatch[1]) : null
      const resource = pipelineMatch?.[2] ?? null
      if (runId === selectedRun.run_id) {
        if (resource === 'checkpoint') {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              completed_nodes: ['prepare'],
              current_node: 'done',
            },
          })
        }
        if (resource === 'context') {
          return jsonResponse({
            pipeline_id: runId,
            context: {},
          })
        }
        if (resource === 'artifacts') {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (resource === 'graph-preview') {
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
        if (resource === 'questions') {
          return jsonResponse({
            pipeline_id: runId,
            questions: [],
          })
        }
        if (resource === 'journal') {
          const requestUrl = new URL(url, 'http://localhost')
          journalRequestUrls.push(requestUrl.toString())
          if (requestUrl.searchParams.get('before_sequence') === '3') {
            return jsonResponse(makeJournalPage(runId, olderEntries, false))
          }
          return jsonResponse(makeJournalPage(runId, latestEntries, true))
        }
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

    const selectedRunCard = screen.getByText('selected.dot').closest('[data-testid="run-history-row"]')
    expect(selectedRunCard).toBeTruthy()
    await user.click(selectedRunCard!)

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-panel')).toBeVisible()
      expect(screen.getByTestId('run-journal-load-older')).toBeVisible()
    })

    await user.selectOptions(screen.getByTestId('run-event-timeline-filter-severity'), 'error')

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-empty')).toHaveTextContent('No journal entries match the current filters.')
      expect(screen.getByTestId('run-journal-load-older')).toBeVisible()
      expect(screen.queryByTestId('run-event-timeline-list')).not.toBeInTheDocument()
    })

    await user.click(screen.getByTestId('run-journal-load-older'))

    await waitFor(() => {
      expect(screen.queryByTestId('run-event-timeline-empty')).not.toBeInTheDocument()
      expect(screen.getByTestId('run-event-timeline-list')).toHaveTextContent('Stage legacy_validate failed: validation gate rejected')
      expect(screen.getAllByTestId('run-event-timeline-row-summary')).toHaveLength(1)
      expect(screen.queryByTestId('run-journal-load-older')).not.toBeInTheDocument()
    })

    expect(
      journalRequestUrls.filter((requestUrl) => requestUrl.includes(`/attractor/pipelines/${selectedRun.run_id}/journal?limit=100&before_sequence=3`)),
    ).toHaveLength(1)
    expect(
      flattenRunJournalSegments(useRunJournalStore.getState().byRunId[selectedRun.run_id]?.segments ?? [])
        .map(({ sequence }) => sequence),
    ).toEqual([4, 3, 2, 1])
  })

  it('bounds rendered journal rows when a single retry correlation group contains long history', async () => {
    const selectedRun = makeRun({
      run_id: 'run-history-large-retry-group',
      flow_name: 'selected.dot',
      status: 'completed',
      outcome: 'failure',
      project_path: '/tmp/project-one',
      ended_at: '2026-03-22T00:05:00Z',
    })
    const groupedHistory = Array.from({ length: 140 }, (_, index) => {
      const sequence = 140 - index
      return makeJournalEntry(
        sequence,
        {
          type: 'StageRetrying',
          sequence,
          emitted_at: new Date(Date.UTC(2026, 2, 22, 0, 0, sequence)).toISOString(),
          node_id: 'review_loop',
          index: 2,
          source_scope: 'root',
          attempt: sequence,
        },
        {
          kind: 'stage',
          severity: sequence % 2 === 0 ? 'warning' : 'info',
          summary: `Retry attempt ${sequence}`,
        },
      )
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
      if (url.endsWith(`/attractor/pipelines/${selectedRun.run_id}`)) {
        return jsonResponse({
          pipeline_id: selectedRun.run_id,
          run_id: selectedRun.run_id,
          flow_name: selectedRun.flow_name,
          status: selectedRun.status,
          outcome: selectedRun.outcome,
          outcome_reason_code: null,
          outcome_reason_message: 'Retry budget exhausted',
          working_directory: selectedRun.working_directory,
          project_path: selectedRun.project_path,
          git_branch: selectedRun.git_branch,
          git_commit: selectedRun.git_commit,
          spec_id: null,
          plan_id: null,
          model: selectedRun.model,
          started_at: selectedRun.started_at,
          ended_at: selectedRun.ended_at,
          last_error: selectedRun.last_error ?? '',
          token_usage: selectedRun.token_usage,
          current_node: 'review_loop',
          completed_nodes: ['prepare'],
          progress: {
            current_node: 'review_loop',
            completed_nodes: ['prepare'],
          },
          continued_from_run_id: null,
          continued_from_node: null,
          continued_from_flow_mode: null,
          continued_from_flow_name: null,
        })
      }
      const pipelineMatch = url.match(/\/attractor\/pipelines\/([^/]+)\/([^/?#]+)/)
      const runId = pipelineMatch?.[1] ? decodeURIComponent(pipelineMatch[1]) : null
      const resource = pipelineMatch?.[2] ?? null
      if (runId === selectedRun.run_id) {
        if (resource === 'checkpoint') {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              completed_nodes: ['prepare'],
              current_node: 'review_loop',
            },
          })
        }
        if (resource === 'context') {
          return jsonResponse({
            pipeline_id: runId,
            context: {},
          })
        }
        if (resource === 'artifacts') {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (resource === 'graph-preview') {
          return jsonResponse({
            status: 'ok',
            graph: {
              graph_attrs: {},
              nodes: [
                { id: 'start', label: 'Start', shape: 'Mdiamond' },
                { id: 'review_loop', label: 'Review Loop', shape: 'box' },
                { id: 'done', label: 'Done', shape: 'Msquare' },
              ],
              edges: [
                { from: 'start', to: 'review_loop', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
                { from: 'review_loop', to: 'done', label: null, condition: null, weight: null, fidelity: null, thread_id: null, loop_restart: false },
              ],
            },
            diagnostics: [],
            errors: [],
          })
        }
        if (resource === 'questions') {
          return jsonResponse({
            pipeline_id: runId,
            questions: [],
          })
        }
        if (resource === 'journal') {
          return jsonResponse(makeJournalPage(runId, groupedHistory, false))
        }
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

    const selectedRunCard = screen.getByText('selected.dot').closest('[data-testid="run-history-row"]')
    expect(selectedRunCard).toBeTruthy()
    await user.click(selectedRunCard!)

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-panel')).toBeVisible()
      expect(screen.getByTestId('run-event-timeline-throughput')).toHaveAttribute('data-loaded-count', '140')
      expect(screen.getAllByTestId('run-event-timeline-group')).toHaveLength(1)
    })

    const throughput = screen.getByTestId('run-event-timeline-throughput')
    const renderedCount = Number(throughput.getAttribute('data-rendered-count') ?? '0')

    expect(renderedCount).toBeGreaterThan(0)
    expect(renderedCount).toBeLessThan(groupedHistory.length)
    expect(renderedCount).toBeLessThanOrEqual(80)
    expect(screen.getAllByTestId('run-event-timeline-row')).toHaveLength(renderedCount)
    expect(screen.getByTestId('run-event-timeline-group-label')).toHaveTextContent('Retry sequence for review_loop')
    expect(screen.getByText('140 entries')).toBeVisible()
    expect(screen.getAllByTestId('run-event-timeline-row-summary')[0]).toHaveTextContent('Retry attempt 140')
    expect(
      screen.getAllByTestId('run-event-timeline-row-summary').some((node) => node.textContent === 'Retry attempt 1'),
    ).toBe(false)
  })

  it('keeps exhausted journal history marked complete after reselect so Load older does not reappear', async () => {
    const selectedRun = makeRun({
      run_id: 'run-history-exhausted',
      flow_name: 'selected.dot',
      status: 'completed',
      outcome: 'success',
      project_path: '/tmp/project-one',
      ended_at: '2026-03-22T00:05:00Z',
    })
    const otherRun = makeRun({
      run_id: 'run-history-secondary',
      flow_name: 'other.dot',
      status: 'completed',
      outcome: 'success',
      project_path: '/tmp/project-one',
      ended_at: '2026-03-22T00:06:00Z',
    })
    const runsById = {
      [selectedRun.run_id]: selectedRun,
      [otherRun.run_id]: otherRun,
    }
    const latestEntries = [
      makeJournalEntry(
        4,
        {
          type: 'StageCompleted',
          sequence: 4,
          emitted_at: '2026-03-22T00:05:00Z',
          node_id: 'done',
          index: 3,
          source_scope: 'root',
        },
        {
          kind: 'stage',
          summary: 'Stage done completed',
        },
      ),
      makeJournalEntry(
        3,
        {
          type: 'StageCompleted',
          sequence: 3,
          emitted_at: '2026-03-22T00:04:00Z',
          node_id: 'plan_current',
          index: 2,
          source_scope: 'root',
        },
        {
          kind: 'stage',
          summary: 'Stage plan_current completed',
        },
      ),
    ]
    const olderEntries = [
      makeJournalEntry(
        2,
        {
          type: 'StageStarted',
          sequence: 2,
          emitted_at: '2026-03-22T00:03:00Z',
          node_id: 'plan_current',
          index: 2,
          source_scope: 'root',
        },
        {
          kind: 'stage',
          summary: 'Stage plan_current started',
        },
      ),
      makeJournalEntry(
        1,
        {
          type: 'StageCompleted',
          sequence: 1,
          emitted_at: '2026-03-22T00:02:00Z',
          node_id: 'prepare',
          index: 1,
          source_scope: 'root',
        },
        {
          kind: 'stage',
          summary: 'Stage prepare completed',
        },
      ),
    ]
    const journalRequestUrls: string[] = []

    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (method !== 'GET') {
        throw new Error(`Unhandled request: ${method} ${url}`)
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-one')) {
        return jsonResponse({ runs: [selectedRun, otherRun] })
      }
      const pipelineStatusMatch = url.match(/\/attractor\/pipelines\/([^/?#]+)$/)
      const pipelineStatusRunId = pipelineStatusMatch?.[1] ? decodeURIComponent(pipelineStatusMatch[1]) : null
      if (pipelineStatusRunId && pipelineStatusRunId in runsById) {
        const run = runsById[pipelineStatusRunId as keyof typeof runsById]
        return jsonResponse({
          pipeline_id: run.run_id,
          run_id: run.run_id,
          flow_name: run.flow_name,
          status: run.status,
          outcome: run.outcome,
          outcome_reason_code: null,
          outcome_reason_message: null,
          working_directory: run.working_directory,
          project_path: run.project_path,
          git_branch: run.git_branch,
          git_commit: run.git_commit,
          spec_id: null,
          plan_id: null,
          model: run.model,
          started_at: run.started_at,
          ended_at: run.ended_at,
          last_error: run.last_error ?? '',
          token_usage: run.token_usage,
          current_node: 'done',
          completed_nodes: ['prepare', 'done'],
          progress: {
            current_node: 'done',
            completed_nodes: ['prepare', 'done'],
          },
          continued_from_run_id: null,
          continued_from_node: null,
          continued_from_flow_mode: null,
          continued_from_flow_name: null,
        })
      }
      const pipelineMatch = url.match(/\/attractor\/pipelines\/([^/]+)\/([^/?#]+)/)
      const runId = pipelineMatch?.[1] ? decodeURIComponent(pipelineMatch[1]) : null
      const resource = pipelineMatch?.[2] ?? null
      if (runId && runId in runsById) {
        if (resource === 'checkpoint') {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              completed_nodes: ['prepare'],
              current_node: 'done',
            },
          })
        }
        if (resource === 'context') {
          return jsonResponse({
            pipeline_id: runId,
            context: {},
          })
        }
        if (resource === 'artifacts') {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (resource === 'graph-preview') {
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
        if (resource === 'questions') {
          return jsonResponse({
            pipeline_id: runId,
            questions: [],
          })
        }
        if (resource === 'journal') {
          const requestUrl = new URL(url, 'http://localhost')
          journalRequestUrls.push(requestUrl.toString())
          if (runId === selectedRun.run_id && requestUrl.searchParams.get('before_sequence') === '3') {
            return jsonResponse(makeJournalPage(runId, olderEntries, false))
          }
          return jsonResponse(makeJournalPage(
            runId,
            runId === selectedRun.run_id ? latestEntries : [],
            runId === selectedRun.run_id,
          ))
        }
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

    const selectedRunCard = screen.getByText('selected.dot').closest('[data-testid="run-history-row"]')
    expect(selectedRunCard).toBeTruthy()
    await user.click(selectedRunCard!)

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-panel')).toBeVisible()
      expect(screen.getByTestId('run-journal-load-older')).toBeVisible()
    })

    await user.click(screen.getByTestId('run-journal-load-older'))

    await waitFor(() => {
      expect(screen.queryByTestId('run-journal-load-older')).not.toBeInTheDocument()
      expect(screen.getByTestId('run-event-timeline-list')).toHaveTextContent('Stage prepare completed')
    })

    expect(useRunJournalStore.getState().byRunId[selectedRun.run_id]).toMatchObject({
      hasOlder: false,
      oldestSequence: 1,
      newestSequence: 4,
    })

    const otherRunCard = screen.getByText('other.dot').closest('[data-testid="run-history-row"]')
    expect(otherRunCard).toBeTruthy()
    await user.click(otherRunCard!)

    const reselectedRunCard = screen.getByText('selected.dot').closest('[data-testid="run-history-row"]')
    expect(reselectedRunCard).toBeTruthy()
    await user.click(reselectedRunCard!)

    await waitFor(() => {
      expect(
        journalRequestUrls.filter((requestUrl) => requestUrl.includes(`/attractor/pipelines/${selectedRun.run_id}/journal?limit=100`)).length,
      ).toBeGreaterThanOrEqual(2)
    })

    await waitFor(() => {
      expect(screen.queryByTestId('run-journal-load-older')).not.toBeInTheDocument()
    })

    expect(
      journalRequestUrls.filter((requestUrl) => requestUrl.includes(`/attractor/pipelines/${selectedRun.run_id}/journal?limit=100&before_sequence=3`)),
    ).toHaveLength(1)
    expect(
      flattenRunJournalSegments(useRunJournalStore.getState().byRunId[selectedRun.run_id]?.segments ?? [])
        .map(({ sequence }) => sequence),
    ).toEqual([4, 3, 2, 1])
    expect(useRunJournalStore.getState().byRunId[selectedRun.run_id]).toMatchObject({
      hasOlder: false,
      oldestSequence: 1,
      newestSequence: 4,
    })
  })
})
