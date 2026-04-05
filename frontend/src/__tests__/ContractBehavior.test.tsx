import App from '@/App'
import { HomeSessionController, RunsSessionController } from '@/app/AppSessionControllers'
import { ExecutionControls } from '@/features/execution/ExecutionControls'
import { ExecutionWorkspace } from '@/features/execution/ExecutionWorkspace'
import { Editor } from '@/features/editor/Editor'
import { GraphSettings } from '@/features/editor/GraphSettings'
import { Navbar } from '@/app/Navbar'
import { ProjectsPanel } from '@/features/projects/ProjectsPanel'
import { RunStream } from '@/features/runs/RunStream'
import { RunsPanel } from '@/features/runs/RunsPanel'
import { SettingsPanel } from '@/features/settings/SettingsPanel'
import { Sidebar } from '@/features/editor/Sidebar'
import { TaskNode } from '@/features/workflow-canvas/TaskNode'
import { ValidationPanel } from '@/features/editor/components/ValidationPanel'
import {
  ApiHttpError,
  ApiSchemaError,
  approveSpecEditProposalValidated,
  deleteConversationValidated,
  fetchConversationSnapshotValidated,
  fetchFlowListValidated,
  fetchFlowPayloadValidated,
  fetchPipelineAnswerValidated,
  fetchPipelineCancelValidated,
  fetchPipelineCheckpointValidated,
  fetchPipelineContextValidated,
  fetchPipelineGraphValidated,
  fetchPipelineQuestionsValidated,
  fetchPipelineStartValidated,
  fetchPipelineStatusValidated,
  fetchPreviewValidated,
  pickProjectDirectoryValidated,
  fetchWorkspaceFlowListValidated,
  fetchWorkspaceFlowRawValidated,
  fetchWorkspaceFlowValidated,
  rejectSpecEditProposalValidated,
  reviewExecutionCardValidated,
  updateWorkspaceFlowLaunchPolicyValidated,
  fetchRunsListValidated,
  sendConversationTurnValidated,
  fetchRuntimeStatusValidated,
  parseConversationSnapshotResponse,
  parseFlowListResponse,
  parseFlowPayloadResponse,
  parseWorkspaceFlowListResponse,
  parseWorkspaceFlowRawResponse,
  parseWorkspaceFlowResponse,
  parsePipelineGraphResponse,
  parseProjectDirectoryPickResponse,
  parsePipelineStatusResponse,
  parsePreviewResponse,
  parseRuntimeStatusResponse,
} from '@/lib/apiClient'
import { buildPipelineStartPayload } from '@/lib/pipelineStartPayload'
import { useStore } from '@/store'
import { ReactFlow, ReactFlowProvider, type Edge, type Node } from '@xyflow/react'
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'

const jsonResponse = (payload: unknown, init?: ResponseInit) =>
  new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
    ...init,
  })

const conversationSnapshot = <T extends Record<string, unknown>>(payload: T) => ({
  schema_version: 4,
  ...payload,
})

const requestUrl = (input: RequestInfo | URL): string => {
  if (typeof input === 'string') {
    return input
  }
  if (input instanceof URL) {
    return input.toString()
  }
  return input.url
}

const resetContractState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'editor',
    activeProjectPath: '/tmp/project-contract-behavior',
    activeFlow: 'contract-behavior.dot',
    executionFlow: null,
    selectedRunId: null,
    selectedRunRecord: null,
    selectedRunCompletedNodes: [],
    selectedRunStatusSync: 'idle',
    selectedRunStatusError: null,
    selectedRunStatusFetchedAtMs: null,
    runRecordOverrides: {},
    runsListSession: {
      ...state.runsListSession,
      scopeMode: 'active',
      selectedRunIdByScopeKey: {},
      status: 'idle',
      error: null,
      runs: [],
      lastFetchedAtMs: null,
      nowMs: Date.now(),
    },
    runDetailSessionsByRunId: {},
    workingDir: DEFAULT_WORKING_DIRECTORY,
    projectRegistry: {
      '/tmp/project-contract-behavior': {
        directoryPath: '/tmp/project-contract-behavior',
        isFavorite: false,
        lastAccessedAt: null,
      },
    },
    projectSessionsByPath: {
      '/tmp/project-contract-behavior': {
        workingDir: DEFAULT_WORKING_DIRECTORY,
        conversationId: null,
        specId: null,
        specStatus: 'draft',
        planId: null,
        planStatus: 'draft',
      },
    },
    projectRegistrationError: null,
    recentProjectPaths: ['/tmp/project-contract-behavior'],
    graphAttrs: {},
    graphAttrErrors: {},
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    editorGraphSettingsPanelOpenByFlow: {},
    editorShowAdvancedGraphAttrsByFlow: {},
    editorLaunchInputDraftsByFlow: {},
    editorLaunchInputDraftErrorByFlow: {},
    editorNodeInspectorSessionsByNodeId: {},
    saveState: 'idle',
    saveStateVersion: 0,
    saveErrorMessage: null,
    saveErrorKind: null,
    selectedNodeId: null,
    selectedEdgeId: null,
    uiDefaults: {
      llm_provider: 'openai',
      llm_model: 'gpt-5.3',
      reasoning_effort: 'high',
    },
  }))
}

const renderProjectsPanelWithController = () =>
  render(
    <>
      <HomeSessionController />
      <ProjectsPanel />
    </>,
  )

const renderRunsPanelWithController = () =>
  render(
    <>
      <RunsSessionController />
      <RunsPanel />
    </>,
  )

const renderWithFlowProvider = (node: ReactNode) => render(<ReactFlowProvider>{node}</ReactFlowProvider>)

const SidebarHarness = ({ nodes, edges }: { nodes: Node[]; edges: Edge[] }) => (
  <>
    <div style={{ width: 800, height: 600 }}>
      <ReactFlow nodes={nodes} edges={edges} fitView />
    </div>
    <Sidebar />
  </>
)

const renderSidebar = (nodes: Node[], edges: Edge[]) => renderWithFlowProvider(<SidebarHarness nodes={nodes} edges={edges} />)

const GraphSettingsHarness = ({ nodes, edges }: { nodes: Node[]; edges: Edge[] }) => (
  <>
    <div style={{ width: 800, height: 600 }}>
      <ReactFlow nodes={nodes} edges={edges} fitView />
    </div>
    <GraphSettings inline />
  </>
)

const renderGraphSettings = (nodes: Node[], edges: Edge[]) =>
  renderWithFlowProvider(<GraphSettingsHarness nodes={nodes} edges={edges} />)

const TaskNodeHarness = ({ nodes, edges = [] }: { nodes: Node[]; edges?: Edge[] }) => (
  <div style={{ width: 800, height: 600 }}>
    <ReactFlow nodes={nodes} edges={edges} nodeTypes={{ task: TaskNode }} fitView />
  </div>
)

const renderTaskNode = (node: Node) => renderWithFlowProvider(<TaskNodeHarness nodes={[node]} />)

const SidebarValidationHarness = ({ nodes, edges }: { nodes: Node[]; edges: Edge[] }) => (
  <>
    <div style={{ width: 800, height: 600 }}>
      <ReactFlow nodes={nodes} edges={edges} fitView />
    </div>
    <Sidebar />
    <ValidationPanel />
  </>
)

const renderSidebarWithValidation = (nodes: Node[], edges: Edge[]) =>
  renderWithFlowProvider(<SidebarValidationHarness nodes={nodes} edges={edges} />)

const setViewportWidth = (width: number) => {
  act(() => {
    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      writable: true,
      value: width,
    })
    window.dispatchEvent(new Event('resize'))
  })
}

describe('Frontend contract behavior', () => {
  const renderSelectedEdgeSidebar = () => {
    act(() => {
      useStore.getState().setSelectedNodeId(null)
      useStore.getState().setSelectedEdgeId('edge-start-task')
    })

    const nodes: Node[] = [
      { id: 'start', position: { x: 0, y: 0 }, data: { label: 'Start', shape: 'Mdiamond' } },
      { id: 'task', position: { x: 150, y: 0 }, data: { label: 'Task', shape: 'box' } },
    ]
    const edges: Edge[] = [
      {
        id: 'edge-start-task',
        source: 'start',
        target: 'task',
        data: {
          label: 'success',
          condition: 'outcome=success',
          weight: 7,
          fidelity: 'summary:low',
          thread_id: 'review-thread',
          loop_restart: true,
        },
      },
    ]

    renderSidebar(nodes, edges)
  }

  const renderManagerSidebarInspector = () => {
    act(() => {
      useStore.getState().setSelectedNodeId('manager')
      useStore.getState().setSelectedEdgeId(null)
      useStore.getState().setGraphAttrs({
        'stack.child_dotfile': 'child/flow.dot',
        'stack.child_workdir': '/tmp/child',
      })
    })

    const nodes: Node[] = [
      {
        id: 'manager',
        position: { x: 0, y: 0 },
        data: {
          label: 'Manager',
          shape: 'house',
          type: 'stack.manager_loop',
          'manager.poll_interval': '25ms',
          'manager.max_cycles': 3,
          'manager.stop_condition': 'child.outcome == "success"',
          'manager.actions': 'observe,steer',
        },
      },
    ]
    renderSidebar(nodes, [])
  }

  beforeEach(() => {
    resetContractState()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.endsWith('/attractor/api/flows')) {
          return new Response(JSON.stringify(['contract-behavior.dot']), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/api/flows/')) {
          return jsonResponse({
            name: 'contract-behavior.dot',
            content: 'digraph G { start [shape=Mdiamond]; done [shape=Msquare]; start -> done; }',
          })
        }
        if (url.includes('/graph-preview')) {
          return jsonResponse({
            status: 'ok',
            graph: {
              nodes: [
                { id: 'start', label: 'Start', shape: 'Mdiamond' },
                { id: 'done', label: 'Done', shape: 'Msquare' },
              ],
              edges: [{ source: 'start', target: 'done' }],
              graph_attrs: {},
              defaults: {},
            },
            diagnostics: [],
            errors: [],
          })
        }
        return jsonResponse({})
      }),
    )
  })

  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('[CID:12.1.02] provides typed endpoint adapters with runtime schema validation for required JSON responses', () => {
    expect(parseFlowListResponse(['a.dot', 'nested/b.dot'])).toEqual(['a.dot', 'nested/b.dot'])
    expect(() => parseFlowListResponse({})).toThrow(ApiSchemaError)
    expect(parseFlowPayloadResponse({ content: 'digraph G {}' })).toEqual({ name: '', content: 'digraph G {}' })
    expect(() => parseFlowPayloadResponse({})).toThrow(ApiSchemaError)
    expect(parseWorkspaceFlowListResponse([
      {
        name: 'test-planning.dot',
        title: 'Execution Planning',
        description: 'Turn an approved spec edit proposal into an execution plan.',
        launch_policy: null,
        effective_launch_policy: 'disabled',
      },
    ])).toMatchObject([
      {
        name: 'test-planning.dot',
        title: 'Execution Planning',
        description: 'Turn an approved spec edit proposal into an execution plan.',
        launch_policy: null,
        effective_launch_policy: 'disabled',
      },
    ])
    expect(() => parseWorkspaceFlowListResponse({})).toThrow(ApiSchemaError)
    expect(parseWorkspaceFlowResponse({
      name: 'test-planning.dot',
      title: 'Execution Planning',
      description: 'Turn an approved spec edit proposal into an execution plan.',
      launch_policy: 'trigger_only',
      effective_launch_policy: 'trigger_only',
    })).toMatchObject({
      name: 'test-planning.dot',
      launch_policy: 'trigger_only',
      effective_launch_policy: 'trigger_only',
    })
    expect(parseWorkspaceFlowRawResponse('digraph G {}')).toBe('digraph G {}')
    expect(() => parseWorkspaceFlowRawResponse({})).toThrow(ApiSchemaError)
    expect(parsePreviewResponse({ graph: { nodes: [], edges: [] } }).status).toBe('ok')
    expect(() => parsePreviewResponse({ graph: { nodes: {} } })).toThrow(ApiSchemaError)
    expect(parsePipelineStatusResponse({
      pipeline_id: 'run-1',
      run_id: 'run-1',
      status: 'completed',
      outcome: 'success',
      flow_name: 'contract-behavior.dot',
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      spec_id: 'spec-123',
      plan_id: 'plan-456',
      model: 'gpt-5.4',
      started_at: '2026-03-22T00:00:00Z',
      ended_at: '2026-03-22T00:05:00Z',
      last_error: '',
      token_usage: 1234,
      completed_nodes: ['start', 'done'],
      continued_from_run_id: 'run-parent',
      continued_from_node: 'Audit Milestone',
      continued_from_flow_mode: 'snapshot',
      continued_from_flow_name: 'implement-spec.dot',
    })).toMatchObject({
      pipeline_id: 'run-1',
      run_id: 'run-1',
      status: 'completed',
      project_path: '/tmp/project-contract-behavior',
      spec_id: 'spec-123',
      plan_id: 'plan-456',
      token_usage: 1234,
      completed_nodes: ['start', 'done'],
    })
    expect(() => parsePipelineStatusResponse({ status: 'running' })).toThrow(ApiSchemaError)
    expect(parseRuntimeStatusResponse({ status: 'idle' }).status).toBe('idle')
    expect(() => parseRuntimeStatusResponse({})).toThrow(ApiSchemaError)
    expect(parseConversationSnapshotResponse(conversationSnapshot({
      conversation_id: 'conversation-1',
      project_path: '/tmp/project',
      turns: [
        {
          id: 'turn-1',
          role: 'assistant',
          content: 'hello',
          timestamp: '2026-03-06T15:00:00Z',
          kind: 'message',
        },
      ],
      event_log: [
        {
          message: 'Execution planning started.',
          timestamp: '2026-03-06T15:01:00Z',
        },
      ],
      spec_edit_proposals: [],
      execution_cards: [],
      execution_workflow: {
        status: 'idle',
      },
    })).conversation_id).toBe('conversation-1')
    expect(() => parseConversationSnapshotResponse({ conversation_id: 'conversation-1' })).toThrow(ApiSchemaError)
    expect(parseProjectDirectoryPickResponse({ status: 'selected', directory_path: '/tmp/project' })).toEqual({
      status: 'selected',
      directory_path: '/tmp/project',
    })
    expect(parseProjectDirectoryPickResponse({ status: 'canceled' })).toEqual({ status: 'canceled' })
    expect(() => parseProjectDirectoryPickResponse({ status: 'selected' })).toThrow(ApiSchemaError)
    expect(parsePipelineGraphResponse('<svg></svg>')).toContain('<svg')
    expect(() => parsePipelineGraphResponse('')).toThrow(ApiSchemaError)
  })

  it('[CID:12.1.03] exercises endpoint adapters with happy paths and common error cases', async () => {
    type SuccessCase = {
      name: string
      invoke: () => Promise<unknown>
      expectedUrl: string
      expectedMethod?: string
      response: Response
      assertResult: (result: unknown) => void
      assertBody?: (init: RequestInit | undefined) => void
    }

    const successCases: SuccessCase[] = [
      {
        name: 'project directory picker',
        invoke: () => pickProjectDirectoryValidated(),
        expectedUrl: '/workspace/api/projects/pick-directory',
        expectedMethod: 'POST',
        response: jsonResponse({
          status: 'selected',
          directory_path: '/tmp/project one',
        }),
        assertResult: (result) =>
          expect(result).toEqual({
            status: 'selected',
            directory_path: '/tmp/project one',
          }),
      },
      {
        name: 'flow list',
        invoke: () => fetchFlowListValidated(),
        expectedUrl: '/attractor/api/flows',
        response: jsonResponse(['alpha.dot']),
        assertResult: (result) => expect(result).toEqual(['alpha.dot']),
      },
      {
        name: 'flow payload',
        invoke: () => fetchFlowPayloadValidated('alpha flow.dot'),
        expectedUrl: '/attractor/api/flows/alpha%20flow.dot',
        response: jsonResponse({ name: 'alpha flow.dot', content: 'digraph G {}' }),
        assertResult: (result) =>
          expect(result).toEqual({
            name: 'alpha flow.dot',
            content: 'digraph G {}',
          }),
      },
      {
        name: 'nested flow payload',
        invoke: () => fetchFlowPayloadValidated('alpha flow/review.dot'),
        expectedUrl: '/attractor/api/flows/alpha%20flow/review.dot',
        response: jsonResponse({ name: 'alpha flow/review.dot', content: 'digraph G {}' }),
        assertResult: (result) =>
          expect(result).toEqual({
            name: 'alpha flow/review.dot',
            content: 'digraph G {}',
          }),
      },
      {
        name: 'workspace flow list',
        invoke: () => fetchWorkspaceFlowListValidated(),
        expectedUrl: '/workspace/api/flows?surface=human',
        response: jsonResponse([
          {
            name: 'test-planning.dot',
            title: 'Execution Planning',
            description: 'Turn approved spec edits into execution plans.',
            launch_policy: null,
            effective_launch_policy: 'disabled',
          },
        ]),
        assertResult: (result) =>
          expect(result).toMatchObject([
            {
              name: 'test-planning.dot',
              title: 'Execution Planning',
              description: 'Turn approved spec edits into execution plans.',
              launch_policy: null,
              effective_launch_policy: 'disabled',
            },
          ]),
      },
      {
        name: 'workspace flow detail',
        invoke: () => fetchWorkspaceFlowValidated('alpha flow.dot'),
        expectedUrl: '/workspace/api/flows/alpha%20flow.dot?surface=human',
        response: jsonResponse({
          name: 'alpha flow.dot',
          title: 'Alpha Flow',
          description: 'Run the alpha workflow.',
          launch_policy: 'agent_requestable',
          effective_launch_policy: 'agent_requestable',
        }),
        assertResult: (result) =>
          expect(result).toMatchObject({
            name: 'alpha flow.dot',
            launch_policy: 'agent_requestable',
            effective_launch_policy: 'agent_requestable',
          }),
      },
      {
        name: 'nested workspace flow detail',
        invoke: () => fetchWorkspaceFlowValidated('alpha flow/review.dot'),
        expectedUrl: '/workspace/api/flows/alpha%20flow/review.dot?surface=human',
        response: jsonResponse({
          name: 'alpha flow/review.dot',
          title: 'Nested Alpha Flow',
          description: 'Run the nested alpha workflow.',
          launch_policy: 'agent_requestable',
          effective_launch_policy: 'agent_requestable',
        }),
        assertResult: (result) =>
          expect(result).toMatchObject({
            name: 'alpha flow/review.dot',
            launch_policy: 'agent_requestable',
            effective_launch_policy: 'agent_requestable',
          }),
      },
      {
        name: 'workspace flow raw',
        invoke: () => fetchWorkspaceFlowRawValidated('alpha flow.dot'),
        expectedUrl: '/workspace/api/flows/alpha%20flow.dot/raw?surface=human',
        response: new Response('digraph G {}', { status: 200 }),
        assertResult: (result) => expect(result).toBe('digraph G {}'),
      },
      {
        name: 'nested workspace flow raw',
        invoke: () => fetchWorkspaceFlowRawValidated('alpha flow/review.dot'),
        expectedUrl: '/workspace/api/flows/alpha%20flow/review.dot/raw?surface=human',
        response: new Response('digraph G {}', { status: 200 }),
        assertResult: (result) => expect(result).toBe('digraph G {}'),
      },
      {
        name: 'workspace flow launch policy update',
        invoke: () => updateWorkspaceFlowLaunchPolicyValidated('alpha flow.dot', 'agent_requestable'),
        expectedUrl: '/workspace/api/flows/alpha%20flow.dot/launch-policy',
        expectedMethod: 'PUT',
        response: jsonResponse({
          name: 'alpha flow.dot',
          title: 'Alpha Flow',
          description: 'Run the alpha workflow.',
          launch_policy: 'agent_requestable',
          effective_launch_policy: 'agent_requestable',
        }),
        assertResult: (result) =>
          expect(result).toMatchObject({
            name: 'alpha flow.dot',
            launch_policy: 'agent_requestable',
            effective_launch_policy: 'agent_requestable',
          }),
        assertBody: (init) => {
          expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
          expect(JSON.parse(String(init?.body))).toEqual({
            launch_policy: 'agent_requestable',
          })
        },
      },
      {
        name: 'nested workspace flow launch policy update',
        invoke: () => updateWorkspaceFlowLaunchPolicyValidated('alpha flow/review.dot', 'agent_requestable'),
        expectedUrl: '/workspace/api/flows/alpha%20flow/review.dot/launch-policy',
        expectedMethod: 'PUT',
        response: jsonResponse({
          name: 'alpha flow/review.dot',
          title: 'Nested Alpha Flow',
          description: 'Run the nested alpha workflow.',
          launch_policy: 'agent_requestable',
          effective_launch_policy: 'agent_requestable',
        }),
        assertResult: (result) =>
          expect(result).toMatchObject({
            name: 'alpha flow/review.dot',
            launch_policy: 'agent_requestable',
            effective_launch_policy: 'agent_requestable',
          }),
        assertBody: (init) => {
          expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
          expect(JSON.parse(String(init?.body))).toEqual({
            launch_policy: 'agent_requestable',
          })
        },
      },
      {
        name: 'conversation snapshot',
        invoke: () => fetchConversationSnapshotValidated('conversation 1', '/tmp/project one'),
        expectedUrl: '/workspace/api/conversations/conversation%201?project_path=%2Ftmp%2Fproject%20one',
        response: jsonResponse(conversationSnapshot({
          conversation_id: 'conversation 1',
          project_path: '/tmp/project one',
          turns: [],
          event_log: [],
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: { status: 'idle' },
        })),
        assertResult: (result) =>
          expect(result).toMatchObject({
            conversation_id: 'conversation 1',
            project_path: '/tmp/project one',
          }),
      },
      {
        name: 'conversation turn',
        invoke: () => sendConversationTurnValidated('conversation 1', {
          project_path: '/tmp/project one',
          message: 'Draft a spec edit proposal.',
          model: 'gpt-5.3',
        }),
        expectedUrl: '/workspace/api/conversations/conversation%201/turns',
        expectedMethod: 'POST',
        response: jsonResponse(conversationSnapshot({
          conversation_id: 'conversation 1',
          project_path: '/tmp/project one',
          turns: [],
          event_log: [],
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: { status: 'idle' },
        })),
        assertResult: (result) => expect(result).toMatchObject({ conversation_id: 'conversation 1' }),
        assertBody: (init) => {
          expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
          expect(JSON.parse(String(init?.body))).toEqual({
            project_path: '/tmp/project one',
            message: 'Draft a spec edit proposal.',
            model: 'gpt-5.3',
          })
        },
      },
      {
        name: 'conversation delete',
        invoke: () => deleteConversationValidated('conversation 1', '/tmp/project one'),
        expectedUrl: '/workspace/api/conversations/conversation%201?project_path=%2Ftmp%2Fproject%20one',
        expectedMethod: 'DELETE',
        response: jsonResponse({
          status: 'deleted',
          conversation_id: 'conversation 1',
          project_path: '/tmp/project one',
        }),
        assertResult: (result) => expect(result).toMatchObject({
          status: 'deleted',
          conversation_id: 'conversation 1',
          project_path: '/tmp/project one',
        }),
      },
      {
        name: 'spec approval',
        invoke: () => approveSpecEditProposalValidated('conversation 1', 'proposal 1', {
          project_path: '/tmp/project one',
          model: 'gpt-5.3',
          flow_source: 'contract-behavior.dot',
        }),
        expectedUrl: '/workspace/api/conversations/conversation%201/spec-edit-proposals/proposal%201/approve',
        expectedMethod: 'POST',
        response: jsonResponse(conversationSnapshot({
          conversation_id: 'conversation 1',
          project_path: '/tmp/project one',
          turns: [],
          event_log: [],
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: { status: 'running', run_id: 'workflow-1', flow_source: 'contract-behavior.dot' },
        })),
        assertResult: (result) => expect(result).toMatchObject({ conversation_id: 'conversation 1' }),
        assertBody: (init) => {
          expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
          expect(JSON.parse(String(init?.body))).toEqual({
            project_path: '/tmp/project one',
            model: 'gpt-5.3',
            flow_source: 'contract-behavior.dot',
          })
        },
      },
      {
        name: 'spec rejection',
        invoke: () => rejectSpecEditProposalValidated('conversation 1', 'proposal 1', {
          project_path: '/tmp/project one',
        }),
        expectedUrl: '/workspace/api/conversations/conversation%201/spec-edit-proposals/proposal%201/reject',
        expectedMethod: 'POST',
        response: jsonResponse(conversationSnapshot({
          conversation_id: 'conversation 1',
          project_path: '/tmp/project one',
          turns: [],
          event_log: [],
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: { status: 'idle' },
        })),
        assertResult: (result) => expect(result).toMatchObject({ conversation_id: 'conversation 1' }),
        assertBody: (init) => {
          expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
          expect(JSON.parse(String(init?.body))).toEqual({
            project_path: '/tmp/project one',
          })
        },
      },
      {
        name: 'execution review',
        invoke: () => reviewExecutionCardValidated('conversation 1', 'execution 1', {
          project_path: '/tmp/project one',
          disposition: 'revision_requested',
          message: 'Split frontend and backend work items.',
          model: 'gpt-5.3',
          flow_source: 'contract-behavior.dot',
        }),
        expectedUrl: '/workspace/api/conversations/conversation%201/execution-cards/execution%201/review',
        expectedMethod: 'POST',
        response: jsonResponse(conversationSnapshot({
          conversation_id: 'conversation 1',
          project_path: '/tmp/project one',
          turns: [],
          event_log: [],
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: { status: 'running', run_id: 'workflow-2', flow_source: 'contract-behavior.dot' },
        })),
        assertResult: (result) => expect(result).toMatchObject({ conversation_id: 'conversation 1' }),
        assertBody: (init) => {
          expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
          expect(JSON.parse(String(init?.body))).toEqual({
            project_path: '/tmp/project one',
            disposition: 'revision_requested',
            message: 'Split frontend and backend work items.',
            model: 'gpt-5.3',
            flow_source: 'contract-behavior.dot',
          })
        },
      },
      {
        name: 'preview',
        invoke: () => fetchPreviewValidated('digraph G {}'),
        expectedUrl: '/attractor/preview',
        expectedMethod: 'POST',
        response: jsonResponse({ status: 'ok', graph: { nodes: [], edges: [] } }),
        assertResult: (result) => expect(result).toMatchObject({ status: 'ok' }),
        assertBody: (init) => {
          expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
          expect(JSON.parse(String(init?.body))).toEqual({ flow_content: 'digraph G {}' })
        },
      },
      {
        name: 'pipeline start',
        invoke: () =>
          fetchPipelineStartValidated({
            flow_name: 'alpha.dot',
            flow_content: 'digraph G {}',
          }),
        expectedUrl: '/attractor/pipelines',
        expectedMethod: 'POST',
        response: jsonResponse({ status: 'accepted', pipeline_id: 'run-start', run_id: 'run-start' }),
        assertResult: (result) => expect(result).toMatchObject({ status: 'accepted', pipeline_id: 'run-start' }),
        assertBody: (init) => {
          expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
          expect(JSON.parse(String(init?.body))).toMatchObject({ flow_name: 'alpha.dot' })
        },
      },
      {
        name: 'pipeline status',
        invoke: () => fetchPipelineStatusValidated('run status'),
        expectedUrl: '/attractor/pipelines/run%20status',
        response: jsonResponse({
          pipeline_id: 'run status',
          run_id: 'run status',
          status: 'completed',
          outcome: 'success',
          flow_name: 'alpha.dot',
          working_directory: '/tmp/workspace',
          project_path: '/tmp/project',
          git_branch: 'main',
          git_commit: 'abc1234',
          spec_id: 'spec-1',
          plan_id: 'plan-1',
          model: 'gpt-5.4',
          started_at: '2026-03-22T00:00:00Z',
          ended_at: '2026-03-22T00:05:00Z',
          last_error: '',
          token_usage: 42,
          completed_nodes: ['start', 'done'],
          continued_from_run_id: 'run-parent',
          continued_from_node: 'Audit Milestone',
          continued_from_flow_mode: 'snapshot',
          continued_from_flow_name: 'implement-spec.dot',
        }),
        assertResult: (result) => expect(result).toMatchObject({
          pipeline_id: 'run status',
          run_id: 'run status',
          status: 'completed',
          project_path: '/tmp/project',
          spec_id: 'spec-1',
          plan_id: 'plan-1',
          token_usage: 42,
          completed_nodes: ['start', 'done'],
        }),
      },
      {
        name: 'pipeline cancel',
        invoke: () => fetchPipelineCancelValidated('run cancel'),
        expectedUrl: '/attractor/pipelines/run%20cancel/cancel',
        expectedMethod: 'POST',
        response: jsonResponse({ status: 'accepted', pipeline_id: 'run cancel' }),
        assertResult: (result) => expect(result).toMatchObject({ status: 'accepted', pipeline_id: 'run cancel' }),
      },
      {
        name: 'pipeline graph',
        invoke: () => fetchPipelineGraphValidated('run graph'),
        expectedUrl: '/attractor/pipelines/run%20graph/graph',
        response: new Response('<svg><g /></svg>', { status: 200 }),
        assertResult: (result) => expect(result).toContain('<svg>'),
      },
      {
        name: 'pipeline questions',
        invoke: () => fetchPipelineQuestionsValidated('run questions'),
        expectedUrl: '/attractor/pipelines/run%20questions/questions',
        response: jsonResponse({ questions: [{ question_id: 'q-1' }] }),
        assertResult: (result) => expect(result).toEqual({ questions: [{ question_id: 'q-1' }] }),
      },
      {
        name: 'pipeline answer',
        invoke: () => fetchPipelineAnswerValidated('run answer', 'q 1', 'approve'),
        expectedUrl: '/attractor/pipelines/run%20answer/questions/q%201/answer',
        expectedMethod: 'POST',
        response: jsonResponse({ status: 'accepted', pipeline_id: 'run answer', question_id: 'q 1' }),
        assertResult: (result) => expect(result).toMatchObject({ status: 'accepted', question_id: 'q 1' }),
        assertBody: (init) => {
          expect(init?.headers).toEqual({ 'Content-Type': 'application/json' })
          expect(JSON.parse(String(init?.body))).toEqual({
            question_id: 'q 1',
            selected_value: 'approve',
          })
        },
      },
      {
        name: 'pipeline checkpoint',
        invoke: () => fetchPipelineCheckpointValidated('run checkpoint'),
        expectedUrl: '/attractor/pipelines/run%20checkpoint/checkpoint',
        response: jsonResponse({ pipeline_id: 'run checkpoint', checkpoint: { node: 'n-1' } }),
        assertResult: (result) =>
          expect(result).toEqual({ pipeline_id: 'run checkpoint', checkpoint: { node: 'n-1' } }),
      },
      {
        name: 'pipeline context',
        invoke: () => fetchPipelineContextValidated('run context'),
        expectedUrl: '/attractor/pipelines/run%20context/context',
        response: jsonResponse({ pipeline_id: 'run context', context: { branch: 'main' } }),
        assertResult: (result) =>
          expect(result).toEqual({ pipeline_id: 'run context', context: { branch: 'main' } }),
      },
      {
        name: 'runs list',
        invoke: () => fetchRunsListValidated(),
        expectedUrl: '/attractor/runs',
        response: jsonResponse({ runs: [{ run_id: 'run-1', status: 'running', outcome: null }] }),
        assertResult: (result) => expect(result).toEqual({ runs: [{ run_id: 'run-1', status: 'running', outcome: null, outcome_reason_code: undefined, outcome_reason_message: undefined, flow_name: '', working_directory: '', model: '', started_at: '', project_path: undefined, git_branch: undefined, git_commit: undefined, spec_id: undefined, plan_id: undefined, ended_at: undefined, last_error: undefined, token_usage: undefined }] }),
      },
      {
        name: 'runtime status',
        invoke: () => fetchRuntimeStatusValidated(),
        expectedUrl: '/attractor/status',
        response: jsonResponse({ status: 'idle' }),
        assertResult: (result) =>
          expect(result).toEqual({
            status: 'idle',
            outcome: undefined,
            outcome_reason_code: undefined,
            outcome_reason_message: undefined,
            last_error: undefined,
            last_working_directory: undefined,
            last_model: undefined,
            last_completed_nodes: null,
            last_flow_name: undefined,
            last_run_id: undefined,
          }),
      },
    ]

    for (const testCase of successCases) {
      const fetchMock = vi.fn(async () => testCase.response)
      vi.stubGlobal('fetch', fetchMock)
      const result = await testCase.invoke()

      expect(fetchMock, `fetch call count for ${testCase.name}`).toHaveBeenCalledTimes(1)
      const [input, init] = fetchMock.mock.calls[0] as [RequestInfo | URL, RequestInit | undefined]
      expect(requestUrl(input), `request URL for ${testCase.name}`).toBe(testCase.expectedUrl)
      expect(init?.method ?? 'GET', `request method for ${testCase.name}`).toBe(testCase.expectedMethod ?? 'GET')
      testCase.assertBody?.(init)
      testCase.assertResult(result)
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        jsonResponse(
          {
            detail: { error: 'Preview validation failed.' },
          },
          { status: 422 },
        ),
      ),
    )
    await expect(fetchPreviewValidated('digraph G {}')).rejects.toMatchObject<ApiHttpError>({
      endpoint: '/attractor/preview',
      status: 422,
      detail: 'Preview validation failed.',
    })

    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('Gateway timeout', { status: 504 })),
    )
    await expect(fetchPipelineStatusValidated('run-504')).rejects.toMatchObject<ApiHttpError>({
      endpoint: '/attractor/pipelines/{id}',
      status: 504,
      detail: 'Gateway timeout',
    })

    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ flows: [] })),
    )
    await expect(fetchFlowListValidated()).rejects.toBeInstanceOf(ApiSchemaError)

    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('   ', { status: 200 })),
    )
    await expect(fetchPipelineGraphValidated('run-empty-graph')).rejects.toBeInstanceOf(ApiSchemaError)
  })

  it('[CID:12.2.01] shows degraded-state UX when runtime status endpoint responses are unavailable or incompatible', async () => {
    const runId = 'run-contract-degraded'
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.endsWith(`/attractor/pipelines/${encodeURIComponent(runId)}`)) {
          return jsonResponse({ runtime: 'idle' })
        }
        return jsonResponse({})
      }),
    )

    act(() => {
      useStore.getState().setSelectedRunId(runId)
    })

    render(<RunStream />)

    await waitFor(() => {
      expect(screen.getByTestId('runtime-api-degraded-banner')).toBeVisible()
    })

    expect(screen.getByTestId('runtime-api-degraded-banner')).toHaveTextContent(
      'Selected run status endpoint is unavailable or incompatible.',
    )
    expect(screen.getByTestId('global-save-state-indicator')).not.toHaveTextContent('Idle')
  })

  it('[CID:12.2.02] keeps non-dependent run-inspector surfaces functional under partial API failure', async () => {
    const runId = 'run-contract-partial-failure'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({ detail: 'backend unavailable' }, { status: 503 })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: {
              'graph.goal': 'Contract drift handling',
              'run.outcome': 'success',
            },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph-preview`)) {
          return jsonResponse({
            status: 'ok',
            graph: {
              graph_attrs: {
                label: 'Contract drift handling',
              },
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
        if (url.endsWith(`${runApiPath}/questions`)) {
          return jsonResponse({
            pipeline_id: runId,
            questions: [],
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        setTimeout(() => {
          this.onopen?.(new Event('open'))
        }, 0)
      }

      close() {}
      addEventListener() {}
      removeEventListener() {}
      dispatchEvent(): boolean {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.getState().setViewMode('runs')
      useStore.getState().setSelectedRunId(runId)
    })

    const user = userEvent.setup()
    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-checkpoint-error')).toBeVisible()
    })

    expect(screen.getByTestId('run-checkpoint-error')).toHaveTextContent('Unable to load checkpoint (HTTP 503).')
    expect(screen.getByTestId('run-context-panel')).toBeVisible()
    expect(screen.getByTestId('run-context-table')).toBeVisible()
    expect(screen.getByText('graph.goal')).toBeVisible()
    expect(screen.getByText('run.outcome')).toBeVisible()
    expect(screen.getByTestId('run-artifact-panel')).toBeVisible()
    await waitFor(() => {
      expect(screen.getByTestId('run-graph-panel')).toBeVisible()
    })
    expect(screen.queryByTestId('run-graph-canvas')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('run-graph-toggle-button'))

    await waitFor(() => {
      expect(screen.getByTestId('run-graph-canvas')).toBeVisible()
    })
    expect(screen.getByTestId('run-partial-api-failure-banner')).toHaveTextContent(
      'Some run detail endpoints are unavailable.',
    )
    expect(screen.getByTestId('run-context-refresh-button')).toBeEnabled()
    expect(screen.getByTestId('run-artifact-refresh-button')).toBeEnabled()
  })

  it('[CID:12.2.03] keeps save paths non-destructive when save response shape drifts', async () => {
    const initialDot = 'digraph contract_behavior { start [label="Start"]; }'
    const editedDot = 'digraph contract_behavior { start [label="Start"]; start -> end; end [label="End"]; }'
    const previewPayload = {
      status: 'ok',
      graph: {
        graph_attrs: {},
        defaults: {
          node: {},
          edge: {},
        },
        subgraphs: [],
        nodes: [
          {
            id: 'start',
            label: 'Start',
            shape: 'box',
          },
        ],
        edges: [],
      },
      diagnostics: [],
      errors: [],
    }
    let previewRequestCount = 0
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.endsWith('/attractor/api/flows/contract-behavior.dot')) {
        return jsonResponse({ content: initialDot })
      }
      if (url.endsWith('/attractor/preview')) {
        previewRequestCount += 1
        return jsonResponse(previewPayload)
      }
      if (url.endsWith('/attractor/api/flows') && init?.method === 'POST') {
        return jsonResponse({ saved: true })
      }
      return jsonResponse({}, { status: 404 })
    })
    vi.stubGlobal('fetch', fetchMock)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        suppressPreview: true,
      }))
    })

    const user = userEvent.setup()
    renderWithFlowProvider(<Editor />)

    await screen.findByTestId('editor-mode-toggle')
    await user.click(screen.getByRole('button', { name: 'Raw DOT' }))
    const rawEditor = await screen.findByTestId('raw-dot-editor')
    fireEvent.change(rawEditor, { target: { value: editedDot } })

    await user.click(screen.getByRole('button', { name: 'Structured' }))

    await waitFor(() => {
      expect(screen.getByTestId('raw-dot-editor')).toBeVisible()
    })
    expect((screen.getByTestId('raw-dot-editor') as HTMLTextAreaElement).value).toBe(editedDot)
    expect(screen.getByTestId('raw-dot-handoff-error')).toHaveTextContent(
      'Flow save failed before confirmation from backend.',
    )
    expect(screen.getByRole('button', { name: 'Structured' })).toBeEnabled()
    expect(previewRequestCount).toBe(1)
  })

  it('[CID:12.3.01] persists canonical active-project identity in UI client state', async () => {
    vi.resetModules()
    const storage = new Map<string, string>()
    const localStorageMock = {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        storage.set(key, value)
      },
      removeItem: (key: string) => {
        storage.delete(key)
      },
      clear: () => {
        storage.clear()
      },
    }
    vi.stubGlobal('localStorage', localStorageMock)
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      value: localStorageMock,
    })

    localStorageMock.setItem(
      'spark.project_registry_state',
      JSON.stringify({
        '/tmp/project-contract-behavior': {
          directoryPath: '/tmp/project-contract-behavior',
          isFavorite: false,
          lastAccessedAt: null,
        },
      }),
    )
    localStorageMock.setItem(
      'spark.ui_route_state',
      JSON.stringify({
        viewMode: 'projects',
        activeProjectPath: '/tmp/project-contract-behavior',
        selectedRunId: null,
      }),
    )

    const { useStore: restoredStore } = await import('@/store')
    restoredStore.getState().setActiveProjectPath('/tmp/project-contract-behavior/./')

    const nextState = restoredStore.getState()
    expect(nextState.activeProjectPath).toBe('/tmp/project-contract-behavior')
    expect(nextState.projectSessionsByPath['/tmp/project-contract-behavior']).toBeDefined()
    expect(nextState.projectSessionsByPath['/tmp/project-contract-behavior/./']).toBeUndefined()

    const persistedRouteStateRaw = localStorageMock.getItem('spark.ui_route_state')
    expect(persistedRouteStateRaw).toBeTruthy()
    const persistedRouteState = JSON.parse(String(persistedRouteStateRaw)) as { activeProjectPath: string | null; activeFlow?: string | null }
    expect(persistedRouteState.activeProjectPath).toBe('/tmp/project-contract-behavior')
    expect(persistedRouteState.activeFlow).toBeUndefined()
  })

  it('[CID:12.3.02] resolves execution payload working directory to concrete project-scoped path', () => {
    const relativeWorkingDirectoryPayload = buildPipelineStartPayload(
      {
        projectPath: '/tmp/project-contract-behavior',
        flowSource: 'contract-behavior.dot',
        workingDirectory: ' ./workspace/../build ',
        backend: 'codex-app-server',
        model: null,
        specArtifactId: null,
        planArtifactId: null,
      },
      'digraph G { start -> done }',
    )

    expect(relativeWorkingDirectoryPayload.working_directory).toBe('/tmp/project-contract-behavior/build')

    const blankWorkingDirectoryPayload = buildPipelineStartPayload(
      {
        projectPath: '/tmp/project-contract-behavior',
        flowSource: 'contract-behavior.dot',
        workingDirectory: '   ',
        backend: 'codex-app-server',
        model: null,
        specArtifactId: null,
        planArtifactId: null,
      },
      'digraph G { start -> done }',
    )

    expect(blankWorkingDirectoryPayload.working_directory).toBe('/tmp/project-contract-behavior')
  })

  it('[CID:12.3.03] retrieves conversation/spec/plan state by project identity', async () => {
    vi.resetModules()
    const { useStore: restoredStore } = await import('@/store')
    restoredStore.setState((state) => ({
      projectSessionsByPath: {
        ...state.projectSessionsByPath,
        '/tmp/project-a': {
          ...state.projectSessionsByPath['/tmp/project-a'],
          activeFlow: null,
          workingDir: '/tmp/project-a',
          conversationId: 'conversation-a',
          projectEventLog: [],
          specId: 'spec-a',
          specStatus: 'approved',
          specProvenance: null,
          planId: 'plan-a',
          planStatus: 'rejected',
          planProvenance: null,
        },
        '/tmp/project-b': {
          ...state.projectSessionsByPath['/tmp/project-b'],
          activeFlow: null,
          workingDir: '/tmp/project-b',
          conversationId: 'conversation-b',
          projectEventLog: [],
          specId: 'spec-b',
          specStatus: 'draft',
          specProvenance: null,
          planId: 'plan-b',
          planStatus: 'revision-requested',
          planProvenance: null,
        },
      },
    }))
    const projectAState = restoredStore.getState().getProjectSessionArtifactState('/tmp/project-a')
    expect(projectAState).toEqual({
      conversationId: 'conversation-a',
      specId: 'spec-a',
      specStatus: 'approved',
      planId: 'plan-a',
      planStatus: 'rejected',
    })

    const projectBState = restoredStore.getState().getProjectSessionArtifactState('/tmp/project-b/./')
    expect(projectBState).toEqual({
      conversationId: 'conversation-b',
      specId: 'spec-b',
      specStatus: 'draft',
      planId: 'plan-b',
      planStatus: 'revision-requested',
    })

    expect(restoredStore.getState().getProjectSessionArtifactState('/tmp/project-missing')).toBeNull()
  })

  it('[CID:12.4.03] routes execution-planning status updates to the workflow event log instead of chat history', async () => {
    const conversationSnapshots: Record<string, Record<string, unknown>> = {}

    class MockConversationEventSource {
      static instances: MockConversationEventSource[] = []

      url: string
      onmessage: ((event: MessageEvent<string>) => void) | null = null
      readyState = 1
      closed = false

      constructor(url: string) {
        this.url = url
        MockConversationEventSource.instances.push(this)
      }

      close() {
        this.closed = true
        this.readyState = 2
      }

      emit(payload: unknown) {
        this.onmessage?.({ data: JSON.stringify(payload) } as MessageEvent<string>)
      }
    }

    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      const endpoint = new URL(url, 'http://localhost')
      const conversationIdMatch = endpoint.pathname.match(/\/api\/conversations\/([^/]+)/)
      const conversationId = conversationIdMatch ? decodeURIComponent(conversationIdMatch[1]!) : null
      const requestBody = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : {}

      if (url.includes('/workspace/api/projects/metadata')) {
        return jsonResponse({
          name: 'project-contract-behavior',
          directory: '/tmp/project-contract-behavior',
          branch: 'main',
          commit: 'abc123def456',
        })
      }
      if (conversationId && endpoint.pathname === `/workspace/api/conversations/${encodeURIComponent(conversationId)}` && !init?.method) {
        const snapshot = conversationSnapshots[conversationId] ?? {
          conversation_id: conversationId,
          project_path: endpoint.searchParams.get('project_path') || '/tmp/project-contract-behavior',
          turns: [],
          event_log: [],
          spec_edit_proposals: [],
          execution_cards: [],
          execution_workflow: { status: 'idle' },
        }
        return jsonResponse(conversationSnapshot(snapshot))
      }
      if (conversationId && endpoint.pathname.endsWith('/turns') && init?.method === 'POST') {
        const snapshot = {
          conversation_id: conversationId,
          project_path: '/tmp/project-contract-behavior',
          turns: [
            {
              id: 'turn-user',
              role: 'user',
              content: String(requestBody.message || ''),
              timestamp: '2026-03-06T15:00:00Z',
              kind: 'message',
            },
            {
              id: 'turn-assistant',
              role: 'assistant',
              content: 'I drafted a spec edit proposal for review.',
              timestamp: '2026-03-06T15:00:10Z',
              kind: 'message',
            },
          ],
          segments: [
            {
              id: 'segment-proposal-contract',
              turn_id: 'turn-assistant',
              order: 1,
              kind: 'spec_edit_proposal',
              role: 'system',
              status: 'complete',
              timestamp: '2026-03-06T15:00:11Z',
              updated_at: '2026-03-06T15:00:11Z',
              completed_at: '2026-03-06T15:00:11Z',
              content: '',
              artifact_id: 'proposal-contract',
              error: null,
              tool_call: null,
              source: null,
            },
            {
              id: 'segment-assistant-contract',
              turn_id: 'turn-assistant',
              order: 2,
              kind: 'assistant_message',
              role: 'assistant',
              status: 'complete',
              timestamp: '2026-03-06T15:00:10Z',
              updated_at: '2026-03-06T15:00:10Z',
              completed_at: '2026-03-06T15:00:10Z',
              content: 'I drafted a spec edit proposal for review.',
              artifact_id: null,
              error: null,
              tool_call: null,
              source: null,
            },
          ],
          event_log: [
            {
              message: 'Drafted spec edit proposal proposal-contract.',
              timestamp: '2026-03-06T15:00:11Z',
            },
          ],
          spec_edit_proposals: [
            {
              id: 'proposal-contract',
              created_at: '2026-03-06T15:00:11Z',
              summary: 'Require explicit spec review before execution planning.',
              status: 'pending',
              changes: [
                {
                  path: 'spec/home-chat.md#review-flow',
                  before: 'Planning begins immediately.',
                  after: 'Planning begins only after spec approval.',
                },
              ],
            },
          ],
          execution_cards: [],
          execution_workflow: { status: 'idle' },
        }
        conversationSnapshots[conversationId] = snapshot
        return jsonResponse(conversationSnapshot(snapshot))
      }
      if (conversationId && endpoint.pathname.endsWith('/approve') && init?.method === 'POST') {
        const snapshot = {
          ...(conversationSnapshots[conversationId] as Record<string, unknown>),
          spec_edit_proposals: [
            {
              id: 'proposal-contract',
              created_at: '2026-03-06T15:00:11Z',
              summary: 'Require explicit spec review before execution planning.',
              status: 'applied',
              changes: [
                {
                  path: 'spec/home-chat.md#review-flow',
                  before: 'Planning begins immediately.',
                  after: 'Planning begins only after spec approval.',
                },
              ],
              canonical_spec_edit_id: 'spec-edit-project-contract-behavior-001',
              approved_at: '2026-03-06T15:01:00Z',
              git_branch: 'main',
              git_commit: 'abc123def456',
            },
          ],
          event_log: [
            {
              message: 'Drafted spec edit proposal proposal-contract.',
              timestamp: '2026-03-06T15:00:11Z',
            },
            {
              message: 'Approved spec edit proposal proposal-contract as canonical spec edit spec-edit-project-contract-behavior-001 and committed it to git.',
              timestamp: '2026-03-06T15:01:00Z',
            },
            {
              message: 'Execution planning started (workflow-contract-12-4-03) using contract-behavior.dot.',
              timestamp: '2026-03-06T15:01:01Z',
            },
          ],
          execution_workflow: {
            run_id: 'workflow-contract-12-4-03',
            status: 'running',
            error: null,
            flow_source: 'contract-behavior.dot',
          },
        }
        conversationSnapshots[conversationId] = snapshot
        return jsonResponse(conversationSnapshot(snapshot))
      }
      return jsonResponse({})
    })

    vi.stubGlobal('fetch', fetchMock)
    vi.stubGlobal('EventSource', MockConversationEventSource as unknown as typeof EventSource)
    vi.spyOn(window, 'confirm').mockReturnValue(true)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'projects',
        activeProjectPath: '/tmp/project-contract-behavior',
        activeFlow: 'contract-behavior.dot',
        projectSessionsByPath: {
          ...state.projectSessionsByPath,
          '/tmp/project-contract-behavior': {
            ...state.projectSessionsByPath['/tmp/project-contract-behavior'],
            specId: null,
            specStatus: 'draft',
          },
        },
      }))
    })

    const user = userEvent.setup()
    renderProjectsPanelWithController()

    await user.type(
      screen.getByTestId('project-ai-conversation-input'),
      'Draft a plan to implement project-home chat workflow approvals.',
    )
    await user.click(screen.getByTestId('project-ai-conversation-send-button'))
    await waitFor(() => {
      expect(screen.getByTestId('project-spec-edit-proposal-preview')).toBeVisible()
    })
    await user.click(screen.getByTestId('project-spec-edit-proposal-apply-button'))

    const conversationId = useStore.getState().projectSessionsByPath['/tmp/project-contract-behavior']?.conversationId
    expect(conversationId).toBeTruthy()

    const failureSnapshot = conversationSnapshot({
      ...(conversationSnapshots[conversationId!] as Record<string, unknown>),
      event_log: [
        ...(conversationSnapshots[conversationId!]?.event_log as Record<string, unknown>[]),
        {
          message: 'Execution planning failed: plan status endpoint unavailable.',
          timestamp: '2026-03-06T15:02:00Z',
        },
      ],
      execution_workflow: {
        run_id: 'workflow-contract-12-4-03',
        status: 'failed',
        error: 'plan status endpoint unavailable.',
        flow_source: 'contract-behavior.dot',
      },
    })
    conversationSnapshots[conversationId!] = failureSnapshot
    act(() => {
      MockConversationEventSource.instances
        .filter((instance) => !instance.closed && instance.url.includes(encodeURIComponent(conversationId!)))
        .forEach((instance) => {
          instance.emit({
            type: 'conversation_snapshot',
            state: failureSnapshot,
          })
        })
    })

    await waitFor(() => {
      expect(screen.getByTestId('project-event-log-list')).toHaveTextContent('Execution planning failed')
    })

    const requestedUrls = fetchMock.mock.calls.map(([input]) => requestUrl(input as RequestInfo | URL))
    expect(requestedUrls.some((url) => url.includes(`/workspace/api/conversations/${encodeURIComponent(conversationId!)}/spec-edit-proposals/proposal-contract/approve`))).toBe(true)
    expect(screen.getByTestId('project-ai-conversation-history-list')).not.toHaveTextContent('Execution planning failed')
    expect(useStore.getState().projectSessionsByPath['/tmp/project-contract-behavior']?.planStatus).toBe('draft')
    expect(useStore.getState().viewMode).toBe('projects')
  })

  it('[CID:12.4.04] integrates execution-card review contract with required revision feedback', async () => {
    const reviewBodies: Array<Record<string, unknown>> = []
    const conversationId = 'conversation-contract-review'
    const reviewSnapshot = conversationSnapshot({
      conversation_id: conversationId,
      project_path: '/tmp/project-contract-behavior',
      turns: [
        {
          id: 'turn-execution-card',
          role: 'assistant',
          content: '',
          timestamp: '2026-03-06T15:02:00Z',
          kind: 'execution_card',
          artifact_id: 'execution-card-contract-001',
        },
      ],
      segments: [
        {
          id: 'segment-execution-card-contract',
          turn_id: 'turn-execution-card',
          order: 1,
          kind: 'execution_card',
          role: 'system',
          status: 'complete',
          timestamp: '2026-03-06T15:02:00Z',
          updated_at: '2026-03-06T15:02:00Z',
          completed_at: '2026-03-06T15:02:00Z',
          content: '',
          artifact_id: 'execution-card-contract-001',
          error: null,
          tool_call: null,
          source: null,
        },
      ],
      event_log: [],
      spec_edit_proposals: [
        {
          id: 'proposal-contract',
          created_at: '2026-03-06T15:00:11Z',
          summary: 'Require explicit spec review before execution planning.',
          status: 'applied',
          changes: [],
          canonical_spec_edit_id: 'spec-edit-project-contract-behavior-001',
          approved_at: '2026-03-06T15:01:00Z',
          git_branch: 'main',
          git_commit: 'abc123def456',
        },
      ],
      execution_cards: [
        {
          id: 'execution-card-contract-001',
          title: 'Implement project-home chat approval workflow',
          summary: 'Turn the approved spec edit into tracker-ready work.',
          objective: 'Ship reviewed project chat and execution-card planning.',
          status: 'draft',
          source_spec_edit_id: 'spec-edit-project-contract-behavior-001',
          source_workflow_run_id: 'workflow-contract-12-4-04',
          created_at: '2026-03-06T15:02:00Z',
          updated_at: '2026-03-06T15:02:00Z',
          flow_source: 'contract-behavior.dot',
          work_items: [
            {
              id: 'WORK-1',
              title: 'Wire conversation snapshots',
              description: 'Replace local state with backend conversation snapshots.',
              acceptance_criteria: ['Chat renders backend turns.'],
              depends_on: [],
            },
          ],
          review_feedback: [],
        },
      ],
      execution_workflow: {
        run_id: 'workflow-contract-12-4-04',
        status: 'idle',
        error: null,
        flow_source: 'contract-behavior.dot',
      },
    })

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = requestUrl(input)
        if (url.includes('/workspace/api/projects/metadata')) {
          return jsonResponse({
            name: 'project-contract-behavior',
            directory: '/tmp/project-contract-behavior',
            branch: 'main',
            commit: 'abc123def456',
          })
        }
        if (url.includes(`/workspace/api/conversations/${encodeURIComponent(conversationId)}`) && !init?.method) {
          return jsonResponse(reviewSnapshot)
        }
        if (url.includes('/execution-cards/') && url.endsWith('/review') && init?.method === 'POST') {
          const body = JSON.parse(String(init.body)) as Record<string, unknown>
          reviewBodies.push(body)
          return jsonResponse(conversationSnapshot({
            ...reviewSnapshot,
            turns: [
              ...reviewSnapshot.turns,
              {
                id: 'turn-review-feedback',
                role: 'user',
                content: String(body.message || ''),
                timestamp: '2026-03-06T15:03:00Z',
                kind: 'message',
              },
            ],
            segments: reviewSnapshot.segments,
            execution_cards: [
              {
                ...reviewSnapshot.execution_cards[0],
                status: 'revision-requested',
                review_feedback: [
                  {
                    id: 'review-contract',
                    disposition: 'revision_requested',
                    message: String(body.message || ''),
                    created_at: '2026-03-06T15:03:00Z',
                    author: 'user',
                  },
                ],
              },
            ],
            execution_workflow: {
              run_id: 'workflow-contract-12-4-04-rerun',
              status: 'running',
              error: null,
              flow_source: 'contract-behavior.dot',
            },
            event_log: [
              {
                message: 'Requested revision for execution card execution-card-contract-001; regenerating with reviewer feedback.',
                timestamp: '2026-03-06T15:03:00Z',
              },
            ],
          }))
        }
        return jsonResponse({})
      }),
    )
    vi.spyOn(window, 'prompt').mockReturnValue('Split frontend and backend work items.')

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'projects',
        activeProjectPath: '/tmp/project-contract-behavior',
        activeFlow: 'contract-behavior.dot',
        projectSessionsByPath: {
          ...state.projectSessionsByPath,
          '/tmp/project-contract-behavior': {
            ...state.projectSessionsByPath['/tmp/project-contract-behavior'],
            conversationId,
          },
        },
      }))
    })

    const user = userEvent.setup()
    renderProjectsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('project-plan-gate-surface')).toBeVisible()
    })

    expect(screen.getByTestId('project-plan-approve-button')).toBeEnabled()
    expect(screen.getByTestId('project-plan-reject-button')).toBeEnabled()
    expect(screen.getByTestId('project-plan-request-revision-button')).toBeEnabled()

    await user.click(screen.getByTestId('project-plan-request-revision-button'))

    await waitFor(() => {
      expect(useStore.getState().projectSessionsByPath['/tmp/project-contract-behavior']?.planStatus).toBe('revision-requested')
    })

    expect(reviewBodies).toEqual([
      {
        project_path: '/tmp/project-contract-behavior',
        disposition: 'revision_requested',
        message: 'Split frontend and backend work items.',
        model: null,
      },
    ])
    expect(screen.getByTestId('project-ai-conversation-history-list')).toHaveTextContent('Split frontend and backend work items.')
    expect(screen.getByTestId('project-event-log-list')).toHaveTextContent('Requested revision for execution card')
  })

  it('[CID:12.4.05] integrates direct execution launch contract and error paths', async () => {
    const buildLaunchFailureMessage = 'build launch contract failure'
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.includes('/workspace/api/projects/metadata')) {
        return jsonResponse({
          name: 'project-contract-behavior',
          directory: '/tmp/project-contract-behavior',
          branch: 'main',
          commit: 'abc123def456',
        })
      }
      if (url.endsWith('/attractor/api/flows/contract-behavior.dot')) {
        return jsonResponse({ content: 'digraph BuildContract { start -> end }' })
      }
      if (url.endsWith('/attractor/pipelines') && init?.method === 'POST') {
        return jsonResponse({ detail: buildLaunchFailureMessage }, { status: 503 })
      }
      return jsonResponse({})
    })
    vi.stubGlobal('fetch', fetchMock)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'execution',
        activeProjectPath: '/tmp/project-contract-behavior',
        executionFlow: 'contract-behavior.dot',
        projectSessionsByPath: {
          ...state.projectSessionsByPath,
          '/tmp/project-contract-behavior': {
            ...state.projectSessionsByPath['/tmp/project-contract-behavior'],
            planId: 'plan-contract-behavior',
            planStatus: 'draft',
          },
        },
      }))
    })

    const user = userEvent.setup()
    render(<ExecutionControls />)

    await user.click(screen.getByTestId('execute-button'))

    await waitFor(() => {
      expect(screen.getByTestId('launch-failure-message')).toHaveTextContent(buildLaunchFailureMessage)
    })
    expect(screen.getByTestId('launch-failure-diagnostics')).toHaveTextContent(buildLaunchFailureMessage)
    expect(screen.getByTestId('launch-retry-button')).toBeEnabled()

    const pipelineCall = fetchMock.mock.calls.find(
      ([request, init]) => requestUrl(request as RequestInfo | URL).endsWith('/attractor/pipelines') && init?.method === 'POST',
    )
    expect(pipelineCall).toBeDefined()
    const pipelinePayload = JSON.parse((pipelineCall?.[1] as RequestInit).body as string) as {
      plan_id?: string | null
    }
    expect(pipelinePayload.plan_id).toBe('plan-contract-behavior')
  })

  it('[CID:13.1.01] supports keyboard navigation across projects, authoring, and execution mode tabs', async () => {
    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'projects',
      }))
    })

    const user = userEvent.setup()
    render(<Navbar />)

    const projectsTab = screen.getByTestId('nav-mode-projects')
    const editorTab = screen.getByTestId('nav-mode-editor')
    const executionTab = screen.getByTestId('nav-mode-execution')

    projectsTab.focus()
    expect(projectsTab).toHaveFocus()
    expect(useStore.getState().viewMode).toBe('projects')

    await user.keyboard('{ArrowRight}')
    expect(editorTab).toHaveFocus()
    expect(useStore.getState().viewMode).toBe('editor')

    await user.keyboard('{ArrowRight}')
    expect(executionTab).toHaveFocus()
    expect(useStore.getState().viewMode).toBe('execution')

    await user.keyboard('{ArrowLeft}')
    expect(editorTab).toHaveFocus()
    expect(useStore.getState().viewMode).toBe('editor')
  })

  it('[CID:13.1.02] provides semantic labels and focus-visible states across core interactive controls', () => {
    renderGraphSettings([], [])

    expect(screen.getByLabelText('Model')).toBeVisible()
    expect(screen.getByLabelText('Working Directory')).toBeVisible()
    expect(screen.getByLabelText('Goal')).toBeVisible()
    expect(screen.getByLabelText('Label')).toBeVisible()
    expect(screen.getByLabelText('Default Max Retries')).toBeVisible()
    expect(screen.getByLabelText('Default Fidelity')).toBeVisible()

    const advancedToggle = screen.getByTestId('graph-advanced-toggle')
    expect(advancedToggle.className).toContain('focus-visible')
    fireEvent.click(advancedToggle)

    expect(screen.getByLabelText('Model Stylesheet')).toBeVisible()
    expect(screen.getByLabelText('Retry Target')).toBeVisible()
    expect(screen.getByLabelText('Fallback Retry Target')).toBeVisible()
    expect(screen.getByLabelText('Stack Child Dotfile')).toBeVisible()
    expect(screen.getByLabelText('Stack Child Workdir')).toBeVisible()
    expect(screen.getByLabelText('Tool Hooks Pre')).toBeVisible()
    expect(screen.getByLabelText('Tool Hooks Post')).toBeVisible()
    expect(screen.getByLabelText('Default LLM Provider')).toBeVisible()
    expect(screen.getByLabelText('Default LLM Model')).toBeVisible()
    expect(screen.getByLabelText('Default Reasoning Effort')).toBeVisible()
    expect(screen.getByRole('button', { name: 'Apply To Nodes' }).className).toContain('focus-visible')
    expect(screen.getByRole('button', { name: 'Reset From Global' }).className).toContain('focus-visible')

    cleanup()
    render(<SettingsPanel />)

    expect(screen.getByLabelText('Default LLM Provider')).toBeVisible()
    expect(screen.getByLabelText('Default LLM Model')).toBeVisible()
    expect(screen.getByLabelText('Default Reasoning Effort')).toBeVisible()

    cleanup()
    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'execution',
        activeProjectPath: '/tmp/project-contract-behavior',
        executionFlow: 'contract-behavior.dot',
        selectedRunId: 'run-focus-audit',
        runtimeStatus: 'running',
      }))
    })

    render(<ExecutionControls />)

    expect(screen.getByTestId('execute-button').className).toContain('focus-visible')
    expect(screen.getByLabelText('Open in Runs after launch').className).toContain('focus-visible')

    cleanup()
    act(() => {
      resetContractState()
    })
    render(<Navbar />)

    expect(screen.getByTestId('top-nav-project-add-button').className).toContain('focus-visible')
    expect(screen.getByTestId('top-nav-project-clear-button').className).toContain('focus-visible')

    cleanup()
    act(() => {
      resetContractState()
      useStore.setState((state) => ({
        ...state,
        viewMode: 'home',
      }))
    })
    renderProjectsPanelWithController()

    expect(screen.getByTestId('project-thread-new-button').className).toContain('focus-visible')

    cleanup()
    act(() => {
      resetContractState()
      useStore.getState().setSelectedNodeId('task')
      useStore.getState().setSelectedEdgeId(null)
    })

    const nodes: Node[] = [
      {
        id: 'task',
        position: { x: 150, y: 0 },
        data: {
          label: 'Task',
          shape: 'box',
          prompt: 'Do work',
          x_node_extension: 'node-extra',
        },
      },
    ]
    renderSidebar(nodes, [])

    const nodeEditor = screen.getByTestId('node-extension-attrs-editor')
    expect(within(nodeEditor).getByLabelText('Key')).toBeVisible()
    expect(within(nodeEditor).getByLabelText('Value').className).toContain('focus-visible')
    expect(within(nodeEditor).getByLabelText('New Key').className).toContain('focus-visible')
    expect(within(nodeEditor).getByLabelText('New Value').className).toContain('focus-visible')
    expect(within(nodeEditor).getByRole('button', { name: 'Remove' }).className).toContain('focus-visible')
    expect(within(nodeEditor).getByRole('button', { name: 'Add Attribute' }).className).toContain('focus-visible')
  })

  it('[CID:13.2.01] applies narrow-viewport responsive layouts for inspector, diagnostics, and run timeline surfaces', async () => {
    const originalViewportWidth = window.innerWidth
    setViewportWidth(760)
    try {
      act(() => {
        useStore.getState().setViewMode('editor')
        useStore.getState().setDiagnostics([
          {
            rule_id: 'node_prompt_required',
            severity: 'warning',
            message: 'Prompt is recommended for codergen nodes.',
            node_id: 'task',
          },
        ])
      })

      const nodes: Node[] = [
        {
          id: 'task',
          position: { x: 150, y: 0 },
          data: {
            label: 'Task',
            shape: 'box',
            prompt: '',
          },
        },
      ]
      renderSidebarWithValidation(nodes, [])

      expect(screen.getByTestId('inspector-panel')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('validation-panel')).toHaveAttribute('data-responsive-layout', 'stacked')

      cleanup()
      const runId = 'run-responsive-contract'
      const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
      const runRecord = {
        run_id: runId,
        flow_name: 'contract-behavior.dot',
        status: 'running',
        outcome: null,
        working_directory: '/tmp/project-contract-behavior/workspace',
        project_path: '/tmp/project-contract-behavior',
        git_branch: 'main',
        git_commit: 'abc1234',
        model: 'gpt-5',
        started_at: '2026-03-04T01:00:00Z',
        ended_at: null,
        last_error: '',
        token_usage: 0,
      }

      vi.stubGlobal(
        'fetch',
        vi.fn(async (input: RequestInfo | URL) => {
          const url = requestUrl(input)
          if (url.includes('/attractor/runs')) {
            return jsonResponse({ runs: [runRecord] })
          }
          if (url.endsWith(`${runApiPath}/checkpoint`)) {
            return jsonResponse({ pipeline_id: runId, checkpoint: { node_statuses: {} } })
          }
          if (url.endsWith(`${runApiPath}/context`)) {
            return jsonResponse({ pipeline_id: runId, context: {} })
          }
          if (url.endsWith(`${runApiPath}/artifacts`)) {
            return jsonResponse({ pipeline_id: runId, artifacts: [] })
          }
          if (url.endsWith(`${runApiPath}/graph`)) {
            return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
              status: 200,
              headers: { 'Content-Type': 'image/svg+xml' },
            })
          }
          if (url.endsWith(`${runApiPath}/questions`)) {
            return jsonResponse({ pipeline_id: runId, questions: [] })
          }
          return jsonResponse({}, { status: 404 })
        }),
      )

      class MockEventSource {
        url: string
        withCredentials = false
        readyState = 1
        onopen: ((event: Event) => void) | null = null
        onmessage: ((event: MessageEvent) => void) | null = null
        onerror: ((event: Event) => void) | null = null

        constructor(url: string) {
          this.url = url
        }

        close() {}
        addEventListener() {}
        removeEventListener() {}
        dispatchEvent(): boolean {
          return false
        }
      }
      vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

      act(() => {
        useStore.getState().setViewMode('runs')
        useStore.getState().setSelectedRunId(runId)
      })
      renderRunsPanelWithController()

      await waitFor(() => {
        expect(screen.getByTestId('run-event-timeline-panel')).toBeVisible()
      })
      expect(screen.getByTestId('run-event-timeline-panel')).toHaveAttribute('data-responsive-layout', 'stacked')
    } finally {
      act(() => {
        setViewportWidth(originalViewportWidth)
      })
    }
  })

  it('[CID:13.2.02] keeps core project and operational tasks usable in narrow viewport layouts', () => {
    const originalViewportWidth = window.innerWidth
    setViewportWidth(760)
    try {
      act(() => {
        useStore.setState((state) => ({
          ...state,
          viewMode: 'projects',
          activeProjectPath: '/tmp/project-contract-behavior',
        }))
      })
      renderProjectsPanelWithController()

      expect(screen.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('project-thread-list')).toBeVisible()
      expect(screen.getByTestId('project-thread-new-button')).toBeVisible()
      expect(screen.queryByTestId('home-sidebar-resize-handle')).not.toBeInTheDocument()

      cleanup()
      act(() => {
        resetContractState()
        useStore.setState((state) => ({
          ...state,
          viewMode: 'execution',
          selectedRunId: 'run-mobile-ops',
          runtimeStatus: 'running',
        }))
      })
      render(<ExecutionWorkspace />)

      expect(screen.getByTestId('execution-workspace')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('execution-flow-panel')).toBeVisible()
      expect(screen.getByTestId('execution-launch-panel')).toBeVisible()

      cleanup()
      act(() => {
        resetContractState()
        useStore.setState((state) => ({
          ...state,
          viewMode: 'projects',
        }))
      })
      render(<Navbar />)

      expect(screen.getByTestId('top-nav')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('view-mode-tabs')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('top-nav-active-project')).toBeVisible()
      expect(screen.queryByTestId('top-nav-active-flow')).not.toBeInTheDocument()
      expect(screen.queryByTestId('top-nav-run-context')).not.toBeInTheDocument()
    } finally {
      setViewportWidth(originalViewportWidth)
    }
  })

  it('[CID:13.2.03] preserves expected desktop and narrow breakpoint layouts for core navigation and operations', () => {
    const originalViewportWidth = window.innerWidth
    try {
      setViewportWidth(1280)
      act(() => {
        resetContractState()
        useStore.setState((state) => ({
          ...state,
          viewMode: 'projects',
        }))
      })
      render(<Navbar />)
      expect(screen.getByTestId('top-nav')).toHaveAttribute('data-responsive-layout', 'inline')
      expect(screen.getByTestId('view-mode-tabs')).toHaveAttribute('data-responsive-layout', 'inline')

      cleanup()
      act(() => {
        resetContractState()
        useStore.setState((state) => ({
          ...state,
          viewMode: 'projects',
        }))
      })
      renderProjectsPanelWithController()
      expect(screen.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'split')
      expect(screen.getByTestId('project-thread-list')).toBeVisible()
      expect(screen.getByTestId('project-thread-new-button')).toBeVisible()
      expect(screen.getByTestId('home-sidebar-resize-handle')).toBeVisible()

      cleanup()
      setViewportWidth(760)
      act(() => {
        resetContractState()
        useStore.setState((state) => ({
          ...state,
          viewMode: 'projects',
        }))
      })
      render(<Navbar />)
      expect(screen.getByTestId('top-nav')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('view-mode-tabs')).toHaveAttribute('data-responsive-layout', 'stacked')

      cleanup()
      act(() => {
        resetContractState()
        useStore.setState((state) => ({
          ...state,
          viewMode: 'projects',
        }))
      })
      renderProjectsPanelWithController()
      expect(screen.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('project-thread-list')).toBeVisible()
      expect(screen.getByTestId('project-thread-new-button')).toBeVisible()
      expect(screen.queryByTestId('home-sidebar-resize-handle')).not.toBeInTheDocument()

      cleanup()
      setViewportWidth(1280)
      act(() => {
        resetContractState()
        useStore.setState((state) => ({
          ...state,
          viewMode: 'execution',
          selectedRunId: 'run-viewport-regression-desktop',
          runtimeStatus: 'running',
        }))
      })
      render(<ExecutionWorkspace />)
      expect(screen.getByTestId('execution-workspace')).toHaveAttribute('data-responsive-layout', 'split')

      cleanup()
      setViewportWidth(760)
      act(() => {
        resetContractState()
        useStore.setState((state) => ({
          ...state,
          viewMode: 'execution',
          selectedRunId: 'run-viewport-regression-mobile',
          runtimeStatus: 'running',
        }))
      })
      render(<ExecutionWorkspace />)
      expect(screen.getByTestId('execution-workspace')).toHaveAttribute('data-responsive-layout', 'stacked')
    } finally {
      setViewportWidth(originalViewportWidth)
    }
  })

  it('[CID:13.3.01] defines explicit performance budgets for canvas interaction and timeline updates', async () => {
    const runId = 'run-performance-budget-contract'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-05T00:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.endsWith('/attractor/api/flows/contract-behavior.dot')) {
          return jsonResponse({
            name: 'contract-behavior.dot',
            content: 'digraph G { start [label="Start"]; end [label="End"]; start -> end; }',
          })
        }
        if (url.endsWith('/attractor/preview')) {
          return jsonResponse({
            status: 'ok',
            graph: {
              nodes: [
                { id: 'start', attrs: { label: 'Start' } },
                { id: 'end', attrs: { label: 'End' } },
              ],
              edges: [{ source: 'start', target: 'end', attrs: {} }],
              graph_attrs: {},
            },
            diagnostics: [],
          })
        }
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({ pipeline_id: runId, checkpoint: { node_statuses: {} } })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({ pipeline_id: runId, context: {} })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({ pipeline_id: runId, artifacts: [] })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        if (url.endsWith(`${runApiPath}/questions`)) {
          return jsonResponse({ pipeline_id: runId, questions: [] })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
      }

      close() {}
      addEventListener() {}
      removeEventListener() {}
      dispatchEvent(): boolean {
        return false
      }
    }
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      resetContractState()
      useStore.setState((state) => ({
        ...state,
        viewMode: 'editor',
        activeFlow: 'contract-behavior.dot',
      }))
    })

    renderWithFlowProvider(<Editor />)

    const canvasBudget = await screen.findByTestId('canvas-interaction-performance-budget')
    expect(canvasBudget).toHaveAttribute('data-budget-ms', '16')
    expect(canvasBudget).toHaveTextContent('16ms')

    cleanup()

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-panel')).toBeVisible()
    })
    const timelineBudget = screen.getByTestId('timeline-update-performance-budget')
    expect(timelineBudget).toHaveAttribute('data-budget-ms', '50')
    expect(timelineBudget).toHaveTextContent('50ms')
  })

  it('[CID:13.3.02] profiles medium graphs and enables canvas optimizations', async () => {
    const nodeCount = 30
    const nodes = Array.from({ length: nodeCount }, (_, index) => ({
      id: `node_${index}`,
      attrs: { label: `Node ${index}` },
    }))
    const edges = nodes.slice(0, -1).map((node, index) => ({
      source: node.id,
      target: nodes[index + 1].id,
      attrs: {},
    }))

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.endsWith('/attractor/api/flows/contract-behavior.dot')) {
          return jsonResponse({
            name: 'contract-behavior.dot',
            content: 'digraph G { start -> end; }',
          })
        }
        if (url.endsWith('/attractor/preview')) {
          return jsonResponse({
            status: 'ok',
            graph: {
              nodes,
              edges,
              graph_attrs: {},
            },
            diagnostics: [],
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    act(() => {
      resetContractState()
      useStore.setState((state) => ({
        ...state,
        viewMode: 'editor',
        activeFlow: 'contract-behavior.dot',
      }))
    })

    renderWithFlowProvider(<Editor />)

    const profile = await screen.findByTestId('canvas-performance-profile')
    await waitFor(() => {
    expect(profile).toHaveAttribute('data-profile', 'medium')
    })
    expect(profile).toHaveAttribute('data-node-count', String(nodeCount))
    expect(profile).toHaveAttribute('data-only-render-visible-elements', 'true')
    expect(profile).toHaveAttribute('data-preview-ms')
    const previewMs = Number(profile.getAttribute('data-preview-ms'))
    expect(previewMs).not.toBeNaN()
    const previewDebounceMs = Number(profile.getAttribute('data-preview-debounce-ms'))
    expect(previewDebounceMs).toBeGreaterThan(300)
    const layoutMs = Number(profile.getAttribute('data-layout-ms'))
    expect(layoutMs).not.toBeNaN()
    expect(profile).toHaveAttribute('data-optimizations', 'visible-only, debounced-preview')
    expect(profile).toHaveTextContent('Optimizations:')
    expect(profile).toHaveTextContent('visible-only')
    expect(profile).toHaveTextContent('debounced-preview')
  })

  it('[CID:13.3.03] caps timeline entries and surfaces trimming under sustained SSE throughput', async () => {
    const runId = 'run-timeline-throughput-contract'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const maxItems = 200
    const totalEvents = maxItems + 35
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-05T04:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({ pipeline_id: runId, checkpoint: { node_statuses: {} } })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({ pipeline_id: runId, context: {} })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({ pipeline_id: runId, artifacts: [] })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        if (url.endsWith(`${runApiPath}/questions`)) {
          return jsonResponse({ pipeline_id: runId, questions: [] })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    let eventSource: MockEventSource | null = null
    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        // eslint-disable-next-line @typescript-eslint/no-this-alias
        eventSource = this
        setTimeout(() => {
          this.onopen?.(new Event('open'))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }
    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      resetContractState()
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-panel')).toBeVisible()
    })
    await waitFor(() => {
      expect(eventSource?.onmessage).toBeTruthy()
    })

    act(() => {
      for (let index = 0; index < totalEvents; index += 1) {
        eventSource?.onmessage?.(new MessageEvent('message', {
          data: JSON.stringify({
            type: 'StageStarted',
            node_id: `stage_${index}`,
            index,
          }),
        }))
      }
    })

    await waitFor(() => {
      expect(screen.getAllByTestId('run-event-timeline-row')).toHaveLength(maxItems)
    })

    const timelineRows = screen.getAllByTestId('run-event-timeline-row')
    expect(timelineRows[0]).toHaveTextContent(`stage_${totalEvents - 1}`)

    const throughputNotice = screen.getByTestId('run-event-timeline-throughput')
    expect(throughputNotice).toHaveAttribute('data-max-items', String(maxItems))
    expect(throughputNotice).toHaveAttribute('data-dropped-count', String(totalEvents - maxItems))
    expect(throughputNotice).toHaveTextContent(`Showing latest ${maxItems} events`)
  })

  it('[CID:14.0.01] propagates navbar project context through Home, Execution, Triggers, and Runs', async () => {
    const buildProjectRecord = (projectPath: string) => ({
      project_id: projectPath.split('/').filter(Boolean).join('-'),
      project_path: projectPath,
      display_name: projectPath.split('/').filter(Boolean).at(-1) ?? projectPath,
      created_at: '2026-03-25T12:00:00Z',
      last_opened_at: '2026-03-25T12:00:00Z',
      last_accessed_at: null,
      is_favorite: false,
      active_conversation_id: null,
    })
    const buildConversationSummary = ({
      conversationId,
      projectPath,
      title,
    }: {
      conversationId: string
      projectPath: string
      title: string
    }) => ({
      conversation_id: conversationId,
      conversation_handle: null,
      project_path: projectPath,
      title,
      created_at: '2026-03-25T12:00:00Z',
      updated_at: '2026-03-25T12:05:00Z',
      last_message_preview: null,
    })
    const buildRunRecord = ({
      flowName,
      projectPath,
      runId,
    }: {
      flowName: string
      projectPath: string
      runId: string
    }) => ({
      run_id: runId,
      flow_name: flowName,
      status: 'completed',
      outcome: 'success',
      outcome_reason_code: null,
      outcome_reason_message: null,
      working_directory: projectPath,
      project_path: projectPath,
      git_branch: 'main',
      git_commit: 'abcdef0',
      spec_id: null,
      plan_id: null,
      model: 'gpt-5.3-codex-spark',
      started_at: '2026-03-25T12:00:00Z',
      ended_at: '2026-03-25T12:05:00Z',
      last_error: null,
      token_usage: 1234,
    })
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      const method = init?.method ?? 'GET'
      if (url.includes('/workspace/api/projects/metadata')) {
        return jsonResponse({ branch: 'main', commit: 'abcdef0' })
      }
      if (url.includes('/workspace/api/projects/state')) {
        const payload = init?.body ? JSON.parse(String(init.body)) as { project_path?: string } : {}
        return jsonResponse(buildProjectRecord(payload.project_path ?? '/tmp/project-alpha'))
      }
      if (url.includes('/workspace/api/projects/conversations')) {
        const projectPath = new URL(url, 'http://localhost').searchParams.get('project_path')
        if (projectPath === '/tmp/project-alpha') {
          return jsonResponse([
            buildConversationSummary({
              conversationId: 'conversation-alpha',
              projectPath: '/tmp/project-alpha',
              title: 'Alpha thread',
            }),
          ])
        }
        if (projectPath === '/tmp/project-beta') {
          return jsonResponse([
            buildConversationSummary({
              conversationId: 'conversation-beta',
              projectPath: '/tmp/project-beta',
              title: 'Beta thread',
            }),
          ])
        }
        return jsonResponse([])
      }
      if (url.includes('/workspace/api/conversations/') && method === 'GET') {
        return jsonResponse({ detail: 'Unknown conversation' }, { status: 404 })
      }
      if (url.endsWith('/workspace/api/triggers') && method === 'GET') {
        return jsonResponse([])
      }
      if (url.includes('/workspace/api/projects')) {
        return jsonResponse([])
      }
      if (url.includes('/attractor/status')) {
        return jsonResponse({ status: 'idle', last_run_id: null })
      }
      if (url.includes('/attractor/api/flows')) {
        return jsonResponse([])
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-alpha')) {
        return jsonResponse({
          runs: [buildRunRecord({ flowName: 'alpha.dot', projectPath: '/tmp/project-alpha', runId: 'run-alpha' })],
        })
      }
      if (url.includes('/attractor/runs?project_path=%2Ftmp%2Fproject-beta')) {
        return jsonResponse({
          runs: [buildRunRecord({ flowName: 'beta.dot', projectPath: '/tmp/project-beta', runId: 'run-beta' })],
        })
      }
      if (url.includes('/attractor/runs')) {
        return jsonResponse({ runs: [] })
      }
      return jsonResponse({})
    })
    vi.stubGlobal('fetch', fetchMock)
    const user = userEvent.setup()

    act(() => {
      resetContractState()
      useStore.setState((state) => ({
        ...state,
        viewMode: 'home',
        activeProjectPath: null,
        projectRegistry: {},
        projectSessionsByPath: {},
        recentProjectPaths: [],
      }))
      useStore.getState().registerProject('/tmp/project-alpha')
      useStore.getState().registerProject('/tmp/project-beta')
      useStore.getState().setActiveProjectPath('/tmp/project-alpha')
      useStore.getState().setViewMode('home')
    })

    render(<App />)

    await waitFor(() => {
      expect(screen.getByText('Alpha thread')).toBeVisible()
    })
    await user.click(screen.getByTestId('top-nav-project-switcher'))
    await user.click(await screen.findByText('project-beta'))

    await waitFor(() => {
      expect(screen.getByText('Beta thread')).toBeVisible()
    })
    expect(screen.queryByText('Alpha thread')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('nav-mode-execution'))
    expect(await screen.findByTestId('execution-project-context-chip')).toHaveTextContent('project-beta')
    expect(screen.getByTestId('execution-no-flow-state')).toBeVisible()

    await user.click(screen.getByTestId('nav-mode-triggers'))
    expect(await screen.findByTestId('triggers-project-context-chip')).toHaveTextContent('project-beta')
    expect(screen.getByLabelText('Execution Target')).toHaveValue('active')
    expect(screen.getByText('Uses the current active project: /tmp/project-beta')).toBeVisible()

    await user.click(screen.getByTestId('nav-mode-runs'))
    await waitFor(() => {
      expect(
        fetchMock.mock.calls.some(([request]) =>
          requestUrl(request as RequestInfo | URL).includes('/attractor/runs?project_path=%2Ftmp%2Fproject-beta'),
        ),
      ).toBe(true)
    })
    expect(await screen.findByTestId('runs-project-context-chip')).toHaveTextContent('project-beta')
    expect(screen.getByText('Run history for the active project.')).toBeVisible()
    expect(screen.getByText('beta.dot')).toBeVisible()
  })

  it('[CID:14.0.02] enforces unique project directories and Git-repo registration invariants', async () => {
    const pickedDirectories = [
      '/tmp/project-contract-behavior',
      '/tmp/non-git-project',
      '/tmp/detached-git-project',
      '/tmp/git-project',
    ]
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.endsWith('/workspace/api/projects')) {
        return jsonResponse([
          {
            project_id: 'project-contract-behavior-1234',
            project_path: '/tmp/project-contract-behavior',
            display_name: 'project-contract-behavior',
            is_favorite: false,
            active_conversation_id: null,
          },
        ])
      }
      if (url.includes('/workspace/api/projects/pick-directory')) {
        const nextDirectory = pickedDirectories.shift()
        return jsonResponse(nextDirectory ? { status: 'selected', directory_path: nextDirectory } : { status: 'canceled' })
      }
      if (url.includes('/workspace/api/projects/register')) {
        const payload = init?.body ? JSON.parse(String(init.body)) as { project_path?: string } : {}
        const projectPath = payload.project_path ?? '/tmp/registered-project'
        return jsonResponse({
          project_id: `project-${Math.random().toString(36).slice(2, 8)}`,
          project_path: projectPath,
          display_name: projectPath.split('/').filter(Boolean).at(-1) ?? 'registered-project',
          is_favorite: false,
          active_conversation_id: null,
        })
      }
      if (url.includes('/workspace/api/projects/metadata')) {
        const directory = new URL(url, 'http://localhost').searchParams.get('directory') ?? ''
        if (directory.includes('non-git')) {
          return jsonResponse({ name: 'non-git-project', directory, branch: null, commit: null })
        }
        if (directory.includes('detached')) {
          return jsonResponse({ name: 'detached-git-project', directory, branch: null, commit: 'abc123def456' })
        }
        return jsonResponse({
          name: directory.split('/').filter(Boolean).at(-1) ?? 'project',
          directory,
          branch: 'main',
          commit: 'abc123def456',
        })
      }
      return jsonResponse({})
    })
    vi.stubGlobal('fetch', fetchMock)
    const user = userEvent.setup()

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'projects',
      }))
    })

    render(<Navbar />)
    const newButton = screen.getByTestId('top-nav-project-add-button')
    await user.click(newButton)

    expect(screen.getByTestId('top-nav-project-error')).toHaveTextContent(
      'Project already registered: /tmp/project-contract-behavior',
    )

    await user.click(newButton)

    await waitFor(() => {
      expect(screen.getByTestId('top-nav-project-error')).toHaveTextContent(
        'Project directory must be a Git repository.',
      )
    })
    expect(useStore.getState().projectRegistry['/tmp/non-git-project']).toBeUndefined()

    await user.click(newButton)

    await waitFor(() => {
      expect(useStore.getState().projectRegistry['/tmp/detached-git-project']).toBeDefined()
    })
    expect(screen.queryByTestId('top-nav-project-error')).not.toBeInTheDocument()

    await user.click(newButton)

    await waitFor(() => {
      expect(useStore.getState().projectRegistry['/tmp/git-project']).toBeDefined()
    })

    act(() => {
      useStore.setState((state) => ({
        ...state,
        projectRegistry: {
          ...state.projectRegistry,
          '/tmp/non-git-existing': {
            directoryPath: '/tmp/non-git-existing',
            isFavorite: false,
            lastAccessedAt: null,
          },
        },
        recentProjectPaths: ['/tmp/non-git-existing', ...state.recentProjectPaths],
      }))
    })

    await user.click(screen.getByTestId('top-nav-project-switcher'))
    await user.click(await screen.findByText('non-git-existing'))

    await waitFor(() => {
      expect(screen.getByTestId('top-nav-project-error')).toHaveTextContent(
        'Project directory must be a Git repository.',
      )
    })
    expect(useStore.getState().activeProjectPath).toBe('/tmp/project-contract-behavior')
  })

  it('[CID:6.3.01] renders edge inspector controls for required edge attrs', async () => {
    renderSelectedEdgeSidebar()
    const edgeForm = await screen.findByTestId('edge-structured-form')
    expect(edgeForm).toBeVisible()
    expect(within(edgeForm).getByPlaceholderText('e.g. Approve')).toBeVisible()
    expect(within(edgeForm).getByPlaceholderText('e.g. outcome = "success"')).toBeVisible()
    expect(within(edgeForm).getByPlaceholderText('0')).toBeVisible()
    expect(within(edgeForm).getByPlaceholderText('full | truncate | compact | summary:low')).toBeVisible()
    expect(within(edgeForm).getByLabelText('Loop Restart')).toBeVisible()
  })

  it('[CID:6.3.02] renders edge condition syntax hints and diagnostics preview feedback', async () => {
    renderSelectedEdgeSidebar()
    await screen.findByTestId('edge-structured-form')

    expect(screen.getByTestId('edge-condition-syntax-hints')).toHaveTextContent('Use && to join clauses')
    expect(screen.getByTestId('edge-condition-syntax-hints')).toHaveTextContent(
      'Supported keys: outcome, preferred_label, context.<path>',
    )
    expect(screen.getByTestId('edge-condition-syntax-hints')).toHaveTextContent('Operators: = or !=')
    expect(screen.getByTestId('edge-condition-preview-feedback')).toHaveTextContent(
      'Condition syntax looks valid in preview.',
    )

    act(() => {
      useStore.getState().setDiagnostics([
        {
          rule_id: 'condition_syntax',
          severity: 'error',
          message: 'Condition parser failed near token.',
          edge: ['start', 'task'],
        },
      ])
    })

    await waitFor(() => {
      expect(screen.getByTestId('edge-condition-preview-feedback')).toHaveTextContent(
        'Condition parser failed near token.',
      )
    })
  })

  it('[CID:6.2.02] renders advanced node controls for codergen and wait.human in sidebar inspector', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().setSelectedNodeId('task')
      useStore.getState().setSelectedEdgeId(null)
    })

    const nodes: Node[] = [
      {
        id: 'task',
        position: { x: 0, y: 0 },
        data: { label: 'Task', shape: 'box', type: 'codergen', prompt: 'Do work' },
      },
      {
        id: 'gate',
        position: { x: 150, y: 0 },
        data: { label: 'Gate', shape: 'hexagon', type: 'wait.human', prompt: 'Choose' },
      },
    ]

    renderSidebar(nodes, [])

    await user.click(await screen.findByRole('button', { name: 'Show Advanced' }))
    expect(screen.getByText('Max Retries')).toBeVisible()
    expect(screen.getByText('Goal Gate')).toBeVisible()
    expect(screen.getByText('Retry Target')).toBeVisible()
    expect(screen.getByText('Fallback Retry Target')).toBeVisible()
    expect(screen.getByText('Fidelity')).toBeVisible()
    expect(screen.getByText('Thread ID')).toBeVisible()
    expect(screen.getByText('Class')).toBeVisible()
    expect(screen.getByText('Timeout')).toBeVisible()
    expect(screen.getByText('LLM Model')).toBeVisible()
    expect(screen.getByText('LLM Provider')).toBeVisible()
    expect(screen.getByText('Reasoning Effort')).toBeVisible()
    expect(screen.getByText('Auto Status')).toBeVisible()
    expect(screen.getByText('Allow Partial')).toBeVisible()

    act(() => {
      useStore.getState().setSelectedNodeId('gate')
    })

    await waitFor(() => {
      expect(screen.getByText('Human Default Choice')).toBeVisible()
    })
  })

  it('[CID:6.2.01] renders manager-loop authoring controls in sidebar inspector', async () => {
    renderManagerSidebarInspector()
    expect(await screen.findByText('Manager Poll Interval')).toBeVisible()
    expect(screen.getByRole('option', { name: 'Manager Loop' })).toBeInTheDocument()
    expect(document.querySelector('#node-handler-type-options option[value="stack.manager_loop"]')).toBeTruthy()
  })

  it('[CID:6.7.02] renders manager-loop control fields in sidebar inspector', async () => {
    renderManagerSidebarInspector()
    expect(await screen.findByText('Manager Poll Interval')).toBeVisible()
    expect(screen.getByText('Manager Max Cycles')).toBeVisible()
    expect(screen.getByText('Manager Stop Condition')).toBeVisible()
    expect(screen.getByText('Manager Actions')).toBeVisible()
  })

  it('[CID:6.7.03] renders manager-loop child-linkage affordance in sidebar inspector', async () => {
    renderManagerSidebarInspector()

    const childLinkage = screen.getByTestId('manager-child-linkage')
    expect(await screen.findByText('Manager Poll Interval')).toBeVisible()
    expect(childLinkage).toHaveTextContent('Child Pipeline Linkage')
    expect(childLinkage).toHaveTextContent('stack.child_dotfile')
    expect(childLinkage).toHaveTextContent('child/flow.dot')
    expect(childLinkage).toHaveTextContent('stack.child_workdir')
    expect(childLinkage).toHaveTextContent('/tmp/child')

    fireEvent.click(screen.getByTestId('manager-open-child-settings'))
    expect(useStore.getState().selectedNodeId).toBeNull()
    expect(useStore.getState().selectedEdgeId).toBeNull()
  })

  it('[CID:6.5.02] renders stylesheet diagnostics feedback in graph settings', async () => {
    const user = userEvent.setup()
    renderWithFlowProvider(<GraphSettings inline />)

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    expect(screen.getByTestId('graph-model-stylesheet-selector-guidance')).toBeVisible()

    act(() => {
      useStore.getState().setDiagnostics([
        {
          rule_id: 'stylesheet_syntax',
          severity: 'error',
          message: 'Invalid stylesheet selector syntax.',
          line: 1,
        },
      ])
    })

    expect(screen.getByTestId('graph-model-stylesheet-diagnostics')).toHaveTextContent(
      'Invalid stylesheet selector syntax.',
    )
  })

  it('[CID:6.6.01] renders graph-scope tool hook fields in graph settings', async () => {
    const user = userEvent.setup()
    renderWithFlowProvider(<GraphSettings inline />)

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    const preHookInput = screen.getByTestId('graph-attr-input-tool.hooks.pre')
    const postHookInput = screen.getByTestId('graph-attr-input-tool.hooks.post')
    expect(preHookInput).toBeVisible()
    expect(postHookInput).toBeVisible()
  })

  it('[CID:6.6.02] renders node-level tool hook override controls in sidebar and node toolbar', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().setSelectedNodeId('tool_node')
      useStore.getState().setSelectedEdgeId(null)
    })

    const toolNodeData = {
      label: 'Tool',
      shape: 'parallelogram',
      type: 'tool',
      'tool.command': 'echo run',
      'tool.hooks.pre': 'echo node pre',
      'tool.hooks.post': 'echo node post',
      'tool.artifacts.paths': 'dist/**',
      'tool.artifacts.stdout': 'stdout.txt',
      'tool.artifacts.stderr': 'stderr.txt',
    }
    renderSidebar([
      {
        id: 'tool_node',
        position: { x: 0, y: 0 },
        data: toolNodeData,
      },
    ], [])

    await user.click(await screen.findByRole('button', { name: 'Show Advanced' }))
    expect(screen.getByTestId('node-attr-input-tool.hooks.pre')).toBeVisible()
    expect(screen.getByTestId('node-attr-input-tool.hooks.post')).toBeVisible()
    expect(screen.getByTestId('node-attr-input-tool.artifacts.paths')).toBeVisible()
    expect(screen.getByTestId('node-attr-input-tool.artifacts.stdout')).toBeVisible()
    expect(screen.getByTestId('node-attr-input-tool.artifacts.stderr')).toBeVisible()

    cleanup()
    act(() => {
      resetContractState()
    })
    renderTaskNode({
      id: 'tool_node',
      type: 'task',
      position: { x: 0, y: 0 },
      selected: true,
      data: toolNodeData,
    })

    fireEvent.click(screen.getByText('Edit', { selector: 'button' }))
    fireEvent.click(screen.getByText('Show Advanced', { selector: 'button' }))

    expect(screen.getByTestId('node-toolbar-attr-input-tool.hooks.pre')).toBeVisible()
    expect(screen.getByTestId('node-toolbar-attr-input-tool.hooks.post')).toBeVisible()
    expect(screen.getByTestId('node-toolbar-attr-input-tool.artifacts.paths')).toBeVisible()
    expect(screen.getByTestId('node-toolbar-attr-input-tool.artifacts.stdout')).toBeVisible()
    expect(screen.getByTestId('node-toolbar-attr-input-tool.artifacts.stderr')).toBeVisible()
  })

  it('[CID:6.6.03] renders tool hook warning surfaces in graph settings and node editors', async () => {
    const user = userEvent.setup()
    renderWithFlowProvider(<GraphSettings inline />)

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    fireEvent.change(screen.getByTestId('graph-attr-input-tool.hooks.pre'), { target: { value: "echo 'unterminated" } })
    fireEvent.change(screen.getByTestId('graph-attr-input-tool.hooks.post'), { target: { value: 'echo "unterminated' } })

    await waitFor(() => {
      expect(screen.getByTestId('graph-attr-warning-tool.hooks.pre')).toHaveTextContent('single quote')
      expect(screen.getByTestId('graph-attr-warning-tool.hooks.post')).toHaveTextContent('double quote')
    })
    act(() => {
      cleanup()
      resetContractState()
      useStore.getState().setSelectedNodeId('tool_node')
      useStore.getState().setSelectedEdgeId(null)
    })

    const toolNodeData = {
      label: 'Tool',
      shape: 'parallelogram',
      type: 'tool',
      'tool.command': 'echo run',
      'tool.hooks.pre': 'echo hi\necho there',
      'tool.hooks.post': "echo 'unterminated",
    }
    renderSidebar([
      {
        id: 'tool_node',
        position: { x: 0, y: 0 },
        data: toolNodeData,
      },
    ], [])

    await user.click(await screen.findByRole('button', { name: 'Show Advanced' }))
    expect(screen.getByTestId('node-attr-warning-tool.hooks.pre')).toHaveTextContent('single line')
    expect(screen.getByTestId('node-attr-warning-tool.hooks.post')).toHaveTextContent('single quote')

    cleanup()
    act(() => {
      resetContractState()
    })
    renderTaskNode({
      id: 'tool_node',
      type: 'task',
      position: { x: 0, y: 0 },
      selected: true,
      data: toolNodeData,
    })

    fireEvent.click(screen.getByText('Edit', { selector: 'button' }))
    fireEvent.click(screen.getByText('Show Advanced', { selector: 'button' }))

    expect(screen.getByTestId('node-toolbar-attr-input-tool.hooks.pre')).toBeVisible()
    expect(screen.getByTestId('node-toolbar-attr-input-tool.hooks.post')).toBeVisible()
    expect(screen.getByTestId('node-toolbar-attr-warning-tool.hooks.pre')).toHaveTextContent('single line')
    expect(screen.getByTestId('node-toolbar-attr-warning-tool.hooks.post')).toHaveTextContent('single quote')
  })

  it('[CID:6.7.01] renders manager-loop shape and type options in task node toolbar', () => {
    resetContractState()
    renderTaskNode({
      id: 'manager',
      type: 'task',
      position: { x: 0, y: 0 },
      selected: true,
      data: {
        label: 'Manager',
        shape: 'house',
        type: 'stack.manager_loop',
        'manager.poll_interval': '25ms',
        'manager.max_cycles': 3,
        'manager.stop_condition': 'child.outcome == "success"',
        'manager.actions': 'observe,steer',
      },
    })

    fireEvent.click(screen.getByText('Edit', { selector: 'button' }))
    expect(screen.getByRole('option', { name: 'Manager Loop' })).toBeInTheDocument()
    expect(document.querySelector('#node-handler-type-options-manager option[value="stack.manager_loop"]')).toBeTruthy()
    expect(screen.getByText('Manager Poll Interval')).toBeVisible()
    expect(screen.getByText('Manager Max Cycles')).toBeVisible()
    expect(screen.getByText('Manager Stop Condition')).toBeVisible()
    expect(screen.getByText('Manager Actions')).toBeVisible()
  })

  it('[CID:10.1.01] keeps pending human gates discoverable in runs and execution views', async () => {
    const runId = 'run-contract-human-gate'
    const pendingPrompt = 'Approve production deploy?'
    const gateId = 'gate-1'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              current_node: 'review_gate',
              completed_nodes: ['start'],
              retry_counts: {},
            },
          })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: { 'graph.goal': 'Human gate discoverability contract' },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: gateId,
              node_id: 'review_gate',
              prompt: pendingPrompt,
              options: [{ label: 'Approve', value: 'approve' }],
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-pending-human-gates-panel')).toBeVisible()
    })
    expect(screen.getByTestId('run-pending-human-gate-item')).toHaveTextContent(pendingPrompt)

    cleanup()
    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'execution',
        selectedRunId: runId,
        runtimeStatus: 'running',
        humanGate: {
          id: gateId,
          runId,
          nodeId: 'review_gate',
          prompt: pendingPrompt,
          options: [{ label: 'Approve', value: 'approve' }],
          flowName: 'contract-behavior.dot',
        },
      }))
    })
    render(<ExecutionControls />)

    expect(screen.getByTestId('execution-pending-human-gate-banner')).toHaveTextContent('Pending human gate')
    expect(screen.getByTestId('execution-pending-human-gate-banner')).toHaveTextContent(pendingPrompt)
  })

  it('[CID:10.1.02] lets operator answer pending human gates from runs view controls', async () => {
    const runId = 'run-contract-human-gate-answer'
    const gateId = 'gate-approve'
    const pendingPrompt = 'Approve production deploy?'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const answerPath = `${runApiPath}/questions/${encodeURIComponent(gateId)}/answer`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = requestUrl(input)
      if (url.includes('/attractor/runs')) {
        return jsonResponse({ runs: [runRecord] })
      }
      if (url.endsWith(`${runApiPath}/checkpoint`)) {
        return jsonResponse({
          pipeline_id: runId,
          checkpoint: {
            current_node: 'review_gate',
            completed_nodes: ['start'],
            retry_counts: {},
          },
        })
      }
      if (url.endsWith(`${runApiPath}/context`)) {
        return jsonResponse({
          pipeline_id: runId,
          context: { 'graph.goal': 'Human gate answerability contract' },
        })
      }
      if (url.endsWith(`${runApiPath}/artifacts`)) {
        return jsonResponse({
          pipeline_id: runId,
          artifacts: [],
        })
      }
      if (url.endsWith(`${runApiPath}/graph`)) {
        return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
          status: 200,
          headers: { 'Content-Type': 'image/svg+xml' },
        })
      }
      if (url.endsWith(answerPath)) {
        return jsonResponse({ status: 'accepted', pipeline_id: runId, question_id: gateId })
      }
      return jsonResponse({}, { status: 404 })
    })
    vi.stubGlobal('fetch', fetchMock)

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: gateId,
              node_id: 'review_gate',
              prompt: pendingPrompt,
              options: [
                { label: 'Approve', value: 'approve' },
                { label: 'Reject', value: 'reject' },
              ],
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-pending-human-gates-panel')).toBeVisible()
    })

    const answerButton = screen.getByTestId('run-pending-human-gate-answer-approve')
    fireEvent.click(answerButton)

    await waitFor(() => {
      const submissionCall = fetchMock.mock.calls.find(([input]) => requestUrl(input as RequestInfo | URL).endsWith(answerPath))
      expect(submissionCall).toBeTruthy()
      const [, init] = submissionCall as [RequestInfo | URL, RequestInit | undefined]
      expect(init?.method).toBe('POST')
      expect(init?.body).toBe(JSON.stringify({
        question_id: gateId,
        selected_value: 'approve',
      }))
    })

    await waitFor(() => {
      expect(screen.queryByTestId('run-pending-human-gate-item')).not.toBeInTheDocument()
    })
  })

  it('[CID:10.2.01] renders MULTIPLE_CHOICE pending gate options with option metadata', async () => {
    const runId = 'run-contract-human-gate-metadata'
    const gateId = 'gate-metadata'
    const pendingPrompt = 'Choose deployment strategy'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              current_node: 'review_gate',
              completed_nodes: ['start'],
              retry_counts: {},
            },
          })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: { 'graph.goal': 'Human gate multiple-choice metadata contract' },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: gateId,
              question_type: 'MULTIPLE_CHOICE',
              node_id: 'review_gate',
              prompt: pendingPrompt,
              options: [
                {
                  key: 'A',
                  label: 'Approve',
                  value: 'approve',
                  description: 'Ship now to production.',
                },
                {
                  key: 'R',
                  label: 'Request Rework',
                  value: 'rework',
                  description: 'Send build back for revision.',
                },
              ],
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-pending-human-gates-panel')).toBeVisible()
    })

    expect(screen.getByTestId('run-pending-human-gate-item')).toHaveTextContent(pendingPrompt)
    expect(screen.getByTestId('run-pending-human-gate-option-metadata-approve')).toHaveTextContent('[A]')
    expect(screen.getByTestId('run-pending-human-gate-option-metadata-approve')).toHaveTextContent('Ship now to production.')
    expect(screen.getByTestId('run-pending-human-gate-option-metadata-rework')).toHaveTextContent('[R]')
    expect(screen.getByTestId('run-pending-human-gate-option-metadata-rework')).toHaveTextContent('Send build back for revision.')
  })

  it('[CID:10.2.02] renders YES_NO and CONFIRMATION pending gates with explicit yes/no and confirm/cancel semantics', async () => {
    const runId = 'run-contract-human-gate-semantic-types'
    const yesNoGateId = 'gate-yes-no'
    const confirmationGateId = 'gate-confirmation'
    const yesNoPrompt = 'Continue rollout?'
    const confirmationPrompt = 'Finalize release promotion?'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              current_node: 'review_gate',
              completed_nodes: ['start'],
              retry_counts: {},
            },
          })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: { 'graph.goal': 'Human gate semantic question-type contract' },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: yesNoGateId,
              question_type: 'YES_NO',
              node_id: 'review_gate',
              prompt: yesNoPrompt,
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: confirmationGateId,
              question_type: 'CONFIRMATION',
              node_id: 'release_gate',
              prompt: confirmationPrompt,
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-pending-human-gates-panel')).toBeVisible()
    })

    const pendingItems = screen.getAllByTestId('run-pending-human-gate-item')
    const yesNoItem = pendingItems.find((item) => item.textContent?.includes(yesNoPrompt))
    const confirmationItem = pendingItems.find((item) => item.textContent?.includes(confirmationPrompt))

    expect(yesNoItem).toBeTruthy()
    expect(confirmationItem).toBeTruthy()

    const yesNoScope = within(yesNoItem as HTMLElement)
    expect(yesNoScope.getByRole('button', { name: 'Yes' })).toBeVisible()
    expect(yesNoScope.getByRole('button', { name: 'No' })).toBeVisible()
    expect(yesNoScope.getByText('Sends YES')).toBeVisible()
    expect(yesNoScope.getByText('Sends NO')).toBeVisible()

    const confirmationScope = within(confirmationItem as HTMLElement)
    expect(confirmationScope.getByRole('button', { name: 'Confirm' })).toBeVisible()
    expect(confirmationScope.getByRole('button', { name: 'Cancel' })).toBeVisible()
    expect(confirmationScope.getByText('Sends YES')).toBeVisible()
    expect(confirmationScope.getByText('Sends NO')).toBeVisible()
  })

  it('[CID:10.2.03] renders FREEFORM pending gates with text input and submit action', async () => {
    const runId = 'run-contract-human-gate-freeform'
    const gateId = 'gate-freeform'
    const pendingPrompt = 'Provide release notes for this deployment gate.'
    const freeformAnswer = 'Need one more staging pass before production rollout.'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const answerPath = `${runApiPath}/questions/${encodeURIComponent(gateId)}/answer`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = requestUrl(input)
      if (url.includes('/attractor/runs')) {
        return jsonResponse({ runs: [runRecord] })
      }
      if (url.endsWith(`${runApiPath}/checkpoint`)) {
        return jsonResponse({
          pipeline_id: runId,
          checkpoint: {
            current_node: 'review_gate',
            completed_nodes: ['start'],
            retry_counts: {},
          },
        })
      }
      if (url.endsWith(`${runApiPath}/context`)) {
        return jsonResponse({
          pipeline_id: runId,
          context: { 'graph.goal': 'Human gate freeform contract' },
        })
      }
      if (url.endsWith(`${runApiPath}/artifacts`)) {
        return jsonResponse({
          pipeline_id: runId,
          artifacts: [],
        })
      }
      if (url.endsWith(`${runApiPath}/graph`)) {
        return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
          status: 200,
          headers: { 'Content-Type': 'image/svg+xml' },
        })
      }
      if (url.endsWith(answerPath)) {
        return jsonResponse({ status: 'accepted', pipeline_id: runId, question_id: gateId })
      }
      return jsonResponse({}, { status: 404 })
    })
    vi.stubGlobal('fetch', fetchMock)

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: gateId,
              question_type: 'FREEFORM',
              node_id: 'review_gate',
              prompt: pendingPrompt,
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-pending-human-gates-panel')).toBeVisible()
    })

    expect(screen.getByTestId('run-pending-human-gate-item')).toHaveTextContent(pendingPrompt)
    const input = screen.getByTestId(`run-pending-human-gate-freeform-input-${gateId}`) as HTMLInputElement
    const submitButton = screen.getByTestId(`run-pending-human-gate-freeform-submit-${gateId}`)
    expect(submitButton).toBeDisabled()

    fireEvent.change(input, { target: { value: freeformAnswer } })
    expect(input.value).toBe(freeformAnswer)
    expect(submitButton).toBeEnabled()
    fireEvent.click(submitButton)

    await waitFor(() => {
      const submissionCall = fetchMock.mock.calls.find(([inputArg]) => requestUrl(inputArg as RequestInfo | URL).endsWith(answerPath))
      expect(submissionCall).toBeTruthy()
      const [, init] = submissionCall as [RequestInfo | URL, RequestInit | undefined]
      expect(init?.method).toBe('POST')
      expect(init?.body).toBe(JSON.stringify({
        question_id: gateId,
        selected_value: freeformAnswer,
      }))
    })

    await waitFor(() => {
      expect(screen.queryByTestId('run-pending-human-gate-item')).not.toBeInTheDocument()
    })
  })

  it('[CID:10.2.04] covers each supported human-gate question type with type-specific UI affordances', async () => {
    const runId = 'run-contract-human-gate-type-matrix'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }
    const multipleChoiceGateId = 'gate-matrix-multiple-choice'
    const yesNoGateId = 'gate-matrix-yes-no'
    const confirmationGateId = 'gate-matrix-confirmation'
    const freeformGateId = 'gate-matrix-freeform'

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              current_node: 'review_gate',
              completed_nodes: ['start'],
              retry_counts: {},
            },
          })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: { 'graph.goal': 'Human gate question type matrix contract' },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: multipleChoiceGateId,
              question_type: 'MULTIPLE_CHOICE',
              node_id: 'review_gate_multiple',
              prompt: 'Choose deployment strategy',
              options: [
                {
                  key: 'P',
                  label: 'Promote',
                  value: 'promote',
                  description: 'Advance this build to production.',
                },
                {
                  key: 'H',
                  label: 'Hold',
                  value: 'hold',
                  description: 'Pause rollout and gather more evidence.',
                },
              ],
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: yesNoGateId,
              question_type: 'YES_NO',
              node_id: 'review_gate_yes_no',
              prompt: 'Continue migration?',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: confirmationGateId,
              question_type: 'CONFIRMATION',
              node_id: 'release_gate_confirmation',
              prompt: 'Finalize promotion?',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: freeformGateId,
              question_type: 'FREEFORM',
              node_id: 'release_gate_freeform',
              prompt: 'Add release notes before promotion.',
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-pending-human-gates-panel')).toBeVisible()
    })

    await waitFor(() => {
      const pendingItems = screen.getAllByTestId('run-pending-human-gate-item')
      expect(pendingItems.some((item) => item.textContent?.includes('Choose deployment strategy'))).toBe(true)
      expect(pendingItems.some((item) => item.textContent?.includes('Continue migration?'))).toBe(true)
      expect(pendingItems.some((item) => item.textContent?.includes('Finalize promotion?'))).toBe(true)
      expect(pendingItems.some((item) => item.textContent?.includes('Add release notes before promotion.'))).toBe(true)
    })

    const pendingItems = screen.getAllByTestId('run-pending-human-gate-item')
    const multipleChoiceItem = pendingItems.find((item) => item.textContent?.includes('Choose deployment strategy'))
    const yesNoItem = pendingItems.find((item) => item.textContent?.includes('Continue migration?'))
    const confirmationItem = pendingItems.find((item) => item.textContent?.includes('Finalize promotion?'))

    expect(multipleChoiceItem).toBeTruthy()
    expect(yesNoItem).toBeTruthy()
    expect(confirmationItem).toBeTruthy()

    const multipleChoiceScope = within(multipleChoiceItem as HTMLElement)
    expect(multipleChoiceScope.getByRole('button', { name: 'Promote' })).toBeVisible()
    expect(screen.getByTestId('run-pending-human-gate-option-metadata-promote')).toHaveTextContent('[P]')
    expect(screen.getByTestId('run-pending-human-gate-option-metadata-promote')).toHaveTextContent(
      'Advance this build to production.',
    )

    const yesNoScope = within(yesNoItem as HTMLElement)
    expect(yesNoScope.getByRole('button', { name: 'Yes' })).toBeVisible()
    expect(yesNoScope.getByRole('button', { name: 'No' })).toBeVisible()
    expect(yesNoScope.getByText('Sends YES')).toBeVisible()
    expect(yesNoScope.getByText('Sends NO')).toBeVisible()

    const confirmationScope = within(confirmationItem as HTMLElement)
    expect(confirmationScope.getByRole('button', { name: 'Confirm' })).toBeVisible()
    expect(confirmationScope.getByRole('button', { name: 'Cancel' })).toBeVisible()
    expect(confirmationScope.getByText('Sends YES')).toBeVisible()
    expect(confirmationScope.getByText('Sends NO')).toBeVisible()

    const freeformInput = screen.getByTestId(`run-pending-human-gate-freeform-input-${freeformGateId}`)
    const freeformSubmit = screen.getByTestId(`run-pending-human-gate-freeform-submit-${freeformGateId}`)
    expect(freeformInput).toBeVisible()
    expect(freeformSubmit).toBeDisabled()
  })

  it('[CID:10.4.01] groups multi-question pending prompts by originating stage', async () => {
    const runId = 'run-contract-human-gate-grouped-prompts'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              current_node: 'review_gate',
              completed_nodes: ['start'],
              retry_counts: {},
            },
          })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: { 'graph.goal': 'Human gate grouped-prompt contract' },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: 'gate-review-1',
              node_id: 'review_gate',
              index: 2,
              question_type: 'MULTIPLE_CHOICE',
              prompt: 'Choose deployment strategy',
              options: [
                { label: 'Promote', value: 'promote' },
                { label: 'Rollback', value: 'rollback' },
              ],
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: 'gate-review-2',
              node_id: 'review_gate',
              index: 2,
              question_type: 'FREEFORM',
              prompt: 'Why this strategy?',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: 'gate-approval-1',
              node_id: 'approval_gate',
              index: 3,
              question_type: 'CONFIRMATION',
              prompt: 'Finalize production promotion?',
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getAllByTestId('run-pending-human-gate-group')).toHaveLength(2)
    })

    const groups = screen.getAllByTestId('run-pending-human-gate-group')
    const reviewGroup = groups.find((group) =>
      within(group).getByTestId('run-pending-human-gate-group-heading').textContent?.includes('review_gate'),
    )
    const approvalGroup = groups.find((group) =>
      within(group).getByTestId('run-pending-human-gate-group-heading').textContent?.includes('approval_gate'),
    )

    expect(reviewGroup).toBeTruthy()
    expect(approvalGroup).toBeTruthy()

    const reviewScope = within(reviewGroup as HTMLElement)
    expect(reviewScope.getAllByTestId('run-pending-human-gate-item')).toHaveLength(2)
    expect(reviewScope.getByText(/Choose deployment strategy/)).toBeVisible()
    expect(reviewScope.getByText(/Why this strategy\?/)).toBeVisible()

    const approvalScope = within(approvalGroup as HTMLElement)
    expect(approvalScope.getAllByTestId('run-pending-human-gate-item')).toHaveLength(1)
    expect(approvalScope.getByText(/Finalize production promotion\?/)).toBeVisible()
  })

  it('[CID:10.4.02] displays interviewer inform messages in context of the originating stage', async () => {
    const runId = 'run-contract-human-gate-inform-messages'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              current_node: 'review_gate',
              completed_nodes: ['start'],
              retry_counts: {},
            },
          })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: { 'graph.goal': 'Human gate inform-message contract' },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'InterviewInform',
              stage: 'review_gate',
              index: 2,
              message: 'Policy reminder: include rollback evidence.',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'InterviewInform',
              stage: 'approval_gate',
              index: 3,
              message: 'Approver is offline; escalation path is active.',
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-pending-human-gates-panel')).toBeVisible()
    })

    await waitFor(() => {
      expect(screen.getByText('Policy reminder: include rollback evidence.')).toBeVisible()
      expect(screen.getByText('Approver is offline; escalation path is active.')).toBeVisible()
    })

    const groups = screen.getAllByTestId('run-pending-human-gate-group')
    const reviewGroup = groups.find((group) =>
      within(group).getByTestId('run-pending-human-gate-group-heading').textContent?.includes('review_gate'),
    )
    const approvalGroup = groups.find((group) =>
      within(group).getByTestId('run-pending-human-gate-group-heading').textContent?.includes('approval_gate'),
    )
    expect(reviewGroup).toBeTruthy()
    expect(approvalGroup).toBeTruthy()

    expect(within(reviewGroup as HTMLElement).queryAllByRole('button')).toHaveLength(0)
    expect(within(approvalGroup as HTMLElement).queryAllByRole('button')).toHaveLength(0)
  })

  it('[CID:10.4.03] preserves grouped interaction order and audit metadata', async () => {
    const runId = 'run-contract-human-gate-order-auditability'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              current_node: 'review_gate',
              completed_nodes: ['start'],
              retry_counts: {},
            },
          })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: { 'graph.goal': 'Human gate grouped-order auditability contract' },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: 'gate-review-1',
              node_id: 'review_gate',
              index: 2,
              question_type: 'MULTIPLE_CHOICE',
              prompt: 'Choose deployment strategy',
              options: [
                { label: 'Promote', value: 'promote' },
                { label: 'Rollback', value: 'rollback' },
              ],
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'InterviewInform',
              stage: 'review_gate',
              index: 2,
              message: 'Policy reminder: include rollback evidence.',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: 'gate-review-2',
              node_id: 'review_gate',
              index: 2,
              question_type: 'FREEFORM',
              prompt: 'Why this strategy?',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'human_gate',
              question_id: 'gate-approval-1',
              node_id: 'approval_gate',
              index: 3,
              question_type: 'CONFIRMATION',
              prompt: 'Finalize production promotion?',
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getAllByTestId('run-pending-human-gate-group')).toHaveLength(2)
    })

    const groups = screen.getAllByTestId('run-pending-human-gate-group')
    const groupHeadings = groups.map((group) =>
      within(group).getByTestId('run-pending-human-gate-group-heading').textContent,
    )
    expect(groupHeadings).toEqual([
      'review_gate (index 2)',
      'approval_gate (index 3)',
    ])

    const reviewGroup = groups[0]
    const reviewScope = within(reviewGroup)
    const reviewItems = reviewScope.getAllByTestId('run-pending-human-gate-item')
    expect(reviewItems).toHaveLength(3)
    expect(reviewItems[0]).toHaveTextContent('Choose deployment strategy')
    expect(reviewItems[1]).toHaveTextContent('Policy reminder: include rollback evidence.')
    expect(reviewItems[2]).toHaveTextContent('Why this strategy?')

    const firstAudit = within(reviewItems[0]).getByTestId('run-pending-human-gate-item-audit')
    expect(firstAudit).toHaveTextContent('Order #1')
    expect(firstAudit).toHaveTextContent('Question ID: gate-review-1')
    expect(firstAudit).toHaveTextContent('Received:')

    const secondAudit = within(reviewItems[1]).getByTestId('run-pending-human-gate-item-audit')
    expect(secondAudit).toHaveTextContent('Order #2')
    expect(secondAudit).toHaveTextContent('Question ID: —')
    expect(secondAudit).toHaveTextContent('Received:')

    const thirdAudit = within(reviewItems[2]).getByTestId('run-pending-human-gate-item-audit')
    expect(thirdAudit).toHaveTextContent('Order #3')
    expect(thirdAudit).toHaveTextContent('Question ID: gate-review-2')
    expect(thirdAudit).toHaveTextContent('Received:')
  })

  it('[CID:10.3.02] renders timeout/default-applied/skipped provenance in run timeline summaries', async () => {
    const runId = 'run-contract-human-gate-provenance'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              current_node: 'review_gate',
              completed_nodes: ['start'],
              retry_counts: {},
            },
          })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: { 'graph.goal': 'Human gate provenance contract' },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'InterviewTimeout',
              stage: 'review_gate',
              index: 2,
              question: 'Select release path',
              outcome_provenance: 'timeout_default_applied',
              default_choice_label: 'Fix',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'InterviewCompleted',
              stage: 'review_gate',
              index: 2,
              question: 'Select release path',
              answer: 'Approve',
              outcome_provenance: 'accepted',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'InterviewCompleted',
              stage: 'release_gate',
              index: 3,
              question: 'Finalize deployment?',
              answer: 'skipped',
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-list')).toBeVisible()
    })

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-list')).toHaveTextContent(
        'Interview timed out for review_gate (default applied: Fix)',
      )
    })
    expect(screen.getByTestId('run-event-timeline-list')).toHaveTextContent(
      'Interview completed for review_gate (accepted answer: Approve)',
    )
    expect(screen.getByTestId('run-event-timeline-list')).toHaveTextContent(
      'Interview completed for release_gate (skipped)',
    )
  })

  it('[CID:10.3.03] falls back to timeout and explicit-answer branches when outcome provenance is omitted', async () => {
    const runId = 'run-contract-human-gate-provenance-fallback'
    const runApiPath = `/attractor/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      outcome: null,
      working_directory: '/tmp/project-contract-behavior/workspace',
      project_path: '/tmp/project-contract-behavior',
      git_branch: 'main',
      git_commit: 'abc1234',
      model: 'gpt-5',
      started_at: '2026-03-04T01:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.includes('/attractor/runs')) {
          return jsonResponse({ runs: [runRecord] })
        }
        if (url.endsWith(`${runApiPath}/checkpoint`)) {
          return jsonResponse({
            pipeline_id: runId,
            checkpoint: {
              current_node: 'review_gate',
              completed_nodes: ['start'],
              retry_counts: {},
            },
          })
        }
        if (url.endsWith(`${runApiPath}/context`)) {
          return jsonResponse({
            pipeline_id: runId,
            context: { 'graph.goal': 'Human gate timeout fallback contract' },
          })
        }
        if (url.endsWith(`${runApiPath}/artifacts`)) {
          return jsonResponse({
            pipeline_id: runId,
            artifacts: [],
          })
        }
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
          })
        }
        return jsonResponse({}, { status: 404 })
      }),
    )

    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        if (!url.includes(`${runApiPath}/events`)) {
          return
        }
        setTimeout(() => {
          this.onopen?.(new Event('open'))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'InterviewTimeout',
              stage: 'review_gate',
              index: 2,
              question: 'Select release path',
              default_choice_label: 'Fix',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'InterviewTimeout',
              stage: 'approval_gate',
              index: 3,
              question: 'Finalize deployment?',
            }),
          }))
          this.onmessage?.(new MessageEvent('message', {
            data: JSON.stringify({
              type: 'InterviewCompleted',
              stage: 'review_gate',
              index: 2,
              question: 'Select release path',
              answer: 'Approve',
            }),
          }))
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'runs',
        selectedRunId: runId,
        runtimeStatus: 'running',
      }))
    })

    renderRunsPanelWithController()

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-list')).toBeVisible()
    })

    await waitFor(() => {
      expect(screen.getByTestId('run-event-timeline-list')).toHaveTextContent(
        'Interview timed out for review_gate (default applied: Fix)',
      )
    })
    expect(screen.getByTestId('run-event-timeline-list')).toHaveTextContent(
      'Interview timed out for approval_gate (no default applied)',
    )
    expect(screen.getByTestId('run-event-timeline-list')).toHaveTextContent(
      'Interview completed for review_gate (accepted answer: Approve)',
    )
  })

  it('[CID:11.3.01] keeps raw-to-structured handoff single-flight during repeated transition clicks', async () => {
    const initialDot = 'digraph contract_behavior { start [label="Start"]; }'
    const previewPayload = {
      graph: {
        graph_attrs: {},
        defaults: {
          node: {},
          edge: {},
        },
        subgraphs: [],
        nodes: [
          {
            id: 'start',
            label: 'Start',
            shape: 'box',
          },
        ],
        edges: [],
      },
      diagnostics: [],
    }
    const saveResolvers: Array<() => void> = []
    const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.endsWith('/attractor/api/flows/contract-behavior.dot')) {
        return Promise.resolve(jsonResponse({ content: initialDot }))
      }
      if (url.endsWith('/attractor/preview')) {
        return Promise.resolve(jsonResponse(previewPayload))
      }
      if (url.endsWith('/attractor/api/flows') && init?.method === 'POST') {
        return new Promise<Response>((resolve) => {
          saveResolvers.push(() => resolve(jsonResponse({ status: 'saved' })))
        })
      }
      return Promise.resolve(jsonResponse({}, { status: 404 }))
    })
    vi.stubGlobal('fetch', fetchMock)

    const user = userEvent.setup()
    renderWithFlowProvider(<Editor />)

    await screen.findByTestId('editor-mode-toggle')
    await user.click(screen.getByRole('button', { name: 'Raw DOT' }))
    expect(await screen.findByTestId('raw-dot-editor')).toBeVisible()
    const previewCallsBeforeHandoff = fetchMock.mock.calls.filter(([input]) => {
      const callUrl = requestUrl(input as RequestInfo | URL)
      return callUrl.endsWith('/attractor/preview')
    }).length

    const structuredButton = screen.getByRole('button', { name: 'Structured' })
    fireEvent.click(structuredButton)
    fireEvent.click(structuredButton)

    await waitFor(() => {
      expect(screen.queryByTestId('raw-dot-editor')).not.toBeInTheDocument()
    })

    const saveCalls = fetchMock.mock.calls.filter(([input, requestInit]) => {
      const callUrl = requestUrl(input as RequestInfo | URL)
      return callUrl.endsWith('/attractor/api/flows') && (requestInit as RequestInit | undefined)?.method === 'POST'
    })
    expect(saveCalls.length).toBeLessThanOrEqual(1)

    const previewCallsAfterHandoff = fetchMock.mock.calls.filter(([input]) => {
      const callUrl = requestUrl(input as RequestInfo | URL)
      return callUrl.endsWith('/attractor/preview')
    }).length
    expect(previewCallsAfterHandoff - previewCallsBeforeHandoff).toBe(1)
    expect(saveResolvers).toHaveLength(0)
  })

  it('[CID:11.3.02] preserves unsurfaced canonical data through structured and raw edit paths', async () => {
    const initialDot = `
digraph contract_behavior {
  graph [goal="Ship release", x_unsurfaced_graph="keep-graph"];
  node [x_unsurfaced_node_default="keep-node-default"];
  edge [x_unsurfaced_edge_default="keep-edge-default"];
  subgraph cluster_review {
    graph [x_unsurfaced_scope="keep-scope"];
    start;
  }
  start [label="Start", shape=box, prompt="Plan release", x_unsurfaced_node="keep-node"];
  end [label="End", shape=Msquare];
  start -> end [label="next", x_unsurfaced_edge="keep-edge"];
}
`.trim()
    const previewPayload = {
      graph: {
        graph_attrs: {
          goal: 'Ship release',
          x_unsurfaced_graph: 'keep-graph',
        },
        defaults: {
          node: {
            x_unsurfaced_node_default: 'keep-node-default',
          },
          edge: {
            x_unsurfaced_edge_default: 'keep-edge-default',
          },
        },
        subgraphs: [
          {
            id: 'cluster_review',
            attrs: {
              x_unsurfaced_scope: 'keep-scope',
            },
            node_ids: ['start'],
            defaults: {
              node: {},
              edge: {},
            },
            subgraphs: [],
          },
        ],
        nodes: [
          {
            id: 'start',
            label: 'Start',
            shape: 'box',
            prompt: 'Plan release',
            x_unsurfaced_node: 'keep-node',
          },
          {
            id: 'end',
            label: 'End',
            shape: 'Msquare',
          },
        ],
        edges: [
          {
            from: 'start',
            to: 'end',
            label: 'next',
            x_unsurfaced_edge: 'keep-edge',
          },
        ],
      },
      diagnostics: [],
    }
    const savedPayloads: Array<{ name: string; content: string }> = []
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.endsWith('/attractor/api/flows/contract-behavior.dot')) {
        return jsonResponse({ content: initialDot })
      }
      if (url.endsWith('/attractor/preview')) {
        return jsonResponse(previewPayload)
      }
      if (url.endsWith('/attractor/api/flows') && init?.method === 'POST') {
        const payload = JSON.parse(String(init.body)) as { name: string; content: string }
        savedPayloads.push(payload)
        return jsonResponse({ status: 'saved' })
      }
      return jsonResponse({}, { status: 404 })
    })
    vi.stubGlobal('fetch', fetchMock)

    const user = userEvent.setup()
    renderWithFlowProvider(<Editor />)

    await screen.findByTestId('editor-mode-toggle')
    await screen.findByText('Start')
    await user.click(screen.getByRole('button', { name: 'Add Node' }))

    await waitFor(() => {
      expect(savedPayloads.length).toBeGreaterThanOrEqual(1)
    })

    const structuredSave = savedPayloads[0].content
    expect(structuredSave).toContain('x_unsurfaced_graph="keep-graph"')
    expect(structuredSave).toContain('x_unsurfaced_node_default="keep-node-default"')
    expect(structuredSave).toContain('x_unsurfaced_edge_default="keep-edge-default"')
    expect(structuredSave).toContain('subgraph cluster_review {')
    expect(structuredSave).toContain('x_unsurfaced_scope="keep-scope"')
    expect(structuredSave).toContain('x_unsurfaced_node="keep-node"')
    expect(structuredSave).toContain('x_unsurfaced_edge="keep-edge"')

    await user.click(screen.getByRole('button', { name: 'Raw DOT' }))
    const rawEditor = await screen.findByTestId('raw-dot-editor')
    const rawDraftValue = (rawEditor as HTMLTextAreaElement).value
    expect(rawDraftValue).toContain('x_unsurfaced_node_default="keep-node-default"')
    expect(rawDraftValue).toContain('x_unsurfaced_edge_default="keep-edge-default"')
    expect(rawDraftValue).toContain('subgraph cluster_review {')
    expect(rawDraftValue).toContain('x_unsurfaced_node="keep-node"')
    expect(rawDraftValue).toContain('x_unsurfaced_edge="keep-edge"')

    await user.click(screen.getByRole('button', { name: 'Structured' }))
    await waitFor(() => {
      expect(screen.queryByTestId('raw-dot-editor')).not.toBeInTheDocument()
    })

    expect(savedPayloads).toHaveLength(1)

    await user.click(screen.getByRole('button', { name: 'Raw DOT' }))
    const roundTrippedRawEditor = await screen.findByTestId('raw-dot-editor')
    const roundTrippedRawValue = (roundTrippedRawEditor as HTMLTextAreaElement).value
    expect(roundTrippedRawValue).toContain('x_unsurfaced_node_default="keep-node-default"')
    expect(roundTrippedRawValue).toContain('x_unsurfaced_edge_default="keep-edge-default"')
    expect(roundTrippedRawValue).toContain('subgraph cluster_review {')
    expect(roundTrippedRawValue).toContain('x_unsurfaced_node="keep-node"')
    expect(roundTrippedRawValue).toContain('x_unsurfaced_edge="keep-edge"')
  })

  it('[CID:11.3.03] blocks raw-to-structured handoff when raw edits conflict with structured assumptions', async () => {
    const initialDot = 'digraph contract_behavior { start [label="Start"]; }'
    const previewOkPayload = {
      status: 'ok',
      graph: {
        graph_attrs: {},
        defaults: {
          node: {},
          edge: {},
        },
        subgraphs: [],
        nodes: [
          {
            id: 'start',
            label: 'Start',
            shape: 'box',
          },
        ],
        edges: [],
      },
      diagnostics: [],
      errors: [],
    }
    const previewConflictPayload = {
      status: 'validation_error',
      graph: {
        graph_attrs: {},
        defaults: {
          node: {},
          edge: {},
        },
        subgraphs: [],
        nodes: [
          {
            id: 'start',
            label: 'Start',
            shape: 'box',
          },
        ],
        edges: [
          {
            from: 'start',
            to: 'missing',
          },
        ],
      },
      diagnostics: [
        {
          rule_id: 'edge_target_exists',
          severity: 'error',
          message: 'edge target does not exist',
          edge: ['start', 'missing'],
        },
      ],
      errors: [
        {
          rule_id: 'edge_target_exists',
          severity: 'error',
          message: 'edge target does not exist',
          edge: ['start', 'missing'],
        },
      ],
    }
    let previewRequestCount = 0
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.endsWith('/attractor/api/flows/contract-behavior.dot')) {
        return jsonResponse({ content: initialDot })
      }
      if (url.endsWith('/attractor/preview')) {
        const payload = previewRequestCount === 0 ? previewOkPayload : previewConflictPayload
        previewRequestCount += 1
        return jsonResponse(payload)
      }
      if (url.endsWith('/attractor/api/flows') && init?.method === 'POST') {
        return jsonResponse({ status: 'saved' })
      }
      return jsonResponse({}, { status: 404 })
    })
    vi.stubGlobal('fetch', fetchMock)

    const user = userEvent.setup()
    renderWithFlowProvider(<Editor />)

    await screen.findByTestId('editor-mode-toggle')
    await user.click(screen.getByRole('button', { name: 'Raw DOT' }))
    const rawEditor = await screen.findByTestId('raw-dot-editor')
    fireEvent.change(rawEditor, { target: { value: 'digraph contract_behavior { start; start -> missing; }' } })

    await user.click(screen.getByRole('button', { name: 'Structured' }))

    await waitFor(() => {
      expect(screen.getByTestId('raw-dot-editor')).toBeVisible()
    })
    expect(screen.getByTestId('raw-dot-handoff-error')).toHaveTextContent(
      'Raw DOT edit conflicts with structured mode assumptions.',
    )
    expect(screen.getByRole('button', { name: 'Structured' })).toBeEnabled()
  })

  it('[CID:11.4.01] renders generic extension key/value editors for non-core graph, node, and edge attrs', async () => {
    const user = userEvent.setup()
    act(() => {
      useStore.getState().setGraphAttrs({
        goal: 'Release',
        x_graph_extension: 'graph-extra',
      } as never)
      useStore.getState().setSelectedNodeId('task')
      useStore.getState().setSelectedEdgeId(null)
    })

    const nodes: Node[] = [
      {
        id: 'start',
        position: { x: 0, y: 0 },
        data: { label: 'Start', shape: 'Mdiamond' },
      },
      {
        id: 'task',
        position: { x: 150, y: 0 },
        data: {
          label: 'Task',
          shape: 'box',
          prompt: 'Do work',
          x_node_extension: 'node-extra',
        },
      },
    ]
    const edges: Edge[] = [
      {
        id: 'edge-start-task',
        source: 'start',
        target: 'task',
        data: {
          label: 'next',
          x_edge_extension: 'edge-extra',
        },
      },
    ]

    renderSidebar(nodes, edges)

    const nodeEditor = await screen.findByTestId('node-extension-attrs-editor')
    expect(within(nodeEditor).getByDisplayValue('x_node_extension')).toBeVisible()
    expect(within(nodeEditor).getByTestId('node-extension-attr-value-0')).toBeVisible()
    expect(within(nodeEditor).getByTestId('node-extension-attr-new-key')).toBeVisible()
    expect(within(nodeEditor).getByTestId('node-extension-attr-new-value')).toBeVisible()
    expect(within(nodeEditor).getByRole('button', { name: 'Add Attribute' })).toBeVisible()

    act(() => {
      useStore.getState().setSelectedNodeId(null)
      useStore.getState().setSelectedEdgeId('edge-start-task')
    })

    const edgeEditor = await screen.findByTestId('edge-extension-attrs-editor')
    expect(within(edgeEditor).getByDisplayValue('x_edge_extension')).toBeVisible()

    cleanup()
    act(() => {
      resetContractState()
      useStore.getState().setGraphAttrs({
        goal: 'Release',
        x_graph_extension: 'graph-extra',
      } as never)
    })

    renderWithFlowProvider(<GraphSettings inline />)
    await user.click(screen.getByTestId('graph-advanced-toggle'))
    const graphEditor = await screen.findByTestId('graph-extension-attrs-editor')
    expect(within(graphEditor).getByDisplayValue('x_graph_extension')).toBeVisible()
  })

  it('[CID:11.4.02] preserves unknown-valid attrs on graph save operations without pre-edit autosave', async () => {
    const savePayloads: Array<{ name: string; content: string }> = []
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.endsWith('/attractor/api/flows') && init?.method === 'POST') {
        const payload = JSON.parse(String(init.body)) as { name: string; content: string }
        savePayloads.push(payload)
        return jsonResponse({ status: 'saved' })
      }
      return jsonResponse({}, { status: 404 })
    })
    vi.stubGlobal('fetch', fetchMock)

    act(() => {
      useStore.getState().setGraphAttrs({
        goal: 'Release',
        x_graph_extension: 'graph-extra',
      } as never)
    })

    const nodes: Node[] = [
      {
        id: 'start',
        position: { x: 0, y: 0 },
        data: { label: 'Start', shape: 'Mdiamond' },
      },
      {
        id: 'task',
        position: { x: 180, y: 0 },
        data: {
          label: 'Task',
          shape: 'box',
          x_node_extension: 'node-extra',
        },
      },
    ]
    const edges: Edge[] = [
      {
        id: 'edge-start-task',
        source: 'start',
        target: 'task',
        data: {
          label: 'next',
          x_edge_extension: 'edge-extra',
        },
      },
    ]

    renderGraphSettings(nodes, edges)
    await screen.findByTestId('graph-structured-form')

    await new Promise((resolve) => window.setTimeout(resolve, 275))
    expect(savePayloads).toHaveLength(0)

    fireEvent.change(screen.getByDisplayValue('Release'), { target: { value: 'Ship now' } })
    await waitFor(() => {
      expect(savePayloads).toHaveLength(1)
    })

    const savedDot = savePayloads[0].content
    expect(savedDot).toContain('x_graph_extension="graph-extra"')
    expect(savedDot).toContain('x_node_extension="node-extra"')
    expect(savedDot).toContain('x_edge_extension="edge-extra"')
  })

  it('[CID:11.4.03] keeps numeric extension attrs stable across repeated structured edits', async () => {
    const savePayloads: Array<{ name: string; content: string }> = []
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.endsWith('/attractor/api/flows') && init?.method === 'POST') {
        const payload = JSON.parse(String(init.body)) as { name: string; content: string }
        savePayloads.push(payload)
        return jsonResponse({ status: 'saved' })
      }
      return jsonResponse({}, { status: 404 })
    })
    vi.stubGlobal('fetch', fetchMock)

    act(() => {
      useStore.getState().setGraphAttrs({
        goal: 'Release',
        label: 'Milestone',
        x_graph_extension_number: 17,
      } as never)
    })

    const nodes: Node[] = [
      {
        id: 'start',
        position: { x: 0, y: 0 },
        data: { label: 'Start', shape: 'Mdiamond' },
      },
      {
        id: 'task',
        position: { x: 180, y: 0 },
        data: {
          label: 'Task',
          shape: 'box',
          x_node_extension_number: 23,
        },
      },
    ]
    const edges: Edge[] = [
      {
        id: 'edge-start-task',
        source: 'start',
        target: 'task',
        data: {
          label: 'next',
          x_edge_extension_number: 29,
        },
      },
    ]

    renderGraphSettings(nodes, edges)
    await screen.findByTestId('graph-structured-form')

    await new Promise((resolve) => window.setTimeout(resolve, 275))
    expect(savePayloads).toHaveLength(0)

    fireEvent.change(screen.getByDisplayValue('Release'), { target: { value: 'Ship now' } })
    await waitFor(() => {
      expect(savePayloads).toHaveLength(1)
    })

    fireEvent.change(screen.getByDisplayValue('Milestone'), { target: { value: 'Milestone 2' } })
    await waitFor(() => {
      expect(savePayloads).toHaveLength(2)
    })

    savePayloads.forEach(({ content }) => {
      expect(content).toContain('x_graph_extension_number=17')
      expect(content).toContain('x_node_extension_number=23')
      expect(content).toContain('x_edge_extension_number=29')
    })
  })

  it('[CID:10.3.01] exposes human.default_choice authoring and timeout-default visibility in node inspector', async () => {
    act(() => {
      useStore.getState().setSelectedNodeId('gate')
      useStore.getState().setSelectedEdgeId(null)
    })

    const nodes: Node[] = [
      {
        id: 'task',
        position: { x: 0, y: 0 },
        data: { label: 'Task', shape: 'box', type: 'codergen', prompt: 'Do work' },
      },
      {
        id: 'gate',
        position: { x: 150, y: 0 },
        data: {
          label: 'Gate',
          shape: 'hexagon',
          type: 'wait.human',
          prompt: 'Choose path',
          'human.default_choice': 'fix',
        },
      },
    ]

    renderSidebar(nodes, [])

    const defaultChoiceInput = await screen.findByDisplayValue('fix')
    expect(defaultChoiceInput).toBeVisible()
    expect(defaultChoiceInput).toHaveAttribute('placeholder', 'target node id')
    expect(defaultChoiceInput).toBeEnabled()
    expect(screen.getByText('Used when this gate times out without an explicit answer.')).toBeVisible()

    act(() => {
      useStore.getState().setSelectedNodeId('task')
    })

    await waitFor(() => {
      expect(screen.queryByText('Human Default Choice')).not.toBeInTheDocument()
    })

    act(() => {
      useStore.getState().setSelectedNodeId('gate')
    })

    await waitFor(() => {
      expect(screen.getByDisplayValue('fix')).toBeVisible()
    })
  })

  it('[CID:11.5.01] ignores browser-persisted project and workflow state that now belongs to Spark storage', async () => {
    vi.resetModules()
    const storage = new Map<string, string>()
    const localStorageMock = {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        storage.set(key, value)
      },
      removeItem: (key: string) => {
        storage.delete(key)
      },
      clear: () => {
        storage.clear()
      },
    }
    vi.stubGlobal('localStorage', localStorageMock)
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      value: localStorageMock,
    })

    localStorageMock.setItem('spark.project_registry_state', JSON.stringify({
      '/tmp/persisted-project': {
        directoryPath: '/tmp/persisted-project',
        isFavorite: true,
        lastAccessedAt: '2026-03-04T00:00:00.000Z',
      },
    }))
    localStorageMock.setItem('spark.project_conversation_state', JSON.stringify({
      '/tmp/persisted-project': {
        conversationId: 'conversation-persisted-project',
        specId: 'spec-persisted-project',
        specStatus: 'approved',
        planId: 'plan-persisted-project',
        planStatus: 'approved',
      },
    }))
    localStorageMock.setItem('spark.ui_route_state', JSON.stringify({
      viewMode: 'projects',
      activeProjectPath: '/tmp/persisted-project',
      selectedRunId: null,
    }))

    const { useStore: restoredStore } = await import('@/store')
    const restoredState = restoredStore.getState()

    expect(restoredState.projectRegistry).toEqual({})
    expect(restoredState.projectSessionsByPath['/tmp/persisted-project']?.conversationId).toBeNull()
    expect(restoredState.projectSessionsByPath['/tmp/persisted-project']?.specId).toBeNull()
    expect(restoredState.projectSessionsByPath['/tmp/persisted-project']?.planId).toBeNull()
    expect(restoredState.activeProjectPath).toBe('/tmp/persisted-project')
  })

  it('[CID:11.5.02] hydrates backend-owned project registry and active conversation linkage into in-memory UI state', async () => {
    vi.resetModules()
    const { useStore: restoredStore } = await import('@/store')

    restoredStore.getState().hydrateProjectRegistry([
      {
        directoryPath: '/tmp/persisted-project',
        isFavorite: true,
        lastAccessedAt: '2026-03-04T00:00:00.000Z',
        activeConversationId: 'conversation-persisted-project',
      },
    ])

    expect(restoredStore.getState().projectRegistry['/tmp/persisted-project']).toEqual({
      directoryPath: '/tmp/persisted-project',
      isFavorite: true,
      lastAccessedAt: '2026-03-04T00:00:00.000Z',
    })
    expect(restoredStore.getState().projectSessionsByPath['/tmp/persisted-project']?.conversationId).toBe(
      'conversation-persisted-project',
    )
  })

})
