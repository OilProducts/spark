import { ExecutionControls } from '@/components/ExecutionControls'
import { Editor } from '@/components/Editor'
import { GraphSettings } from '@/components/GraphSettings'
import { RunsPanel } from '@/components/RunsPanel'
import { Sidebar } from '@/components/Sidebar'
import { TaskNode } from '@/components/TaskNode'
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
    selectedRunId: null,
    workingDir: DEFAULT_WORKING_DIRECTORY,
    projectRegistry: {
      '/tmp/project-contract-behavior': {
        directoryPath: '/tmp/project-contract-behavior',
        isFavorite: false,
        lastAccessedAt: null,
      },
    },
    projectScopedWorkspaces: {
      '/tmp/project-contract-behavior': {
        activeFlow: 'contract-behavior.dot',
        selectedRunId: null,
        workingDir: DEFAULT_WORKING_DIRECTORY,
        conversationId: null,
        conversationHistory: [],
        specId: null,
        specStatus: 'draft',
        planId: null,
        planStatus: 'draft',
        artifactRunId: null,
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
    saveState: 'idle',
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

const TaskNodeHarness = ({ nodes, edges = [] }: { nodes: Node[]; edges?: Edge[] }) => (
  <div style={{ width: 800, height: 600 }}>
    <ReactFlow nodes={nodes} edges={edges} nodeTypes={{ task: TaskNode }} fitView />
  </div>
)

const renderTaskNode = (node: Node) => renderWithFlowProvider(<TaskNodeHarness nodes={[node]} />)

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
          'manager.stop_condition': 'child.status == "success"',
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
      vi.fn(async () =>
        new Response(JSON.stringify(['contract-behavior.dot']), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      ),
    )
  })

  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
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
    const preHookInput = screen.getByTestId('graph-attr-input-tool_hooks.pre')
    const postHookInput = screen.getByTestId('graph-attr-input-tool_hooks.post')
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
      tool_command: 'echo run',
      'tool_hooks.pre': 'echo node pre',
      'tool_hooks.post': 'echo node post',
    }
    renderSidebar([
      {
        id: 'tool_node',
        position: { x: 0, y: 0 },
        data: toolNodeData,
      },
    ], [])

    await user.click(await screen.findByRole('button', { name: 'Show Advanced' }))
    expect(screen.getByTestId('node-attr-input-tool_hooks.pre')).toBeVisible()
    expect(screen.getByTestId('node-attr-input-tool_hooks.post')).toBeVisible()

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

    expect(screen.getByTestId('node-toolbar-attr-input-tool_hooks.pre')).toBeVisible()
    expect(screen.getByTestId('node-toolbar-attr-input-tool_hooks.post')).toBeVisible()
  })

  it('[CID:6.6.03] renders tool hook warning surfaces in graph settings and node editors', async () => {
    const user = userEvent.setup()
    renderWithFlowProvider(<GraphSettings inline />)

    await user.click(screen.getByTestId('graph-advanced-toggle'))
    fireEvent.change(screen.getByTestId('graph-attr-input-tool_hooks.pre'), { target: { value: "echo 'unterminated" } })
    fireEvent.change(screen.getByTestId('graph-attr-input-tool_hooks.post'), { target: { value: 'echo "unterminated' } })

    await waitFor(() => {
      expect(screen.getByTestId('graph-attr-warning-tool_hooks.pre')).toHaveTextContent('single quote')
      expect(screen.getByTestId('graph-attr-warning-tool_hooks.post')).toHaveTextContent('double quote')
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
      tool_command: 'echo run',
      'tool_hooks.pre': 'echo hi\necho there',
      'tool_hooks.post': "echo 'unterminated",
    }
    renderSidebar([
      {
        id: 'tool_node',
        position: { x: 0, y: 0 },
        data: toolNodeData,
      },
    ], [])

    await user.click(await screen.findByRole('button', { name: 'Show Advanced' }))
    expect(screen.getByTestId('node-attr-warning-tool_hooks.pre')).toHaveTextContent('single line')
    expect(screen.getByTestId('node-attr-warning-tool_hooks.post')).toHaveTextContent('single quote')

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

    expect(screen.getByTestId('node-toolbar-attr-input-tool_hooks.pre')).toBeVisible()
    expect(screen.getByTestId('node-toolbar-attr-input-tool_hooks.post')).toBeVisible()
    expect(screen.getByTestId('node-toolbar-attr-warning-tool_hooks.pre')).toHaveTextContent('single line')
    expect(screen.getByTestId('node-toolbar-attr-warning-tool_hooks.post')).toHaveTextContent('single quote')
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
        'manager.stop_condition': 'child.status == "success"',
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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const answerPath = `${runApiPath}/questions/${encodeURIComponent(gateId)}/answer`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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

    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const answerPath = `${runApiPath}/questions/${encodeURIComponent(gateId)}/answer`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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

    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const runRecord = {
      run_id: runId,
      flow_name: 'contract-behavior.dot',
      status: 'running',
      result: 'running',
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
      if (url.endsWith('/api/flows/contract-behavior.dot')) {
        return Promise.resolve(jsonResponse({ content: initialDot }))
      }
      if (url.endsWith('/preview')) {
        return Promise.resolve(jsonResponse(previewPayload))
      }
      if (url.endsWith('/api/flows') && init?.method === 'POST') {
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

    const structuredButton = screen.getByRole('button', { name: 'Structured' })
    fireEvent.click(structuredButton)
    fireEvent.click(structuredButton)

    await waitFor(() => {
      const saveCalls = fetchMock.mock.calls.filter(([input, requestInit]) => {
        const callUrl = requestUrl(input as RequestInfo | URL)
        return callUrl.endsWith('/api/flows') && (requestInit as RequestInit | undefined)?.method === 'POST'
      })
      expect(saveCalls).toHaveLength(1)
    })

    expect(saveResolvers).toHaveLength(1)
    saveResolvers[0]()

    await waitFor(() => {
      expect(screen.queryByTestId('raw-dot-editor')).not.toBeInTheDocument()
    })
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
      if (url.endsWith('/api/flows/contract-behavior.dot')) {
        return jsonResponse({ content: initialDot })
      }
      if (url.endsWith('/preview')) {
        return jsonResponse(previewPayload)
      }
      if (url.endsWith('/api/flows') && init?.method === 'POST') {
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
      expect(savedPayloads.length).toBeGreaterThanOrEqual(2)
    })

    const rawHandoffSave = savedPayloads[savedPayloads.length - 1].content
    expect(rawHandoffSave).toContain('x_unsurfaced_node_default="keep-node-default"')
    expect(rawHandoffSave).toContain('x_unsurfaced_edge_default="keep-edge-default"')
    expect(rawHandoffSave).toContain('subgraph cluster_review {')
    expect(rawHandoffSave).toContain('x_unsurfaced_node="keep-node"')
    expect(rawHandoffSave).toContain('x_unsurfaced_edge="keep-edge"')
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
      if (url.endsWith('/api/flows/contract-behavior.dot')) {
        return jsonResponse({ content: initialDot })
      }
      if (url.endsWith('/preview')) {
        const payload = previewRequestCount === 0 ? previewOkPayload : previewConflictPayload
        previewRequestCount += 1
        return jsonResponse(payload)
      }
      if (url.endsWith('/api/flows') && init?.method === 'POST') {
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
})
