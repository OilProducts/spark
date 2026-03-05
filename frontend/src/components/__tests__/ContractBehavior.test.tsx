import { ExecutionControls } from '@/components/ExecutionControls'
import { Editor } from '@/components/Editor'
import { GraphSettings } from '@/components/GraphSettings'
import { Navbar } from '@/components/Navbar'
import { ProjectsPanel } from '@/components/ProjectsPanel'
import { RunStream } from '@/components/RunStream'
import { RunsPanel } from '@/components/RunsPanel'
import { SettingsPanel } from '@/components/SettingsPanel'
import { Sidebar } from '@/components/Sidebar'
import { TaskNode } from '@/components/TaskNode'
import { ValidationPanel } from '@/components/ValidationPanel'
import {
  ApiHttpError,
  ApiSchemaError,
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
  fetchRunsListValidated,
  fetchRuntimeStatusValidated,
  parseFlowListResponse,
  parseFlowPayloadResponse,
  parsePipelineGraphResponse,
  parsePipelineStatusResponse,
  parsePreviewResponse,
  parseRuntimeStatusResponse,
} from '@/lib/apiClient'
import { buildPipelineStartPayload } from '@/lib/pipelineStartPayload'
import { useStore } from '@/store'
import { ReactFlow, ReactFlowProvider, type Edge, type Node } from '@xyflow/react'
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
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

const collectRuntimeSourceFiles = (directoryPath: string): string[] => {
  const entries = readdirSync(directoryPath, { withFileTypes: true })
  const files: string[] = []
  for (const entry of entries) {
    if (entry.name === '__tests__' || entry.name === 'test') {
      continue
    }
    const entryPath = join(directoryPath, entry.name)
    if (entry.isDirectory()) {
      files.push(...collectRuntimeSourceFiles(entryPath))
      continue
    }
    if (!entry.isFile()) {
      continue
    }
    if (!entry.name.endsWith('.ts') && !entry.name.endsWith('.tsx')) {
      continue
    }
    files.push(entryPath)
  }
  return files
}

const resolveFrontendSrcRoot = (): string => {
  const currentWorkingDirectory = process.cwd()
  const directSrcPath = join(currentWorkingDirectory, 'src')
  if (existsSync(directSrcPath)) {
    return directSrcPath
  }
  const nestedSrcPath = join(currentWorkingDirectory, 'frontend', 'src')
  if (existsSync(nestedSrcPath)) {
    return nestedSrcPath
  }
  throw new Error(`Unable to locate frontend src directory from cwd: ${currentWorkingDirectory}`)
}

const readRuntimeUiSource = (): string => {
  const srcRoot = resolveFrontendSrcRoot()
  const files = [
    ...collectRuntimeSourceFiles(join(srcRoot, 'components')),
    ...collectRuntimeSourceFiles(join(srcRoot, 'lib')),
  ]
  return files
    .sort((left, right) => left.localeCompare(right))
    .map((filePath) => readFileSync(filePath, 'utf-8'))
    .join('\n')
}

const readFrontendIndexCss = (): string => {
  const srcRoot = resolveFrontendSrcRoot()
  const indexCssPath = join(srcRoot, 'index.css')
  if (!existsSync(indexCssPath)) {
    throw new Error(`Unable to locate frontend index.css at ${indexCssPath}`)
  }
  return readFileSync(indexCssPath, 'utf-8')
}

const parseRootHslToken = (cssSource: string, tokenName: string): [number, number, number] => {
  const tokenPattern = new RegExp(`--${tokenName}:\\s*([\\d.]+)\\s+([\\d.]+)%\\s+([\\d.]+)%\\s*;`)
  const tokenMatch = cssSource.match(tokenPattern)
  if (!tokenMatch) {
    throw new Error(`Unable to find --${tokenName} token in frontend index.css`)
  }
  return [Number(tokenMatch[1]), Number(tokenMatch[2]), Number(tokenMatch[3])]
}

const hslToRgb = ([h, s, l]: [number, number, number]): [number, number, number] => {
  const hue = ((h % 360) + 360) % 360
  const saturation = Math.max(0, Math.min(100, s)) / 100
  const lightness = Math.max(0, Math.min(100, l)) / 100
  const chroma = (1 - Math.abs((2 * lightness) - 1)) * saturation
  const hueSegment = hue / 60
  const secondary = chroma * (1 - Math.abs((hueSegment % 2) - 1))
  let redPrime = 0
  let greenPrime = 0
  let bluePrime = 0

  if (hueSegment >= 0 && hueSegment < 1) {
    redPrime = chroma
    greenPrime = secondary
  } else if (hueSegment >= 1 && hueSegment < 2) {
    redPrime = secondary
    greenPrime = chroma
  } else if (hueSegment >= 2 && hueSegment < 3) {
    greenPrime = chroma
    bluePrime = secondary
  } else if (hueSegment >= 3 && hueSegment < 4) {
    greenPrime = secondary
    bluePrime = chroma
  } else if (hueSegment >= 4 && hueSegment < 5) {
    redPrime = secondary
    bluePrime = chroma
  } else {
    redPrime = chroma
    bluePrime = secondary
  }

  const match = lightness - chroma / 2
  const toByte = (channel: number): number => Math.round((channel + match) * 255)
  return [toByte(redPrime), toByte(greenPrime), toByte(bluePrime)]
}

const blendOnWhite = (
  [red, green, blue]: [number, number, number],
  alpha: number,
): [number, number, number] => {
  const normalizedAlpha = Math.max(0, Math.min(1, alpha))
  const blendChannel = (channel: number): number =>
    Math.round((normalizedAlpha * channel) + ((1 - normalizedAlpha) * 255))
  return [blendChannel(red), blendChannel(green), blendChannel(blue)]
}

const contrastRatio = (
  [redA, greenA, blueA]: [number, number, number],
  [redB, greenB, blueB]: [number, number, number],
): number => {
  const toLinear = (channel: number): number => {
    const srgb = channel / 255
    return srgb <= 0.03928 ? srgb / 12.92 : ((srgb + 0.055) / 1.055) ** 2.4
  }
  const luminance = ([red, green, blue]: [number, number, number]): number =>
    (0.2126 * toLinear(red)) + (0.7152 * toLinear(green)) + (0.0722 * toLinear(blue))
  const lighter = Math.max(luminance([redA, greenA, blueA]), luminance([redB, greenB, blueB]))
  const darker = Math.min(luminance([redA, greenA, blueA]), luminance([redB, greenB, blueB]))
  return (lighter + 0.05) / (darker + 0.05)
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
  Object.defineProperty(window, 'innerWidth', {
    configurable: true,
    writable: true,
    value: width,
  })
  window.dispatchEvent(new Event('resize'))
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

  it('[CID:12.1.01] verifies runtime UI code covers every required API endpoint from ui-spec section 12.1', () => {
    const runtimeSource = readRuntimeUiSource()
    const requiredEndpointPatterns: Array<{ endpoint: string; pattern: RegExp }> = [
      { endpoint: '/api/flows', pattern: /fetch\(\s*['"]\/api\/flows['"]|fetchFlowListValidated\(/ },
      { endpoint: '/api/flows/{name}', pattern: /fetch\(\s*`\/api\/flows\/\$\{encodeURIComponent\([^)]+\)\}`|fetchFlowPayloadValidated\(/ },
      { endpoint: '/preview', pattern: /fetch\(\s*['"]\/preview['"]|fetchPreviewValidated\(/ },
      { endpoint: '/pipelines', pattern: /fetch\(\s*['"]\/pipelines['"]|fetchPipelineStartValidated\(/ },
      { endpoint: '/pipelines/{id}', pattern: /fetch\(\s*`\/pipelines\/\$\{encodeURIComponent\([^)]+\)\}`\s*(?:,|\))|fetchPipelineStatusValidated\(/ },
      { endpoint: '/pipelines/{id}/events', pattern: /new EventSource\(\s*`\/pipelines\/\$\{encodeURIComponent\([^)]+\)\}\/events`/ },
      { endpoint: '/pipelines/{id}/cancel', pattern: /fetch\(\s*`\/pipelines\/\$\{encodeURIComponent\([^)]+\)\}\/cancel`\s*,\s*\{\s*method:\s*['"]POST['"]|fetchPipelineCancelValidated\(/ },
      { endpoint: '/pipelines/{id}/graph', pattern: /fetch\(\s*`\/pipelines\/\$\{encodeURIComponent\([^)]+\)\}\/graph`|fetchPipelineGraphValidated\(/ },
      { endpoint: '/pipelines/{id}/questions', pattern: /fetch\(\s*`\/pipelines\/\$\{encodeURIComponent\([^)]+\)\}\/questions`\s*(?:,|\))|fetchPipelineQuestionsValidated\(/ },
      { endpoint: '/pipelines/{id}/questions/{qid}/answer', pattern: /fetch\(\s*`\/pipelines\/\$\{encodeURIComponent\([^)]+\)\}\/questions\/\$\{encodeURIComponent\([^)]+\)\}\/answer`|fetchPipelineAnswerValidated\(/ },
      { endpoint: '/pipelines/{id}/checkpoint', pattern: /fetch\(\s*`\/pipelines\/\$\{encodeURIComponent\([^)]+\)\}\/checkpoint`|fetchPipelineCheckpointValidated\(/ },
      { endpoint: '/pipelines/{id}/context', pattern: /fetch\(\s*`\/pipelines\/\$\{encodeURIComponent\([^)]+\)\}\/context`|fetchPipelineContextValidated\(/ },
      { endpoint: '/runs', pattern: /fetch\(\s*['"]\/runs['"]|fetchRunsListValidated\(/ },
      { endpoint: '/status', pattern: /fetch\(\s*['"]\/status['"]|fetchRuntimeStatusValidated\(/ },
    ]

    const missingEndpoints = requiredEndpointPatterns
      .filter(({ pattern }) => !pattern.test(runtimeSource))
      .map(({ endpoint }) => endpoint)

    expect(missingEndpoints).toEqual([])
  })

  it('[CID:12.1.02] provides typed endpoint adapters with runtime schema validation for required JSON responses', () => {
    const runtimeSource = readRuntimeUiSource()
    const requiredPatterns: Array<{ requirement: string; pattern: RegExp }> = [
      { requirement: 'API schema error type', pattern: /class\s+ApiSchemaError\s+extends\s+Error/ },
      { requirement: 'schema assertion helper', pattern: /function\s+expectObjectRecord\s*\(/ },
      { requirement: 'flows list response validator', pattern: /function\s+parseFlowListResponse\s*\(/ },
      { requirement: 'flow payload response validator', pattern: /function\s+parseFlowPayloadResponse\s*\(/ },
      { requirement: 'preview response validator', pattern: /function\s+parsePreviewResponse\s*\(/ },
      { requirement: 'pipeline start response validator', pattern: /function\s+parsePipelineStartResponse\s*\(/ },
      { requirement: 'pipeline status response validator', pattern: /function\s+parsePipelineStatusResponse\s*\(/ },
      { requirement: 'pipeline cancel response validator', pattern: /function\s+parsePipelineCancelResponse\s*\(/ },
      { requirement: 'pipeline checkpoint response validator', pattern: /function\s+parsePipelineCheckpointResponse\s*\(/ },
      { requirement: 'pipeline context response validator', pattern: /function\s+parsePipelineContextResponse\s*\(/ },
      { requirement: 'pipeline questions response validator', pattern: /function\s+parsePipelineQuestionsResponse\s*\(/ },
      { requirement: 'pipeline answer response validator', pattern: /function\s+parsePipelineAnswerResponse\s*\(/ },
      { requirement: 'pipeline graph response validator', pattern: /function\s+parsePipelineGraphResponse\s*\(/ },
      { requirement: 'runs list response validator', pattern: /function\s+parseRunsListResponse\s*\(/ },
      { requirement: 'runtime status response validator', pattern: /function\s+parseRuntimeStatusResponse\s*\(/ },
      { requirement: 'validated flows adapter', pattern: /function\s+fetchFlowListValidated\s*\(/ },
      { requirement: 'validated flow adapter', pattern: /function\s+fetchFlowPayloadValidated\s*\(/ },
      { requirement: 'validated preview adapter', pattern: /function\s+fetchPreviewValidated\s*\(/ },
      { requirement: 'validated pipeline start adapter', pattern: /function\s+fetchPipelineStartValidated\s*\(/ },
      { requirement: 'validated pipeline status adapter', pattern: /function\s+fetchPipelineStatusValidated\s*\(/ },
      { requirement: 'validated pipeline cancel adapter', pattern: /function\s+fetchPipelineCancelValidated\s*\(/ },
      { requirement: 'validated pipeline checkpoint adapter', pattern: /function\s+fetchPipelineCheckpointValidated\s*\(/ },
      { requirement: 'validated pipeline context adapter', pattern: /function\s+fetchPipelineContextValidated\s*\(/ },
      { requirement: 'validated pipeline questions adapter', pattern: /function\s+fetchPipelineQuestionsValidated\s*\(/ },
      { requirement: 'validated pipeline answer adapter', pattern: /function\s+fetchPipelineAnswerValidated\s*\(/ },
      { requirement: 'validated pipeline graph adapter', pattern: /function\s+fetchPipelineGraphValidated\s*\(/ },
      { requirement: 'validated runs list adapter', pattern: /function\s+fetchRunsListValidated\s*\(/ },
      { requirement: 'validated runtime status adapter', pattern: /function\s+fetchRuntimeStatusValidated\s*\(/ },
    ]

    const missingRequirements = requiredPatterns
      .filter(({ pattern }) => !pattern.test(runtimeSource))
      .map(({ requirement }) => requirement)

    expect(missingRequirements).toEqual([])

    expect(parseFlowListResponse(['a.dot', 'b.dot'])).toEqual(['a.dot', 'b.dot'])
    expect(() => parseFlowListResponse({})).toThrow(ApiSchemaError)
    expect(parseFlowPayloadResponse({ content: 'digraph G {}' })).toEqual({ name: '', content: 'digraph G {}' })
    expect(() => parseFlowPayloadResponse({})).toThrow(ApiSchemaError)
    expect(parsePreviewResponse({ graph: { nodes: [], edges: [] } }).status).toBe('ok')
    expect(() => parsePreviewResponse({ graph: { nodes: {} } })).toThrow(ApiSchemaError)
    expect(parsePipelineStatusResponse({ pipeline_id: 'run-1', status: 'running' }).pipeline_id).toBe('run-1')
    expect(() => parsePipelineStatusResponse({ status: 'running' })).toThrow(ApiSchemaError)
    expect(parseRuntimeStatusResponse({ status: 'idle' }).status).toBe('idle')
    expect(() => parseRuntimeStatusResponse({})).toThrow(ApiSchemaError)
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
        name: 'flow list',
        invoke: () => fetchFlowListValidated(),
        expectedUrl: '/api/flows',
        response: jsonResponse(['alpha.dot']),
        assertResult: (result) => expect(result).toEqual(['alpha.dot']),
      },
      {
        name: 'flow payload',
        invoke: () => fetchFlowPayloadValidated('alpha flow.dot'),
        expectedUrl: '/api/flows/alpha%20flow.dot',
        response: jsonResponse({ name: 'alpha flow.dot', content: 'digraph G {}' }),
        assertResult: (result) =>
          expect(result).toEqual({
            name: 'alpha flow.dot',
            content: 'digraph G {}',
          }),
      },
      {
        name: 'preview',
        invoke: () => fetchPreviewValidated('digraph G {}'),
        expectedUrl: '/preview',
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
        expectedUrl: '/pipelines',
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
        expectedUrl: '/pipelines/run%20status',
        response: jsonResponse({ pipeline_id: 'run status', status: 'running' }),
        assertResult: (result) => expect(result).toMatchObject({ pipeline_id: 'run status', status: 'running' }),
      },
      {
        name: 'pipeline cancel',
        invoke: () => fetchPipelineCancelValidated('run cancel'),
        expectedUrl: '/pipelines/run%20cancel/cancel',
        expectedMethod: 'POST',
        response: jsonResponse({ status: 'accepted', pipeline_id: 'run cancel' }),
        assertResult: (result) => expect(result).toMatchObject({ status: 'accepted', pipeline_id: 'run cancel' }),
      },
      {
        name: 'pipeline graph',
        invoke: () => fetchPipelineGraphValidated('run graph'),
        expectedUrl: '/pipelines/run%20graph/graph',
        response: new Response('<svg><g /></svg>', { status: 200 }),
        assertResult: (result) => expect(result).toContain('<svg>'),
      },
      {
        name: 'pipeline questions',
        invoke: () => fetchPipelineQuestionsValidated('run questions'),
        expectedUrl: '/pipelines/run%20questions/questions',
        response: jsonResponse({ questions: [{ question_id: 'q-1' }] }),
        assertResult: (result) => expect(result).toEqual({ questions: [{ question_id: 'q-1' }] }),
      },
      {
        name: 'pipeline answer',
        invoke: () => fetchPipelineAnswerValidated('run answer', 'q 1', 'approve'),
        expectedUrl: '/pipelines/run%20answer/questions/q%201/answer',
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
        expectedUrl: '/pipelines/run%20checkpoint/checkpoint',
        response: jsonResponse({ pipeline_id: 'run checkpoint', checkpoint: { node: 'n-1' } }),
        assertResult: (result) =>
          expect(result).toEqual({ pipeline_id: 'run checkpoint', checkpoint: { node: 'n-1' } }),
      },
      {
        name: 'pipeline context',
        invoke: () => fetchPipelineContextValidated('run context'),
        expectedUrl: '/pipelines/run%20context/context',
        response: jsonResponse({ pipeline_id: 'run context', context: { branch: 'main' } }),
        assertResult: (result) =>
          expect(result).toEqual({ pipeline_id: 'run context', context: { branch: 'main' } }),
      },
      {
        name: 'runs list',
        invoke: () => fetchRunsListValidated(),
        expectedUrl: '/runs',
        response: jsonResponse({ runs: [{ run_id: 'run-1', status: 'running' }] }),
        assertResult: (result) => expect(result).toEqual({ runs: [{ run_id: 'run-1', status: 'running', flow_name: '', working_directory: '', model: '', started_at: '', result: undefined, project_path: undefined, git_branch: undefined, git_commit: undefined, spec_id: undefined, plan_id: undefined, ended_at: undefined, last_error: undefined, token_usage: undefined }] }),
      },
      {
        name: 'runtime status',
        invoke: () => fetchRuntimeStatusValidated(),
        expectedUrl: '/status',
        response: jsonResponse({ status: 'idle' }),
        assertResult: (result) =>
          expect(result).toEqual({
            status: 'idle',
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
      endpoint: '/preview',
      status: 422,
      detail: 'Preview validation failed.',
    })

    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('Gateway timeout', { status: 504 })),
    )
    await expect(fetchPipelineStatusValidated('run-504')).rejects.toMatchObject<ApiHttpError>({
      endpoint: '/pipelines/{id}',
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
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.endsWith('/status')) {
          return jsonResponse({ runtime: 'idle' })
        }
        return jsonResponse({})
      }),
    )

    render(<RunStream />)

    await waitFor(() => {
      expect(screen.getByTestId('runtime-api-degraded-banner')).toBeVisible()
    })

    expect(screen.getByTestId('runtime-api-degraded-banner')).toHaveTextContent(
      'Runtime status endpoint is unavailable or incompatible.',
    )
    expect(screen.getByTestId('global-save-state-indicator')).toHaveTextContent('Idle')
  })

  it('[CID:12.2.02] keeps non-dependent run-inspector surfaces functional under partial API failure', async () => {
    const runId = 'run-contract-partial-failure'
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
        if (url.endsWith(`${runApiPath}/graph`)) {
          return new Response('<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10" /></svg>', {
            status: 200,
            headers: { 'Content-Type': 'image/svg+xml' },
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
    })

    render(<RunsPanel />)

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
      expect(screen.getByTestId('run-graphviz-viewer-image')).toBeVisible()
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
      if (url.endsWith('/api/flows/contract-behavior.dot')) {
        return jsonResponse({ content: initialDot })
      }
      if (url.endsWith('/preview')) {
        previewRequestCount += 1
        return jsonResponse(previewPayload)
      }
      if (url.endsWith('/api/flows') && init?.method === 'POST') {
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
      'sparkspawn.project_registry_state',
      JSON.stringify({
        '/tmp/project-contract-behavior': {
          directoryPath: '/tmp/project-contract-behavior',
          isFavorite: false,
          lastAccessedAt: null,
        },
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.ui_route_state',
      JSON.stringify({
        viewMode: 'projects',
        activeProjectPath: '/tmp/project-contract-behavior',
        activeFlow: null,
        selectedRunId: null,
      }),
    )

    const { useStore: restoredStore } = await import('@/store')
    restoredStore.getState().setActiveProjectPath('/tmp/project-contract-behavior/./')

    const nextState = restoredStore.getState()
    expect(nextState.activeProjectPath).toBe('/tmp/project-contract-behavior')
    expect(nextState.projectScopedWorkspaces['/tmp/project-contract-behavior']).toBeDefined()
    expect(nextState.projectScopedWorkspaces['/tmp/project-contract-behavior/./']).toBeUndefined()

    const persistedRouteStateRaw = localStorageMock.getItem('sparkspawn.ui_route_state')
    expect(persistedRouteStateRaw).toBeTruthy()
    const persistedRouteState = JSON.parse(String(persistedRouteStateRaw)) as { activeProjectPath: string | null }
    expect(persistedRouteState.activeProjectPath).toBe('/tmp/project-contract-behavior')
  })

  it('[CID:12.3.02] resolves execution payload working directory to concrete project-scoped path', () => {
    const relativeWorkingDirectoryPayload = buildPipelineStartPayload(
      {
        projectPath: '/tmp/project-contract-behavior',
        flowSource: 'contract-behavior.dot',
        workingDirectory: ' ./workspace/../build ',
        backend: 'codex',
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
        backend: 'codex',
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
      'sparkspawn.project_registry_state',
      JSON.stringify({
        '/tmp/project-a': {
          directoryPath: '/tmp/project-a',
          isFavorite: false,
          lastAccessedAt: null,
        },
        '/tmp/project-b': {
          directoryPath: '/tmp/project-b',
          isFavorite: false,
          lastAccessedAt: null,
        },
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.ui_route_state',
      JSON.stringify({
        viewMode: 'projects',
        activeProjectPath: '/tmp/project-a',
        activeFlow: null,
        selectedRunId: null,
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.project_conversation_state',
      JSON.stringify({
        '/tmp/project-a': {
          conversationId: 'conversation-a',
          conversationHistory: [],
          specId: 'spec-a',
          specStatus: 'approved',
          planId: 'plan-a',
          planStatus: 'rejected',
        },
        '/tmp/project-b/./': {
          conversationId: 'conversation-b',
          conversationHistory: [],
          specId: 'spec-b',
          specStatus: 'draft',
          planId: 'plan-b',
          planStatus: 'revision-requested',
        },
      }),
    )

    const { useStore: restoredStore } = await import('@/store')
    const projectAState = restoredStore.getState().getProjectScopedArtifactState('/tmp/project-a')
    expect(projectAState).toEqual({
      conversationId: 'conversation-a',
      specId: 'spec-a',
      specStatus: 'approved',
      planId: 'plan-a',
      planStatus: 'rejected',
    })

    const projectBState = restoredStore.getState().getProjectScopedArtifactState('/tmp/project-b/./')
    expect(projectBState).toEqual({
      conversationId: 'conversation-b',
      specId: 'spec-b',
      specStatus: 'draft',
      planId: 'plan-b',
      planStatus: 'revision-requested',
    })

    expect(restoredStore.getState().getProjectScopedArtifactState('/tmp/project-missing')).toBeNull()
  })

  it('[CID:12.4.03] integrates plan-generation invocation/status contract with degraded-state handling', async () => {
    const launchedRunId = 'run-plan-contract-12-4-03'
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.includes('/api/projects/metadata')) {
        return jsonResponse({ branch: 'main', commit: 'abc123def456' })
      }
      if (url.endsWith('/api/flows/contract-behavior.dot')) {
        return jsonResponse({ content: 'digraph Plan { start -> end }' })
      }
      if (url.endsWith('/pipelines') && init?.method === 'POST') {
        return jsonResponse({ status: 'started', pipeline_id: launchedRunId })
      }
      if (url.endsWith(`/pipelines/${launchedRunId}`)) {
        return jsonResponse({ detail: 'plan status endpoint unavailable' }, { status: 503 })
      }
      return jsonResponse({})
    })
    vi.stubGlobal('fetch', fetchMock)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'projects',
        activeProjectPath: '/tmp/project-contract-behavior',
        activeFlow: 'contract-behavior.dot',
        projectScopedWorkspaces: {
          ...state.projectScopedWorkspaces,
          '/tmp/project-contract-behavior': {
            ...state.projectScopedWorkspaces['/tmp/project-contract-behavior'],
            specId: 'spec-contract-behavior',
            specStatus: 'approved',
          },
        },
      }))
    })

    const user = userEvent.setup()
    render(<ProjectsPanel />)

    await user.click(screen.getByTestId('project-plan-generation-launch-button'))

    await waitFor(() => {
      expect(useStore.getState().selectedRunId).toBe(launchedRunId)
    })

    const requestedUrls = fetchMock.mock.calls.map(([input]) => requestUrl(input as RequestInfo | URL))
    expect(requestedUrls).toContain(`/pipelines/${launchedRunId}`)
    expect(screen.getByTestId('project-plan-generation-status-degraded')).toHaveTextContent(
      'plan status endpoint unavailable',
    )
    expect(useStore.getState().projectScopedWorkspaces['/tmp/project-contract-behavior']?.planStatus).toBe('draft')
    expect(useStore.getState().viewMode).toBe('execution')
  })

  it('[CID:12.4.04] integrates plan approval/rejection/revision transition contract', async () => {
    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'projects',
        activeProjectPath: '/tmp/project-contract-behavior',
        projectScopedWorkspaces: {
          ...state.projectScopedWorkspaces,
          '/tmp/project-contract-behavior': {
            ...state.projectScopedWorkspaces['/tmp/project-contract-behavior'],
            planId: 'plan-contract-behavior',
            planStatus: 'draft',
            conversationHistory: [],
          },
        },
      }))
    })

    const user = userEvent.setup()
    render(<ProjectsPanel />)

    const gateSurface = screen.getByTestId('project-plan-gate-surface')
    const approveButton = screen.getByTestId('project-plan-approve-button')
    const rejectButton = screen.getByTestId('project-plan-reject-button')
    const requestRevisionButton = screen.getByTestId('project-plan-request-revision-button')

    expect(gateSurface).toHaveTextContent('Plan status: draft')
    expect(approveButton).toBeEnabled()
    expect(rejectButton).toBeEnabled()
    expect(requestRevisionButton).toBeEnabled()

    await user.click(approveButton)
    await waitFor(() => {
      expect(useStore.getState().projectScopedWorkspaces['/tmp/project-contract-behavior']?.planStatus).toBe('approved')
    })
    expect(screen.getByTestId('project-plan-gate-surface')).toHaveTextContent('Plan status: approved')
    expect(screen.getByTestId('project-plan-approve-button')).toBeDisabled()
    expect(screen.getByTestId('project-plan-reject-button')).toBeEnabled()
    expect(screen.getByTestId('project-plan-request-revision-button')).toBeEnabled()

    await user.click(screen.getByTestId('project-plan-reject-button'))
    await waitFor(() => {
      expect(useStore.getState().projectScopedWorkspaces['/tmp/project-contract-behavior']?.planStatus).toBe('rejected')
    })
    expect(screen.getByTestId('project-plan-gate-surface')).toHaveTextContent('Plan status: rejected')

    await user.click(screen.getByTestId('project-plan-request-revision-button'))
    await waitFor(() => {
      expect(useStore.getState().projectScopedWorkspaces['/tmp/project-contract-behavior']?.planStatus).toBe(
        'revision-requested',
      )
    })
    expect(screen.getByTestId('project-plan-gate-surface')).toHaveTextContent('Plan status: revision-requested')

    const transitionMessages = (
      useStore.getState().projectScopedWorkspaces['/tmp/project-contract-behavior']?.conversationHistory || []
    ).map((entry) => entry.content)
    expect(transitionMessages).toEqual(
      expect.arrayContaining([
        'Approved plan plan-contract-behavior (draft -> approved).',
        'Rejected plan plan-contract-behavior (approved -> rejected).',
        'Requested revision for plan plan-contract-behavior (rejected -> revision-requested).',
      ]),
    )
  })

  it('[CID:12.4.05] integrates build invocation-from-approved-plan contract and error paths', async () => {
    const buildLaunchFailureMessage = 'build launch contract failure'
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = requestUrl(input)
      if (url.includes('/api/projects/metadata')) {
        return jsonResponse({ branch: 'main', commit: 'abc123def456' })
      }
      if (url.endsWith('/api/flows/contract-behavior.dot')) {
        return jsonResponse({ content: 'digraph BuildContract { start -> end }' })
      }
      if (url.endsWith('/pipelines') && init?.method === 'POST') {
        return jsonResponse({ detail: buildLaunchFailureMessage }, { status: 503 })
      }
      return jsonResponse({})
    })
    vi.stubGlobal('fetch', fetchMock)

    act(() => {
      useStore.setState((state) => ({
        ...state,
        viewMode: 'editor',
        activeProjectPath: '/tmp/project-contract-behavior',
        activeFlow: 'contract-behavior.dot',
        projectScopedWorkspaces: {
          ...state.projectScopedWorkspaces,
          '/tmp/project-contract-behavior': {
            ...state.projectScopedWorkspaces['/tmp/project-contract-behavior'],
            planId: 'plan-contract-behavior',
            planStatus: 'draft',
          },
        },
      }))
    })

    const user = userEvent.setup()
    render(<Navbar />)

    await user.click(screen.getByTestId('execute-button'))

    expect(screen.getByTestId('run-start-error-banner')).toHaveTextContent(
      'Build workflow launch requires an approved plan state.',
    )
    expect(screen.getByTestId('build-workflow-failure-diagnostics')).toHaveTextContent(
      'Build workflow launch requires an approved plan state.',
    )
    expect(screen.getByTestId('build-workflow-rerun-button')).toBeDisabled()
    expect(fetchMock).not.toHaveBeenCalled()

    act(() => {
      useStore.setState((state) => ({
        ...state,
        projectScopedWorkspaces: {
          ...state.projectScopedWorkspaces,
          '/tmp/project-contract-behavior': {
            ...state.projectScopedWorkspaces['/tmp/project-contract-behavior'],
            planStatus: 'approved',
          },
        },
      }))
    })

    await user.click(screen.getByTestId('execute-button'))

    await waitFor(() => {
      expect(screen.getByTestId('run-start-error-banner')).toHaveTextContent(buildLaunchFailureMessage)
    })
    expect(screen.getByTestId('build-workflow-failure-message')).toHaveTextContent(buildLaunchFailureMessage)
    expect(screen.getByTestId('build-workflow-rerun-button')).toBeEnabled()

    const pipelineCall = fetchMock.mock.calls.find(
      ([request, init]) => requestUrl(request as RequestInfo | URL).endsWith('/pipelines') && init?.method === 'POST',
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
    expect(screen.getByLabelText('Default Max Retry')).toBeVisible()
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
        selectedRunId: 'run-focus-audit',
        runtimeStatus: 'running',
      }))
    })

    render(<ExecutionControls />)

    expect(screen.getByTestId('execution-footer-cancel-button').className).toContain('focus-visible')
    expect(screen.getByTestId('execution-footer-pause-button').className).toContain('focus-visible')
    expect(screen.getByTestId('execution-footer-resume-button').className).toContain('focus-visible')

    cleanup()
    act(() => {
      resetContractState()
    })
    render(<ProjectsPanel />)

    expect(screen.getByLabelText('Project directory path')).toBeVisible()
    expect(screen.getByTestId('project-path-input').className).toContain('focus-visible')
    expect(screen.getByTestId('project-register-button').className).toContain('focus-visible')
    expect(screen.getByTestId('favorite-toggle-button').className).toContain('focus-visible')
    expect(screen.getByTestId('project-edit-button').className).toContain('focus-visible')

    fireEvent.click(screen.getByTestId('project-edit-button'))
    expect(screen.getByLabelText('Edit project directory path')).toBeVisible()
    expect(screen.getByTestId('project-edit-input').className).toContain('focus-visible')
    expect(screen.getByTestId('project-edit-cancel-button').className).toContain('focus-visible')
    expect(screen.getByTestId('project-edit-save-button').className).toContain('focus-visible')

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

  it('[CID:13.1.03] verifies diagnostic and status color contrast meets WCAG-oriented thresholds', () => {
    const indexCss = readFrontendIndexCss()
    const destructiveTokenRgb = hslToRgb(parseRootHslToken(indexCss, 'destructive'))
    const white: [number, number, number] = [255, 255, 255]

    const warningText: [number, number, number] = [146, 64, 14] // tailwind amber-800
    const warningBackgroundBase: [number, number, number] = [245, 158, 11] // tailwind amber-500
    const infoText: [number, number, number] = [3, 105, 161] // tailwind sky-700
    const infoBackgroundBase: [number, number, number] = [14, 165, 233] // tailwind sky-500
    const successText: [number, number, number] = [22, 101, 52] // tailwind green-800
    const successBackgroundBase: [number, number, number] = [34, 197, 94] // tailwind green-500

    const samples: Array<{ name: string; ratio: number }> = [
      { name: 'error text on base surface', ratio: contrastRatio(destructiveTokenRgb, white) },
      { name: 'error text on diagnostic badge /10', ratio: contrastRatio(destructiveTokenRgb, blendOnWhite(destructiveTokenRgb, 0.1)) },
      { name: 'error text on diagnostic badge /15', ratio: contrastRatio(destructiveTokenRgb, blendOnWhite(destructiveTokenRgb, 0.15)) },
      { name: 'error text on status badge /20', ratio: contrastRatio(destructiveTokenRgb, blendOnWhite(destructiveTokenRgb, 0.2)) },
      { name: 'warning text on base surface', ratio: contrastRatio(warningText, white) },
      { name: 'warning text on diagnostic badge /10', ratio: contrastRatio(warningText, blendOnWhite(warningBackgroundBase, 0.1)) },
      { name: 'warning text on diagnostic badge /15', ratio: contrastRatio(warningText, blendOnWhite(warningBackgroundBase, 0.15)) },
      { name: 'warning text on status badge /20', ratio: contrastRatio(warningText, blendOnWhite(warningBackgroundBase, 0.2)) },
      { name: 'info text on base surface', ratio: contrastRatio(infoText, white) },
      { name: 'info text on diagnostic badge /10', ratio: contrastRatio(infoText, blendOnWhite(infoBackgroundBase, 0.1)) },
      { name: 'info text on diagnostic badge /15', ratio: contrastRatio(infoText, blendOnWhite(infoBackgroundBase, 0.15)) },
      { name: 'info text on status badge /20', ratio: contrastRatio(infoText, blendOnWhite(infoBackgroundBase, 0.2)) },
      { name: 'success text on base surface', ratio: contrastRatio(successText, white) },
      { name: 'success text on status badge /10', ratio: contrastRatio(successText, blendOnWhite(successBackgroundBase, 0.1)) },
      { name: 'success text on status badge /15', ratio: contrastRatio(successText, blendOnWhite(successBackgroundBase, 0.15)) },
      { name: 'success text on status badge /20', ratio: contrastRatio(successText, blendOnWhite(successBackgroundBase, 0.2)) },
    ]

    for (const sample of samples) {
      if (sample.ratio < 4.5) {
        throw new Error(`${sample.name} contrast ratio ${sample.ratio.toFixed(2)} is below 4.50`)
      }
    }
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
      render(<RunsPanel />)

      await waitFor(() => {
        expect(screen.getByTestId('run-event-timeline-panel')).toBeVisible()
      })
      expect(screen.getByTestId('run-event-timeline-panel')).toHaveAttribute('data-responsive-layout', 'stacked')
    } finally {
      setViewportWidth(originalViewportWidth)
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
      render(<ProjectsPanel />)

      expect(screen.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('project-register-controls')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getAllByTestId('project-row-actions')[0]).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('project-register-button')).toBeVisible()
      expect(screen.getByTestId('favorite-toggle-button')).toBeVisible()

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
      render(<ExecutionControls />)

      expect(screen.getByTestId('execution-footer-controls')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('execution-footer-cancel-button')).toBeVisible()
      expect(screen.getByTestId('execution-footer-unsupported-controls-reason')).toBeVisible()

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
      expect(screen.getByTestId('execute-button')).toBeVisible()
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
      render(<ProjectsPanel />)
      expect(screen.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'split')
      expect(screen.getByTestId('project-register-controls')).toHaveAttribute('data-responsive-layout', 'inline')
      expect(screen.getAllByTestId('project-row-actions')[0]).toHaveAttribute('data-responsive-layout', 'inline')

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
      render(<ProjectsPanel />)
      expect(screen.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getByTestId('project-register-controls')).toHaveAttribute('data-responsive-layout', 'stacked')
      expect(screen.getAllByTestId('project-row-actions')[0]).toHaveAttribute('data-responsive-layout', 'stacked')

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
      render(<ExecutionControls />)
      expect(screen.getByTestId('execution-footer-controls')).toHaveAttribute('data-responsive-layout', 'inline')

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
      render(<ExecutionControls />)
      expect(screen.getByTestId('execution-footer-controls')).toHaveAttribute('data-responsive-layout', 'stacked')
    } finally {
      setViewportWidth(originalViewportWidth)
    }
  })

  it('[CID:13.3.01] defines explicit performance budgets for canvas interaction and timeline updates', async () => {
    const runId = 'run-performance-budget-contract'
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
      started_at: '2026-03-05T00:00:00Z',
      ended_at: null,
      last_error: '',
      token_usage: 0,
    }

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = requestUrl(input)
        if (url.endsWith('/api/flows/contract-behavior.dot')) {
          return jsonResponse({
            name: 'contract-behavior.dot',
            content: 'digraph G { start [label="Start"]; end [label="End"]; start -> end; }',
          })
        }
        if (url.endsWith('/preview')) {
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
        if (url.endsWith('/runs')) {
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

    render(<RunsPanel />)

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
        if (url.endsWith('/api/flows/contract-behavior.dot')) {
          return jsonResponse({
            name: 'contract-behavior.dot',
            content: 'digraph G { start -> end; }',
          })
        }
        if (url.endsWith('/preview')) {
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
    const runApiPath = `/pipelines/${encodeURIComponent(runId)}`
    const maxItems = 200
    const totalEvents = maxItems + 35
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
      started_at: '2026-03-05T04:00:00Z',
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

    render(<RunsPanel />)

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

  it('[CID:14.0.01] marks the active project in Projects quick-switch lists', () => {
    act(() => {
      resetContractState()
      useStore.setState((state) => ({
        ...state,
        viewMode: 'projects',
        activeProjectPath: '/tmp/project-alpha',
        projectRegistry: {
          '/tmp/project-alpha': {
            directoryPath: '/tmp/project-alpha',
            isFavorite: true,
            lastAccessedAt: '2026-03-05T00:00:00Z',
          },
          '/tmp/project-beta': {
            directoryPath: '/tmp/project-beta',
            isFavorite: false,
            lastAccessedAt: '2026-03-05T00:00:00Z',
          },
        },
        recentProjectPaths: ['/tmp/project-alpha', '/tmp/project-beta'],
      }))
    })

    render(<ProjectsPanel />)

    const favoritesList = screen.getByTestId('favorite-projects-list')
    const recentsList = screen.getByTestId('recent-projects-list')
    const favoriteActiveButton = within(favoritesList).getByRole('button', { name: /project-alpha/i })
    const recentActiveButton = within(recentsList).getByRole('button', { name: /project-alpha/i })
    const recentInactiveButton = within(recentsList).getByRole('button', { name: /project-beta/i })

    expect(favoriteActiveButton).toHaveAttribute('aria-current', 'true')
    expect(recentActiveButton).toHaveAttribute('aria-current', 'true')
    expect(recentInactiveButton).not.toHaveAttribute('aria-current')

    expect(within(favoriteActiveButton).getByText('Active')).toBeVisible()
    expect(within(recentActiveButton).getByText('Active')).toBeVisible()
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
      if (url.endsWith('/api/flows') && init?.method === 'POST') {
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
      if (url.endsWith('/api/flows') && init?.method === 'POST') {
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

  it('[CID:11.5.01] restores persisted project registry across sessions and keeps unique-directory enforcement active', async () => {
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
      'sparkspawn.project_registry_state',
      JSON.stringify({
        '/tmp/persisted-project': {
          directoryPath: '/tmp/persisted-project',
          isFavorite: true,
          lastAccessedAt: '2026-03-04T00:00:00.000Z',
        },
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.ui_route_state',
      JSON.stringify({
        viewMode: 'projects',
        activeProjectPath: '/tmp/persisted-project',
        activeFlow: null,
        selectedRunId: null,
      }),
    )

    const { useStore: restoredStore } = await import('@/store')
    const restoredState = restoredStore.getState()

    expect(restoredState.projectRegistry).toEqual({
      '/tmp/persisted-project': {
        directoryPath: '/tmp/persisted-project',
        isFavorite: true,
        lastAccessedAt: '2026-03-04T00:00:00.000Z',
      },
    })

    const duplicateResult = restoredState.registerProject('/tmp/persisted-project')
    expect(duplicateResult.ok).toBe(false)
    expect(duplicateResult.error).toBe('Project already registered: /tmp/persisted-project')

    const newProjectResult = restoredState.registerProject('/tmp/new-project')
    expect(newProjectResult.ok).toBe(true)
    const persistedRegistryRaw = localStorageMock.getItem('sparkspawn.project_registry_state')
    expect(persistedRegistryRaw).toBeTruthy()
    const persistedRegistry = JSON.parse(String(persistedRegistryRaw)) as Record<string, { directoryPath: string }>
    expect(persistedRegistry['/tmp/persisted-project']?.directoryPath).toBe('/tmp/persisted-project')
    expect(persistedRegistry['/tmp/new-project']?.directoryPath).toBe('/tmp/new-project')
  })

  it('[CID:11.5.02] restores project-scoped conversation/spec/plan linkage for normalized project paths', async () => {
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
      'sparkspawn.project_registry_state',
      JSON.stringify({
        '/tmp/persisted-project': {
          directoryPath: '/tmp/persisted-project',
          isFavorite: false,
          lastAccessedAt: '2026-03-04T00:00:00.000Z',
        },
        '/tmp/other-project': {
          directoryPath: '/tmp/other-project',
          isFavorite: false,
          lastAccessedAt: '2026-03-04T00:00:00.000Z',
        },
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.ui_route_state',
      JSON.stringify({
        viewMode: 'projects',
        activeProjectPath: '/tmp/persisted-project',
        activeFlow: null,
        selectedRunId: null,
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.project_conversation_state',
      JSON.stringify({
        '/tmp/persisted-project/./': {
          conversationId: 'conversation-persisted-project',
          conversationHistory: [
            {
              role: 'user',
              content: 'Persisted scope history',
              timestamp: '2026-03-04T00:05:00.000Z',
            },
          ],
          specId: 'spec-persisted-project',
          specStatus: 'approved',
          planId: 'plan-persisted-project',
          planStatus: 'rejected',
        },
        '/tmp/other-project': {
          conversationId: 'conversation-other-project',
          conversationHistory: [
            {
              role: 'assistant',
              content: 'Other scope history',
              timestamp: '2026-03-04T00:06:00.000Z',
            },
          ],
          specId: 'spec-other-project',
          specStatus: 'draft',
          planId: 'plan-other-project',
          planStatus: 'revision-requested',
        },
      }),
    )

    const { useStore: restoredStore } = await import('@/store')

    const persistedScope = restoredStore.getState().projectScopedWorkspaces['/tmp/persisted-project']
    expect(persistedScope.conversationId).toBe('conversation-persisted-project')
    expect(persistedScope.specId).toBe('spec-persisted-project')
    expect(persistedScope.specStatus).toBe('approved')
    expect(persistedScope.planId).toBe('plan-persisted-project')
    expect(persistedScope.planStatus).toBe('rejected')
    expect(persistedScope.conversationHistory).toHaveLength(1)
    expect(persistedScope.conversationHistory[0]?.content).toBe('Persisted scope history')

    restoredStore.getState().setActiveProjectPath('/tmp/other-project')
    const switchedScope = restoredStore.getState().projectScopedWorkspaces['/tmp/other-project']
    expect(switchedScope.conversationId).toBe('conversation-other-project')
    expect(switchedScope.specId).toBe('spec-other-project')
    expect(switchedScope.planId).toBe('plan-other-project')
    expect(switchedScope.planStatus).toBe('revision-requested')
    expect(switchedScope.conversationHistory).toHaveLength(1)
    expect(switchedScope.conversationHistory[0]?.content).toBe('Other scope history')
  })

  it('[CID:11.5.03] clears stale flow/run route context when persisted active project path is invalid on reopen', async () => {
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
      'sparkspawn.project_registry_state',
      JSON.stringify({
        '/tmp/persisted-project': {
          directoryPath: '/tmp/persisted-project',
          isFavorite: true,
          lastAccessedAt: '2026-03-04T00:00:00.000Z',
        },
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.ui_route_state',
      JSON.stringify({
        viewMode: 'editor',
        activeProjectPath: 'relative/project',
        activeFlow: 'leaked-flow.dot',
        selectedRunId: 'leaked-run-id',
      }),
    )

    const { useStore: restoredStore } = await import('@/store')
    const restoredState = restoredStore.getState()

    expect(restoredState.activeProjectPath).toBeNull()
    expect(restoredState.viewMode).toBe('projects')
    expect(restoredState.activeFlow).toBeNull()
    expect(restoredState.selectedRunId).toBeNull()
    expect(restoredState.workingDir).toBe(DEFAULT_WORKING_DIRECTORY)
    expect(restoredState.projectScopedWorkspaces).toEqual({})
    expect(restoredState.projectRegistry['/tmp/persisted-project']).toBeDefined()
  })

  it('[CID:11.6.01] persists and restores spec/plan provenance references for workflow-generated artifacts', async () => {
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
      'sparkspawn.project_registry_state',
      JSON.stringify({
        '/tmp/persisted-project': {
          directoryPath: '/tmp/persisted-project',
          isFavorite: false,
          lastAccessedAt: '2026-03-04T00:00:00.000Z',
        },
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.ui_route_state',
      JSON.stringify({
        viewMode: 'projects',
        activeProjectPath: '/tmp/persisted-project',
        activeFlow: null,
        selectedRunId: null,
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.project_conversation_state',
      JSON.stringify({
        '/tmp/persisted-project': {
          conversationId: 'conversation-persisted-project',
          conversationHistory: [],
          specId: 'spec-persisted-project',
          specStatus: 'approved',
          specProvenance: {
            source: 'spec-edit-proposal',
            referenceId: 'proposal-123',
            capturedAt: '2026-03-04T00:05:00.000Z',
          },
          planId: 'plan-persisted-project',
          planStatus: 'draft',
          planProvenance: {
            source: 'plan-generation-workflow',
            referenceId: 'run-plan-123',
            capturedAt: '2026-03-04T00:10:00.000Z',
          },
        },
      }),
    )

    const { useStore: restoredStore } = await import('@/store')
    const restoredScope = restoredStore.getState().projectScopedWorkspaces['/tmp/persisted-project']
    expect(restoredScope.specProvenance).toEqual({
      source: 'spec-edit-proposal',
      referenceId: 'proposal-123',
      capturedAt: '2026-03-04T00:05:00.000Z',
      runId: null,
      gitBranch: null,
      gitCommit: null,
    })
    expect(restoredScope.planProvenance).toEqual({
      source: 'plan-generation-workflow',
      referenceId: 'run-plan-123',
      capturedAt: '2026-03-04T00:10:00.000Z',
      runId: null,
      gitBranch: null,
      gitCommit: null,
    })

    restoredStore.getState().setSpecProvenance({
      source: 'spec-edit-proposal',
      referenceId: 'proposal-456',
      capturedAt: '2026-03-04T00:15:00.000Z',
    })
    restoredStore.getState().setPlanProvenance({
      source: 'plan-generation-workflow',
      referenceId: 'run-plan-456',
      capturedAt: '2026-03-04T00:20:00.000Z',
    })

    const persistedConversationStateRaw = localStorageMock.getItem('sparkspawn.project_conversation_state')
    expect(persistedConversationStateRaw).toBeTruthy()
    const persistedConversationState = JSON.parse(String(persistedConversationStateRaw)) as Record<
      string,
      {
        specProvenance?: {
          source?: string
          referenceId?: string
          capturedAt?: string
          runId?: string | null
          gitBranch?: string | null
          gitCommit?: string | null
        }
        planProvenance?: {
          source?: string
          referenceId?: string
          capturedAt?: string
          runId?: string | null
          gitBranch?: string | null
          gitCommit?: string | null
        }
      }
    >
    expect(persistedConversationState['/tmp/persisted-project']?.specProvenance).toMatchObject({
      source: 'spec-edit-proposal',
      referenceId: 'proposal-456',
      capturedAt: '2026-03-04T00:15:00.000Z',
    })
    expect(persistedConversationState['/tmp/persisted-project']?.planProvenance).toMatchObject({
      source: 'plan-generation-workflow',
      referenceId: 'run-plan-456',
      capturedAt: '2026-03-04T00:20:00.000Z',
    })
  })

  it('[CID:11.6.02] persists and restores provenance run-linkage and available git branch/commit context', async () => {
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
      'sparkspawn.project_registry_state',
      JSON.stringify({
        '/tmp/persisted-project': {
          directoryPath: '/tmp/persisted-project',
          isFavorite: false,
          lastAccessedAt: '2026-03-04T00:00:00.000Z',
        },
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.ui_route_state',
      JSON.stringify({
        viewMode: 'projects',
        activeProjectPath: '/tmp/persisted-project',
        activeFlow: null,
        selectedRunId: null,
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.project_conversation_state',
      JSON.stringify({
        '/tmp/persisted-project': {
          conversationId: null,
          conversationHistory: [],
          specId: 'spec-persisted-project',
          specStatus: 'approved',
          specProvenance: {
            source: 'spec-edit-proposal',
            referenceId: 'proposal-123',
            capturedAt: '2026-03-04T00:05:00.000Z',
            runId: 'run-spec-123',
            gitBranch: 'feature/spec-provenance',
            gitCommit: 'abcde12345',
          },
          planId: 'plan-persisted-project',
          planStatus: 'approved',
          planProvenance: {
            source: 'plan-generation-workflow',
            referenceId: 'run-plan-123',
            capturedAt: '2026-03-04T00:10:00.000Z',
            runId: 'run-plan-123',
            gitBranch: 'feature/plan-provenance',
            gitCommit: '98765fedcb',
          },
        },
      }),
    )

    const { useStore: restoredStore } = await import('@/store')
    const restoredScope = restoredStore.getState().projectScopedWorkspaces['/tmp/persisted-project']
    expect(restoredScope.specProvenance).toEqual({
      source: 'spec-edit-proposal',
      referenceId: 'proposal-123',
      capturedAt: '2026-03-04T00:05:00.000Z',
      runId: 'run-spec-123',
      gitBranch: 'feature/spec-provenance',
      gitCommit: 'abcde12345',
    })
    expect(restoredScope.planProvenance).toEqual({
      source: 'plan-generation-workflow',
      referenceId: 'run-plan-123',
      capturedAt: '2026-03-04T00:10:00.000Z',
      runId: 'run-plan-123',
      gitBranch: 'feature/plan-provenance',
      gitCommit: '98765fedcb',
    })

    restoredStore.getState().setSpecProvenance({
      source: 'spec-edit-proposal',
      referenceId: 'proposal-456',
      capturedAt: '2026-03-04T00:15:00.000Z',
      runId: 'run-spec-456',
      gitBranch: 'main',
      gitCommit: '22222aaaaa',
    })
    restoredStore.getState().setPlanProvenance({
      source: 'plan-generation-workflow',
      referenceId: 'run-plan-456',
      capturedAt: '2026-03-04T00:20:00.000Z',
      runId: 'run-plan-456',
      gitBranch: 'main',
      gitCommit: '33333bbbbb',
    })

    const persistedConversationStateRaw = localStorageMock.getItem('sparkspawn.project_conversation_state')
    expect(persistedConversationStateRaw).toBeTruthy()
    const persistedConversationState = JSON.parse(String(persistedConversationStateRaw)) as Record<
      string,
      {
        specProvenance?: {
          source?: string
          referenceId?: string
          capturedAt?: string
          runId?: string | null
          gitBranch?: string | null
          gitCommit?: string | null
        }
        planProvenance?: {
          source?: string
          referenceId?: string
          capturedAt?: string
          runId?: string | null
          gitBranch?: string | null
          gitCommit?: string | null
        }
      }
    >
    expect(persistedConversationState['/tmp/persisted-project']?.specProvenance).toEqual({
      source: 'spec-edit-proposal',
      referenceId: 'proposal-456',
      capturedAt: '2026-03-04T00:15:00.000Z',
      runId: 'run-spec-456',
      gitBranch: 'main',
      gitCommit: '22222aaaaa',
    })
    expect(persistedConversationState['/tmp/persisted-project']?.planProvenance).toEqual({
      source: 'plan-generation-workflow',
      referenceId: 'run-plan-456',
      capturedAt: '2026-03-04T00:20:00.000Z',
      runId: 'run-plan-456',
      gitBranch: 'main',
      gitCommit: '33333bbbbb',
    })
  })

  it('[CID:11.6.03] persists and restores plan status lifecycle transitions', async () => {
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
      'sparkspawn.project_registry_state',
      JSON.stringify({
        '/tmp/lifecycle-project': {
          directoryPath: '/tmp/lifecycle-project',
          isFavorite: false,
          lastAccessedAt: '2026-03-04T00:00:00.000Z',
        },
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.ui_route_state',
      JSON.stringify({
        viewMode: 'projects',
        activeProjectPath: '/tmp/lifecycle-project',
        activeFlow: null,
        selectedRunId: null,
      }),
    )
    localStorageMock.setItem(
      'sparkspawn.project_conversation_state',
      JSON.stringify({
        '/tmp/lifecycle-project': {
          conversationId: null,
          conversationHistory: [],
          specId: 'spec-lifecycle-project',
          specStatus: 'approved',
          planId: 'plan-lifecycle-project',
          planStatus: 'draft',
        },
      }),
    )

    const { useStore: restoredStore } = await import('@/store')
    expect(restoredStore.getState().projectScopedWorkspaces['/tmp/lifecycle-project']?.planStatus).toBe('draft')

    const lifecycleStatuses = ['approved', 'rejected', 'revision-requested', 'draft'] as const
    lifecycleStatuses.forEach((status) => {
      restoredStore.getState().setPlanStatus(status)
      const persistedConversationStateRaw = localStorageMock.getItem('sparkspawn.project_conversation_state')
      expect(persistedConversationStateRaw).toBeTruthy()
      const persistedConversationState = JSON.parse(String(persistedConversationStateRaw)) as Record<
        string,
        { planStatus?: string }
      >
      expect(persistedConversationState['/tmp/lifecycle-project']?.planStatus).toBe(status)
    })

    vi.resetModules()
    vi.stubGlobal('localStorage', localStorageMock)
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      value: localStorageMock,
    })
    const { useStore: reopenedStore } = await import('@/store')
    expect(reopenedStore.getState().projectScopedWorkspaces['/tmp/lifecycle-project']?.planStatus).toBe('draft')
  })
})
