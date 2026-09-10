import type { ComponentProps } from 'react'
import userEvent from '@testing-library/user-event'
import { ContextKeyListEditor } from '@/features/editor/components/ContextKeyListEditor'
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

vi.mock('@/lib/useLlmProfiles', () => ({
  useLlmProfiles: () => [],
}))

afterEach(() => {
  cleanup()
})

describe('Inspector and node authoring behavior', () => {
  it('resolves handler types and field visibility for manager-loop authoring', () => {
    expect(getHandlerType('subflow')).toBe('stack.manager_loop')
    expect(getHandlerType('human_gate')).toBe('wait.human')

    const managerVisibility = getNodeFieldVisibility('stack.manager_loop')
    expect(managerVisibility.showManagerOptions).toBe(true)
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
    expect(graphDiagnostics.fidelity).toHaveLength(1)
  })

  it('renders conditional parallel threshold fields in the node inspector', () => {
    const onPropertyChange = vi.fn()
    const baseProps = {
      selectedNodeId: 'fan',
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
    fireEvent.change(screen.getByRole('textbox', { name: 'K Threshold' }), { target: { value: '3' } })
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
    fireEvent.change(screen.getByRole('textbox', { name: 'Quorum Threshold' }), { target: { value: '0.6' } })
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

  it('maps node kind edits to typed config and visual shape data', () => {
    const subflowData = applyNodePropertyChangeToData(
      { label: 'Task', kind: 'agent_task', config: { kind: 'agent_task', prompt: 'Do it' } },
      'kind',
      'subflow',
    )
    expect(subflowData).toMatchObject({
      label: 'Task',
      kind: 'subflow',
      shape: 'house',
      config: { kind: 'subflow', prompt: 'Do it' },
      flow_ref: 'child.yaml',
    })
    expect(subflowData).not.toHaveProperty('type')

    expect(
      applyNodePropertyChangeToData(
        { kind: 'subflow', config: { kind: 'subflow' } },
        'flow_ref',
        'nested/child.yaml',
      ),
    ).toMatchObject({
      flow_ref: 'nested/child.yaml',
      config: { kind: 'subflow', flow_ref: 'nested/child.yaml' },
    })
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
            flow_ref: 'child.yaml',
            'manager.poll_interval': '25ms',
            'manager.max_cycles': '4',
            'manager.stop_condition': 'context.stack.child.ready=true',
            'manager.actions': 'observe,steer',
            'manager.steer_cooldown': '2s',
            'stack.child_autostart': false,
          },
        }}
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

    fireEvent.change(screen.getByRole('textbox', { name: 'Manager Steer Cooldown' }), { target: { value: '5s' } })
    expect(onPropertyChange).toHaveBeenCalledWith('manager.steer_cooldown', '5s')

    fireEvent.click(screen.getByTestId('node-attr-checkbox-stack.child_autostart'))
    expect(onPropertyChange).toHaveBeenCalledWith('stack.child_autostart', true)
  })
})

function panelProps(kind = 'agent_task'): ComponentProps<typeof NodeInspectorPanel> {
  return {
    selectedNodeId: 'node',
    selectedNode: { id: 'node', position: { x: 0, y: 0 }, data: { kind, label: 'Implement' } },
    visibility: getNodeFieldVisibility(getHandlerType(kind)),
    readsContextDraft: '', readsContextError: null,
    writesContextDraft: '', writesContextError: null,
    showAdvanced: true, nodeFieldDiagnostics: {}, selectedNodeExtensionEntries: [],
    selectedNodeToolHookPreWarning: null, selectedNodeToolHookPostWarning: null,
    onPropertyChange: vi.fn(), onOpenGraphChildSettings: vi.fn(),
    onReadsContextChange: vi.fn(), onWritesContextChange: vi.fn(), onSetShowAdvanced: vi.fn(),
    onNodeExtensionValueChange: vi.fn(), onNodeExtensionRemove: vi.fn(), onNodeExtensionAdd: vi.fn(),
    renderFieldDiagnostics: vi.fn((_scope, field, diagnostics, testId) => (
      <div data-testid={testId}>{diagnostics[field]?.map((entry, index) => <p key={index}>{entry.message}</p>)}</div>
    )),
  }
}

function expectLabelsAndReferences() {
  for (const label of document.querySelectorAll('label')) {
    expect(label.htmlFor).not.toBe('')
    expect(document.getElementById(label.htmlFor)).toBeInTheDocument()
  }
  for (const control of document.querySelectorAll('[aria-describedby]')) {
    for (const id of control.getAttribute('aria-describedby')!.split(' ')) {
      expect(document.getElementById(id)).toBeInTheDocument()
    }
  }
  const ids = Array.from(document.querySelectorAll('[id]'), element => element.id)
  expect(new Set(ids).size).toBe(ids.length)
}

describe('Accessible node inspector fields', () => {
  it.each([
    ['agent_task', [['Prompt Instruction', 'prompt'], ['Max Retries', 'max_retries'], ['Timeout', 'timeout'],
      ['Retry Target', 'retry_target'], ['Fallback Retry Target', 'fallback_retry_target'], ['Fidelity', 'fidelity'],
      ['Thread ID', 'thread_id'], ['Class', 'class'], ['Reasoning Effort', 'reasoning_effort']]],
    ['tool', [['Tool Command', 'tool.command'], ['Pre Hook Override', 'tool.hooks.pre'],
      ['Post Hook Override', 'tool.hooks.post'], ['Artifact Paths', 'tool.artifacts.paths'],
      ['Stdout Artifact', 'tool.artifacts.stdout'], ['Stderr Artifact', 'tool.artifacts.stderr']]],
    ['parallel', [['Max Parallel', 'max_parallel']]],
    ['subflow', [['Child Flow Reference', 'flow_ref'], ['Manager Poll Interval', 'manager.poll_interval'],
      ['Manager Max Cycles', 'manager.max_cycles'], ['Manager Stop Condition', 'manager.stop_condition'],
      ['Manager Actions', 'manager.actions'], ['Manager Steer Cooldown', 'manager.steer_cooldown']]],
  ])('edits named %s fields, including advanced settings', (kind, fields) => {
    const props = panelProps(kind)
    render(<NodeInspectorPanel {...props} />)
    for (const [name, key] of [['Label', 'label'], ...fields]) {
      fireEvent.change(screen.getByRole('textbox', { name, exact: true }), { target: { value: 'edited' } })
      expect(props.onPropertyChange).toHaveBeenCalledWith(key, 'edited')
    }
    fireEvent.change(screen.getByRole('combobox', { name: 'Node Kind' }), { target: { value: 'tool' } })
    expect(props.onPropertyChange).toHaveBeenCalledWith('kind', 'tool')
    if (kind === 'agent_task') {
      for (const [name, key] of [['LLM Model', 'llm_model'], ['LLM Provider', 'llm_provider']]) {
        fireEvent.change(screen.getByRole('combobox', { name }), { target: { value: 'custom' } })
        expect(props.onPropertyChange).toHaveBeenCalledWith(key, 'custom')
      }
      for (const [name, key] of [['Goal Gate', 'goal_gate'], ['Auto Status', 'auto_status'], ['Allow Partial', 'allow_partial']]) {
        fireEvent.click(screen.getByRole('checkbox', { name }))
        expect(props.onPropertyChange).toHaveBeenCalledWith(key, true)
      }
    }
    if (kind === 'parallel') {
      for (const [name, key, value] of [['Join Policy', 'join_policy', 'quorum'], ['Error Policy', 'error_policy', 'fail_fast']]) {
        fireEvent.change(screen.getByRole('combobox', { name }), { target: { value } })
        expect(props.onPropertyChange).toHaveBeenCalledWith(key, value)
      }
    }
    expectLabelsAndReferences()
  })

  it('keeps advanced field associations when expanded and panel IDs unique', () => {
    const props = panelProps()
    const { rerender } = render(<NodeInspectorPanel {...props} showAdvanced={false} />)
    expect(screen.queryByRole('textbox', { name: 'Timeout' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Show Advanced' }))
    expect(props.onSetShowAdvanced).toHaveBeenCalledWith(expect.any(Function))
    rerender(<><NodeInspectorPanel {...props} /><NodeInspectorPanel {...props} /></>)
    expect(screen.getAllByRole('textbox', { name: 'Timeout' })).toHaveLength(2)
    expectLabelsAndReferences()
  })

  it('associates diagnostics and clears errors and references after selection changes', () => {
    const props = panelProps()
    const error = { rule_id: 'test', severity: 'error' as const, message: 'Retry target is missing.' }
    const warning = { rule_id: 'test', severity: 'warning' as const, message: 'Prompt is recommended.' }
    props.nodeFieldDiagnostics = { prompt: [warning], goal_gate: [error], retry_target: [error], fallback_retry_target: [error], fidelity: [warning] }
    const { rerender } = render(<NodeInspectorPanel {...props} />)
    const prompt = screen.getByRole('textbox', { name: 'Prompt Instruction' })
    expect(prompt).toHaveAccessibleDescription(warning.message)
    expect(prompt).not.toHaveAttribute('aria-invalid')
    for (const name of ['Retry Target', 'Fallback Retry Target', 'Goal Gate']) {
      const control = screen.getByLabelText(name, { exact: true })
      expect(control).toHaveAccessibleDescription(error.message)
      expect(control).toHaveAttribute('aria-invalid', 'true')
    }
    expect(screen.getByRole('textbox', { name: 'Fidelity' })).toHaveAccessibleDescription(warning.message)
    expect(props.renderFieldDiagnostics).toHaveBeenCalledWith('node', 'retry_target', props.nodeFieldDiagnostics, 'node-field-diagnostics-retry_target')
    expectLabelsAndReferences()
    rerender(<NodeInspectorPanel {...props} nodeFieldDiagnostics={{}} />)
    expect(prompt).not.toHaveAttribute('aria-describedby')
    expect(screen.getByRole('textbox', { name: 'Retry Target', exact: true })).not.toHaveAttribute('aria-invalid')
    expect(screen.queryByText(error.message)).not.toBeInTheDocument()
    rerender(<NodeInspectorPanel {...panelProps('subflow')} selectedNodeId="implement" />)
    expect(screen.queryByRole('textbox', { name: 'Prompt Instruction' })).not.toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'Child Flow Reference' })).toHaveAccessibleDescription(/Subflow nodes use/)
    expectLabelsAndReferences()
  })

  it('associates tool warnings without marking errors and removes obsolete references', () => {
    const props = panelProps('tool')
    const { rerender } = render(<NodeInspectorPanel {...props} selectedNodeToolHookPreWarning="Pre hook warning" selectedNodeToolHookPostWarning="Post hook warning" />)
    for (const name of ['Pre', 'Post']) {
      const input = screen.getByRole('textbox', { name: `${name} Hook Override` })
      expect(input).toHaveAccessibleDescription(`${name} hook warning`)
      expect(input).not.toHaveAttribute('aria-invalid')
    }
    rerender(<NodeInspectorPanel {...props} />)
    for (const name of ['Pre', 'Post']) {
      expect(screen.getByRole('textbox', { name: `${name} Hook Override` })).not.toHaveAttribute('aria-describedby')
    }
    expectLabelsAndReferences()
  })

  it('keeps context names and callbacks distinct, describes drafts and clears errors', () => {
    const props = panelProps('subflow')
    const { rerender } = render(<NodeInspectorPanel {...props} readsContextError="Invalid context key" readsContextDraft="bad" />)
    const reads = screen.getByRole('textbox', { name: 'Reads Context' })
    const writes = screen.getByRole('textbox', { name: 'Writes Context' })
    expect(reads).toHaveValue('bad')
    expect(reads).toHaveAccessibleDescription(/expects to consume.*Invalid context key/)
    expect(reads).toHaveAttribute('aria-invalid', 'true')
    expect(writes).toHaveAccessibleDescription(/expected to produce.*One `context.\*` key per line/)
    expect(writes).not.toHaveAttribute('aria-invalid')
    fireEvent.change(reads, { target: { value: 'context.read' } })
    fireEvent.change(writes, { target: { value: 'context.write' } })
    expect(props.onReadsContextChange).toHaveBeenCalledExactlyOnceWith('context.read')
    expect(props.onWritesContextChange).toHaveBeenCalledExactlyOnceWith('context.write')
    const oldErrorId = screen.getByTestId('node-reads-context-editor-error').id
    rerender(<NodeInspectorPanel {...props} readsContextDraft="context.read" writesContextError="Write error" />)
    expect(reads).not.toHaveAttribute('aria-invalid')
    expect(reads).toHaveAccessibleDescription(/One `context.\*` key per line/)
    expect(reads.getAttribute('aria-describedby')).not.toContain(oldErrorId)
    expect(document.getElementById(oldErrorId)).toBeNull()
    expect(writes).toHaveAccessibleDescription(/Write error/)
    expect(writes).toHaveAttribute('aria-invalid', 'true')
    expectLabelsAndReferences()
  })

  it('uses unique stable IDs for repeated context editors with the same test ID', () => {
    const props = { title: 'Reads Context', description: 'Read keys', value: '', error: null, testId: 'keys', onChange: vi.fn() }
    const { rerender } = render(<><ContextKeyListEditor {...props} /><ContextKeyListEditor {...props} /></>)
    const ids = screen.getAllByRole('textbox', { name: 'Reads Context' }).map(input => input.id)
    expectLabelsAndReferences()
    rerender(<><ContextKeyListEditor {...props} value="context.a" /><ContextKeyListEditor {...props} /></>)
    expect(screen.getAllByRole('textbox', { name: 'Reads Context' }).map(input => input.id)).toEqual(ids)
  })

  it('supports keyboard navigation and label-click focus on Implement subflow fields', async () => {
    const user = userEvent.setup()
    render(<NodeInspectorPanel {...panelProps('subflow')} />)
    await user.tab()
    expect(screen.getByRole('textbox', { name: 'Label', exact: true })).toHaveFocus()
    await user.tab()
    expect(screen.getByRole('combobox', { name: 'Node Kind' })).toHaveFocus()
    await user.tab()
    expect(screen.getByRole('textbox', { name: 'Reads Context' })).toHaveFocus()
    await user.tab()
    expect(screen.getByRole('textbox', { name: 'Writes Context' })).toHaveFocus()
    await user.tab()
    expect(screen.getByRole('textbox', { name: 'Child Flow Reference' })).toHaveFocus()
    for (const name of ['Label', 'Reads Context', 'Writes Context', 'Child Flow Reference', 'Manager Poll Interval', 'Manager Max Cycles', 'Manager Stop Condition', 'Manager Actions', 'Manager Steer Cooldown']) {
      await user.click(screen.getByText(name, { selector: 'label', exact: true }))
      expect(screen.getByRole('textbox', { name, exact: true })).toHaveFocus()
    }
  })

  it('retains extension edits and associates extension warnings', () => {
    const props = panelProps()
    props.selectedNodeExtensionEntries = [{ key: 'custom', value: 'old' }]
    render(<NodeInspectorPanel {...props} />)
    fireEvent.change(screen.getByRole('textbox', { name: 'Value', exact: true }), { target: { value: 'new' } })
    expect(props.onNodeExtensionValueChange).toHaveBeenCalledWith('custom', 'new')
    const key = screen.getByRole('textbox', { name: 'New Key' })
    fireEvent.change(key, { target: { value: 'custom' } })
    expect(key).toHaveAccessibleDescription('Key already exists.')
    expect(key).not.toHaveAttribute('aria-invalid')
    fireEvent.change(key, { target: { value: 'prompt' } })
    expect(key).toHaveAccessibleDescription('Core attributes belong in dedicated controls.')
    fireEvent.change(key, { target: { value: 'another' } })
    expect(key).not.toHaveAttribute('aria-describedby')
    fireEvent.change(screen.getByRole('textbox', { name: 'New Value' }), { target: { value: 'added' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add Attribute' }))
    expect(props.onNodeExtensionAdd).toHaveBeenCalledWith('another', 'added')
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }))
    expect(props.onNodeExtensionRemove).toHaveBeenCalledWith('custom')
    expectLabelsAndReferences()
  })
})
