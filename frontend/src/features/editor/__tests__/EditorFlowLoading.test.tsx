import { Editor } from '@/features/editor/Editor'
import { useStore } from '@/store'
import { ReactFlowProvider } from '@xyflow/react'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { EditorPreviewResponse } from '@/features/editor/services/editorPreview'
import {
  loadEditorFlowPayload,
  loadEditorPreview,
} from '@/features/editor/services/editorPreview'

vi.mock('@/features/editor/services/editorPreview', () => ({
  loadEditorFlowPayload: vi.fn(),
  loadEditorPreview: vi.fn(),
}))

vi.mock('@/features/editor/components/ValidationPanel', () => ({
  ValidationPanel: () => null,
}))

type Deferred<T> = {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason?: unknown) => void
}

const createDeferred = <T,>(): Deferred<T> => {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

const buildPreview = (nodeCount: number): EditorPreviewResponse => ({
  status: 'ok',
  graph: {
    nodes: Array.from({ length: nodeCount }, (_, index) => ({
      id: `node_${index + 1}`,
      label: `Node ${index + 1}`,
      shape: 'box',
      prompt: `Prompt ${index + 1}`,
    })),
    edges: [],
    graph_attrs: {},
  },
  diagnostics: [],
  errors: [],
})

const buildManagerPreview = (options?: {
  expanded?: boolean
  managerLabel?: string
}): EditorPreviewResponse => ({
  status: 'ok',
  graph: {
    nodes: [
      { id: 'start', label: 'Start', shape: 'Mdiamond' },
      {
        id: 'manager',
        label: options?.managerLabel ?? 'Manager',
        shape: 'house',
        type: 'stack.manager_loop',
      },
    ],
    edges: [
      { from: 'start', to: 'manager' },
    ],
    graph_attrs: {},
    ...(options?.expanded
      ? {
          child_previews: {
            manager: {
              flow_name: 'child-worker.dot',
              flow_path: '/tmp/child-worker.dot',
              flow_label: 'Child Worker',
              read_only: true,
              provenance: 'derived_child_preview',
              graph: {
                graph_attrs: {},
                nodes: [
                  { id: 'child_start', label: 'Child Start', shape: 'Mdiamond' },
                ],
                edges: [],
              },
            },
          },
        }
      : {}),
  },
  diagnostics: [],
  errors: [],
})

const resetEditorState = (activeFlow: string | null) => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'editor',
    activeFlow,
    executionFlow: null,
    suppressPreview: true,
    graphAttrs: {},
    editorExpandChildFlowsByFlow: {},
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    selectedNodeId: null,
    selectedEdgeId: null,
    uiDefaults: {
      llm_provider: 'openai',
      llm_model: 'gpt-5.4-mini',
      reasoning_effort: 'high',
    },
  }))
}

const renderEditor = () => render(
  <ReactFlowProvider>
    <Editor />
  </ReactFlowProvider>,
)

describe('Editor flow loading behavior', () => {
  beforeEach(() => {
    resetEditorState('flow-a.dot')
    localStorage.setItem('spark.debug.performance', '1')
    vi.mocked(loadEditorFlowPayload).mockImplementation(async (flowName: string) => ({
      name: flowName,
      content: flowName,
    }))
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('clears the previous canvas when the next flow load fails', async () => {
    const previewDeferreds = new Map<string, Deferred<EditorPreviewResponse>>([
      ['flow-a.dot', createDeferred<EditorPreviewResponse>()],
      ['flow-b.dot', createDeferred<EditorPreviewResponse>()],
    ])
    vi.mocked(loadEditorPreview).mockImplementation((dot: string) => {
      const deferred = previewDeferreds.get(dot)
      if (!deferred) {
        throw new Error(`Unexpected preview request: ${dot}`)
      }
      return deferred.promise
    })

    renderEditor()
    const profile = await screen.findByTestId('canvas-performance-profile')

    act(() => {
      previewDeferreds.get('flow-a.dot')?.resolve(buildPreview(1))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '1'))

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-b.dot' }))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '0'))

    act(() => {
      previewDeferreds.get('flow-b.dot')?.reject(new Error('preview failed'))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '0'))
  })

  it('ignores a superseded flow load that resolves after a newer selection', async () => {
    const previewDeferreds = new Map<string, Deferred<EditorPreviewResponse>>([
      ['flow-a.dot', createDeferred<EditorPreviewResponse>()],
      ['flow-b.dot', createDeferred<EditorPreviewResponse>()],
      ['flow-c.dot', createDeferred<EditorPreviewResponse>()],
    ])
    vi.mocked(loadEditorPreview).mockImplementation((dot: string) => {
      const deferred = previewDeferreds.get(dot)
      if (!deferred) {
        throw new Error(`Unexpected preview request: ${dot}`)
      }
      return deferred.promise
    })

    renderEditor()
    const profile = await screen.findByTestId('canvas-performance-profile')

    act(() => {
      previewDeferreds.get('flow-a.dot')?.resolve(buildPreview(1))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '1'))

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-b.dot' }))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '0'))

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-c.dot' }))
    })

    act(() => {
      previewDeferreds.get('flow-c.dot')?.resolve(buildPreview(3))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '3'))

    act(() => {
      previewDeferreds.get('flow-b.dot')?.resolve(buildPreview(2))
    })
    await waitFor(() => expect(profile).toHaveAttribute('data-node-count', '3'))
  })

  it('persists the child-flow expansion toggle per active flow and keeps expanded editor previews read-only', async () => {
    const user = userEvent.setup()
    vi.mocked(loadEditorPreview).mockImplementation(async (_dot: string, _init, options) => {
      const flowName = options?.flowName ?? 'unknown.dot'
      if (flowName === 'flow-a.dot') {
        return buildManagerPreview({ expanded: options?.expandChildren === true })
      }
      if (flowName === 'flow-b.dot') {
        return buildManagerPreview({ managerLabel: 'Secondary Manager' })
      }
      throw new Error(`Unexpected preview request for ${flowName}`)
    })

    renderEditor()

    await screen.findByTestId('editor-child-flow-toggle')
    await waitFor(() => {
      expect(
        vi.mocked(loadEditorPreview).mock.calls.some(([, , options]) =>
          options?.flowName === 'flow-a.dot' && options?.expandChildren !== true),
      ).toBe(true)
    })

    expect(screen.getByRole('button', { name: '+ Node' })).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Expanded' }))

    await waitFor(() => {
      expect(
        vi.mocked(loadEditorPreview).mock.calls.some(([, , options]) =>
          options?.flowName === 'flow-a.dot' && options?.expandChildren === true),
      ).toBe(true)
    })
    await screen.findByText('Expanded child-flow mode is a read-only canvas preview. Switch to Parent Only to edit.')
    expect(screen.getAllByText('Read-only Preview').length).toBeGreaterThan(0)
    expect(screen.queryByRole('button', { name: '+ Node' })).not.toBeInTheDocument()
    expect(useStore.getState().editorExpandChildFlowsByFlow['flow-a.dot']).toBe(true)

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-b.dot' }))
    })

    await waitFor(() => {
      expect(
        vi.mocked(loadEditorPreview).mock.calls.some(([, , options]) =>
          options?.flowName === 'flow-b.dot' && options?.expandChildren !== true),
      ).toBe(true)
    })
    await waitFor(() => {
      expect(
        screen.queryByText('Expanded child-flow mode is a read-only canvas preview. Switch to Parent Only to edit.'),
      ).not.toBeInTheDocument()
    })
    expect(screen.getByRole('button', { name: '+ Node' })).toBeInTheDocument()
    expect(useStore.getState().editorExpandChildFlowsByFlow['flow-a.dot']).toBe(true)
    expect(useStore.getState().editorExpandChildFlowsByFlow['flow-b.dot']).toBeUndefined()

    act(() => {
      useStore.setState((state) => ({ ...state, activeFlow: 'flow-a.dot' }))
    })

    await waitFor(() => {
      expect(
        vi.mocked(loadEditorPreview).mock.calls.some(([, , options]) =>
          options?.flowName === 'flow-a.dot' && options?.expandChildren === true),
      ).toBe(true)
    })
    await screen.findByText('Expanded child-flow mode is a read-only canvas preview. Switch to Parent Only to edit.')
    expect(screen.queryByRole('button', { name: '+ Node' })).not.toBeInTheDocument()
  })
})
