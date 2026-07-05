import {
  resolveEdgeFieldDiagnostics,
  resolveGraphFieldDiagnostics,
  resolveNodeFieldDiagnostics,
} from '@/lib/inspectorFieldDiagnostics'
import { getHandlerType, getNodeFieldVisibility } from '@/lib/nodeVisibility'
import { NodeInspectorPanel } from '@/features/editor/components/NodeInspectorPanel'
import { applyNodePropertyChangeToData } from '@/features/editor/Sidebar'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/lib/api/llmProfilesApi', () => ({
  fetchLlmProfiles: vi.fn(async () => []),
}))

afterEach(() => {
  cleanup()
})

describe('Inspector and node authoring behavior', () => {
  it('resolves handler types and field visibility for manager-loop authoring', () => {
    expect(getHandlerType('house', '')).toBe('stack.manager_loop')
    expect(getHandlerType('box', 'wait.human')).toBe('wait.human')

    const managerVisibility = getNodeFieldVisibility('stack.manager_loop')
    expect(managerVisibility.showManagerOptions).toBe(true)
    expect(managerVisibility.showTypeOverride).toBe(true)
    expect(managerVisibility.showPrompt).toBe(false)
    expect(managerVisibility.showLlmSettings).toBe(false)

    const codergenVisibility = getNodeFieldVisibility('codergen')
    expect(codergenVisibility.showPrompt).toBe(true)
    expect(codergenVisibility.showLlmSettings).toBe(true)
    expect(codergenVisibility.showAdvanced).toBe(true)
  })

  it('maps node diagnostics to actionable inspector fields', () => {
    const nodeDiagnostics = resolveNodeFieldDiagnostics(
      [
        {
          rule_id: 'goal_gate_has_retry',
          severity: 'error',
          message: 'goal_gate requires retry_target or fallback_retry_target.',
          node_id: 'node_a',
        },
        {
          rule_id: 'retry_target_exists',
          severity: 'error',
          message: 'fallback_retry_target references missing node.',
          node_id: 'node_a',
        },
        {
          rule_id: 'prompt_on_llm_nodes',
          severity: 'warning',
          message: 'Prompt is recommended for llm nodes.',
          node_id: 'node_a',
        },
      ],
      'node_a',
    )

    expect(nodeDiagnostics.goal_gate).toHaveLength(1)
    expect(nodeDiagnostics.retry_target).toHaveLength(1)
    expect(nodeDiagnostics.fallback_retry_target).toHaveLength(2)
    expect(nodeDiagnostics.prompt).toHaveLength(1)
    expect(nodeDiagnostics.label).toHaveLength(1)
  })

  it('maps edge and graph diagnostics to condition/fidelity/stylesheet fields', () => {
    const diagnostics = [
      {
        rule_id: 'condition_syntax',
        severity: 'error' as const,
        message: 'Condition parser failed near token.',
        edge: ['start', 'review'] as [string, string],
      },
      {
        rule_id: 'fidelity_valid',
        severity: 'warning' as const,
        message: 'Edge fidelity value is not recognized.',
        edge: ['start', 'review'] as [string, string],
      },
      {
        rule_id: 'stylesheet_syntax',
        severity: 'error' as const,
        message: 'Invalid stylesheet selector syntax.',
      },
      {
        rule_id: 'fidelity_valid',
        severity: 'error' as const,
        message: 'Graph fidelity must be one of supported values.',
      },
    ]

    const edgeDiagnostics = resolveEdgeFieldDiagnostics(diagnostics, 'start', 'review')
    expect(edgeDiagnostics.condition).toHaveLength(1)
    expect(edgeDiagnostics.fidelity).toHaveLength(1)

    const graphDiagnostics = resolveGraphFieldDiagnostics(diagnostics)
    expect(graphDiagnostics.model_stylesheet).toHaveLength(1)
    expect(graphDiagnostics.default_fidelity).toHaveLength(1)
  })

  it('renders conditional parallel threshold fields in the node inspector', () => {
    const onPropertyChange = vi.fn()
    const baseProps = {
      selectedNodeId: 'fan',
      graphAttrs: {},
      visibility: getNodeFieldVisibility('parallel'),
      readsContextDraft: '',
      readsContextError: null,
      writesContextDraft: '',
      writesContextError: null,
      showAdvanced: false,
      nodeFieldDiagnostics: {},
      selectedNodeExtensionEntries: [],
      selectedNodeToolHookPreWarning: null,
      selectedNodeToolHookPostWarning: null,
      selectedNodeShapeTypeMismatchWarning: null,
      onPropertyChange,
      onOpenGraphChildSettings: vi.fn(),
      onReadsContextChange: vi.fn(),
      onWritesContextChange: vi.fn(),
      onSetShowAdvanced: vi.fn(),
      onNodeExtensionValueChange: vi.fn(),
      onNodeExtensionRemove: vi.fn(),
      onNodeExtensionAdd: vi.fn(),
      renderFieldDiagnostics: vi.fn(() => null),
    }

    const { rerender } = render(
      <NodeInspectorPanel
        {...baseProps}
        selectedNode={{
          id: 'fan',
          position: { x: 0, y: 0 },
          data: { label: 'Fan', shape: 'component', join_policy: 'k_of_n', join_k: '2' },
        }}
      />,
    )

    expect(screen.getByText('K Threshold')).toBeInTheDocument()
    expect(screen.queryByText('Quorum Threshold')).not.toBeInTheDocument()
    fireEvent.change(screen.getByTestId('node-attr-input-join_k'), { target: { value: '3' } })
    expect(onPropertyChange).toHaveBeenCalledWith('join_k', '3')

    rerender(
      <NodeInspectorPanel
        {...baseProps}
        selectedNode={{
          id: 'fan',
          position: { x: 0, y: 0 },
          data: { label: 'Fan', shape: 'component', join_policy: 'quorum', join_quorum: '0.75' },
        }}
      />,
    )

    expect(screen.queryByText('K Threshold')).not.toBeInTheDocument()
    expect(screen.getByText('Quorum Threshold')).toBeInTheDocument()
    fireEvent.change(screen.getByTestId('node-attr-input-join_quorum'), { target: { value: '0.6' } })
    expect(onPropertyChange).toHaveBeenCalledWith('join_quorum', '0.6')
  })

  it('clears stale parallel threshold attrs when inspector join policy changes', () => {
    expect(
      applyNodePropertyChangeToData(
        { join_policy: 'k_of_n', join_k: '2', join_quorum: '0.75' },
        'join_policy',
        'wait_all',
      ),
    ).toEqual({ join_policy: 'wait_all' })

    expect(
      applyNodePropertyChangeToData(
        { join_policy: 'k_of_n', join_k: '2', join_quorum: '0.75' },
        'join_policy',
        'quorum',
      ),
    ).toEqual({ join_policy: 'quorum', join_quorum: '0.75' })

    expect(
      applyNodePropertyChangeToData(
        { join_policy: 'quorum', join_k: '2', join_quorum: '0.75' },
        'join_policy',
        'k_of_n',
      ),
    ).toEqual({ join_policy: 'k_of_n', join_k: '2' })
  })

  it('renders and edits full manager-loop authoring fields in the node inspector', () => {
    const onPropertyChange = vi.fn()
    render(
      <NodeInspectorPanel
        selectedNodeId="manager"
        selectedNode={{
          id: 'manager',
          position: { x: 0, y: 0 },
          data: {
            label: 'Manager',
            shape: 'house',
            type: 'stack.manager_loop',
            'manager.poll_interval': '25ms',
            'manager.max_cycles': '4',
            'manager.stop_condition': 'context.stack.child.ready=true',
            'manager.actions': 'observe,steer',
            'manager.steer_cooldown': '2s',
            'stack.child_autostart': false,
          },
        }}
        graphAttrs={{ 'stack.child_dotfile': 'child.dot' }}
        visibility={getNodeFieldVisibility('stack.manager_loop')}
        readsContextDraft=""
        readsContextError={null}
        writesContextDraft=""
        writesContextError={null}
        showAdvanced={false}
        nodeFieldDiagnostics={{}}
        selectedNodeExtensionEntries={[]}
        selectedNodeToolHookPreWarning={null}
        selectedNodeToolHookPostWarning={null}
        selectedNodeShapeTypeMismatchWarning={null}
        onPropertyChange={onPropertyChange}
        onOpenGraphChildSettings={vi.fn()}
        onReadsContextChange={vi.fn()}
        onWritesContextChange={vi.fn()}
        onSetShowAdvanced={vi.fn()}
        onNodeExtensionValueChange={vi.fn()}
        onNodeExtensionRemove={vi.fn()}
        onNodeExtensionAdd={vi.fn()}
        renderFieldDiagnostics={vi.fn(() => null)}
      />,
    )

    fireEvent.change(screen.getByTestId('node-attr-input-manager.steer_cooldown'), { target: { value: '5s' } })
    expect(onPropertyChange).toHaveBeenCalledWith('manager.steer_cooldown', '5s')

    fireEvent.click(screen.getByTestId('node-attr-checkbox-stack.child_autostart'))
    expect(onPropertyChange).toHaveBeenCalledWith('stack.child_autostart', true)
  })
})
