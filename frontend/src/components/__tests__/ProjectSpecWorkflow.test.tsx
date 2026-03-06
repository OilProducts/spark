import { ProjectsPanel } from '@/components/ProjectsPanel'
import { useStore } from '@/store'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'

const resetProjectWorkflowState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'home',
    activeProjectPath: null,
    activeFlow: null,
    selectedRunId: null,
    workingDir: DEFAULT_WORKING_DIRECTORY,
    model: 'gpt-5.3',
    projectRegistry: {},
    projectScopedWorkspaces: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    logs: [],
  }))
}

const resolveRequestUrl = (input: RequestInfo | URL): string => {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.toString()
  return input.url
}

describe('Project-scoped workflow behavior', () => {
  let planStatusContractUnavailable = false

  beforeEach(() => {
    planStatusContractUnavailable = false
    resetProjectWorkflowState()
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/api/projects/metadata')) {
          return new Response(JSON.stringify({ branch: 'main', commit: 'abc123def456' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/api/flows/')) {
          return new Response(JSON.stringify({ content: 'digraph Plan { start -> end }' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.endsWith('/pipelines') && init?.method === 'POST') {
          return new Response(JSON.stringify({ status: 'started', pipeline_id: 'run-plan-101' }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.endsWith('/pipelines/run-plan-101')) {
          if (planStatusContractUnavailable) {
            return new Response(JSON.stringify({ detail: 'plan status endpoint unavailable' }), {
              status: 503,
              headers: { 'Content-Type': 'application/json' },
            })
          }
          return new Response(
            JSON.stringify({
              pipeline_id: 'run-plan-101',
              status: 'running',
              flow_name: 'plan-generation.dot',
              working_directory: '/tmp/plan-project',
              model: 'gpt-5.3',
            }),
            {
              status: 200,
              headers: { 'Content-Type': 'application/json' },
            },
          )
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('creates project-scoped conversation history and requires explicit confirmation before applying proposal edits', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/workflow-project')
      useStore.getState().setActiveProjectPath('/tmp/workflow-project')
    })

    render(<ProjectsPanel />)
    expect(screen.getByTestId('project-ai-conversation-surface')).toBeVisible()

    await user.type(
      screen.getByTestId('project-ai-conversation-input'),
      'Please add a project-first home chat flow with approval before planning.',
    )
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent(
      'Please add a project-first home chat flow with approval before planning.',
    )
    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toBeVisible()
    })
    expect(useStore.getState().projectScopedWorkspaces['/tmp/workflow-project']?.conversationId).toMatch(
      /^conversation-/,
    )

    const confirmSpy = vi.spyOn(window, 'confirm')
    confirmSpy.mockReturnValueOnce(false)
    await user.click(screen.getByTestId('project-spec-edit-proposal-apply-button'))
    expect(screen.getByTestId('project-spec-edit-proposal-preview')).toBeVisible()
    expect(useStore.getState().projectScopedWorkspaces['/tmp/workflow-project']?.specId).toBeNull()

    confirmSpy.mockReturnValueOnce(true)
    await user.click(screen.getByTestId('project-spec-edit-proposal-apply-button'))
    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toHaveTextContent('Applied')
    })

    const projectScope = useStore.getState().projectScopedWorkspaces['/tmp/workflow-project']
    expect(projectScope.specId).toMatch(/^spec-/)
    expect(projectScope.specStatus).toBe('approved')
    expect(projectScope.projectEventLog).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          message: expect.stringContaining('Applied spec edit proposal'),
        }),
      ]),
    )
    expect(projectScope.specProvenance).toEqual(
      expect.objectContaining({
        source: 'spec-edit-proposal',
        referenceId: expect.stringMatching(/^proposal-/),
        runId: null,
        gitBranch: 'main',
        gitCommit: 'abc123def456',
      }),
    )
    expect(projectScope.specProvenance?.capturedAt).toEqual(expect.any(String))
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Applied spec edit proposal')
    expect(screen.queryByTestId('project-spec-edit-proposal-apply-button')).not.toBeInTheDocument()
    expect(screen.getByTestId('project-spec-edit-proposal-preview')).toHaveTextContent(
      'This approved spec card now acts as the upstream artifact for the execution card below.',
    )
  })

  it('reconstructs an applied spec card from approved provenance when no pending proposal state remains', () => {
    act(() => {
      useStore.getState().registerProject('/tmp/persisted-spec-project')
      useStore.getState().setActiveProjectPath('/tmp/persisted-spec-project')
      useStore.setState((state) => ({
        ...state,
        projectScopedWorkspaces: {
          ...state.projectScopedWorkspaces,
          '/tmp/persisted-spec-project': {
            ...state.projectScopedWorkspaces['/tmp/persisted-spec-project'],
            conversationId: 'conversation-persisted-spec-project',
            conversationHistory: [
              {
                role: 'user',
                content: 'Refine the home chat and execution cards so tracker-ready work is clearer.',
                timestamp: '2026-03-05T14:00:00Z',
              },
              {
                role: 'assistant',
                content: 'I prepared a specification proposal for that refinement.',
                timestamp: '2026-03-05T14:01:00Z',
              },
            ],
            specId: 'spec-persisted-spec-project',
            specStatus: 'approved',
            specProvenance: {
              source: 'spec-edit-proposal',
              referenceId: 'proposal-persisted-spec-project',
              capturedAt: '2026-03-05T14:02:00Z',
              runId: null,
              gitBranch: 'main',
              gitCommit: 'abc123def456',
            },
          },
        },
      }))
    })

    render(<ProjectsPanel />)

    const specCard = screen.getByTestId('project-spec-edit-proposal-preview')
    expect(specCard).toBeVisible()
    expect(specCard).toHaveTextContent('Applied')
    expect(specCard).toHaveTextContent('proposal-persisted-spec-project')
    expect(specCard).toHaveTextContent('Refine the home chat and execution cards so tracker-ready work is clear')
    expect(screen.queryByTestId('project-spec-edit-proposal-apply-button')).not.toBeInTheDocument()
  })

  it('seeds a visible example spec card for sparkspawn when execution context exists without proposal provenance', () => {
    act(() => {
      useStore.getState().registerProject('/tmp/sparkspawn')
      useStore.getState().setActiveProjectPath('/tmp/sparkspawn')
      useStore.setState((state) => ({
        ...state,
        projectScopedWorkspaces: {
          ...state.projectScopedWorkspaces,
          '/tmp/sparkspawn': {
            ...state.projectScopedWorkspaces['/tmp/sparkspawn'],
            planId: 'plan-sparkspawn-demo',
            planStatus: 'draft',
            conversationHistory: [],
            specId: null,
            specStatus: 'draft',
            specProvenance: null,
          },
        },
      }))
    })

    render(<ProjectsPanel />)

    const specCard = screen.getByTestId('project-spec-edit-proposal-preview')
    expect(specCard).toBeVisible()
    expect(specCard).toHaveTextContent('Example')
    expect(specCard).toHaveTextContent('proposal-example-sparkspawn-home-chat')
    expect(specCard).toHaveTextContent('Seeded example for local UI evaluation in the sparkspawn project.')
    expect(screen.queryByTestId('project-spec-edit-proposal-apply-button')).not.toBeInTheDocument()
  })

  it('auto-launches plan workflow after spec proposal approval and records run context', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/plan-project')
      useStore.getState().setActiveProjectPath('/tmp/plan-project')
      useStore.getState().setActiveFlow('plan-generation.dot')
    })

    render(<ProjectsPanel />)
    expect(screen.queryByTestId('project-plan-generation-surface')).not.toBeInTheDocument()
    await user.type(
      screen.getByTestId('project-ai-conversation-input'),
      'Generate a plan for implementing project-home chat proposal approvals.',
    )
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toBeVisible()
    })
    await user.click(screen.getByTestId('project-spec-edit-proposal-apply-button'))
    await waitFor(() => {
      expect(useStore.getState().selectedRunId).toBe('run-plan-101')
    })

    const state = useStore.getState()
    const scope = state.projectScopedWorkspaces['/tmp/plan-project']
    expect(screen.getByTestId('project-plan-generation-surface')).toBeVisible()
    expect(scope.planId).toMatch(/^plan-/)
    expect(scope.planStatus).toBe('draft')
    expect(scope.planProvenance).toEqual(
      expect.objectContaining({
        source: 'plan-generation-workflow',
        referenceId: 'run-plan-101',
        capturedAt: expect.any(String),
        runId: 'run-plan-101',
        gitBranch: 'main',
        gitCommit: 'abc123def456',
      }),
    )
    expect(state.viewMode).toBe('home')
    expect(screen.getByTestId('project-plan-gate-surface')).toBeVisible()
  })

  it('[CID:12.4.03] keeps plan-generation launch active when status retrieval is degraded', async () => {
    planStatusContractUnavailable = true
    const user = userEvent.setup()
    act(() => {
      useStore.getState().registerProject('/tmp/plan-status-degraded-project')
      useStore.getState().setActiveProjectPath('/tmp/plan-status-degraded-project')
      useStore.getState().setActiveFlow('plan-generation.dot')
    })

    render(<ProjectsPanel />)
    await user.type(
      screen.getByTestId('project-ai-conversation-input'),
      'Draft plan steps and work items for home-first project workflow.',
    )
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toBeVisible()
    })
    await user.click(screen.getByTestId('project-spec-edit-proposal-apply-button'))

    await waitFor(() => {
      expect(useStore.getState().selectedRunId).toBe('run-plan-101')
    })

    expect(screen.getByTestId('project-plan-generation-status-degraded')).toHaveTextContent(
      'plan status endpoint unavailable',
    )
    expect(useStore.getState().projectScopedWorkspaces['/tmp/plan-status-degraded-project']?.planStatus).toBe('draft')
    expect(useStore.getState().viewMode).toBe('home')
  })
})
