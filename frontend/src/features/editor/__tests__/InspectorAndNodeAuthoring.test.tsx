import type { ComponentProps } from 'react'
import userEvent from '@testing-library/user-event'
import { ContextKeyListEditor } from '@/features/editor/components/ContextKeyListEditor'
import {
  resolveEdgeFieldDiagnostics,
  resolveNodeFieldDiagnostics,
} from '@/lib/inspectorFieldDiagnostics'
import { getHandlerType, getNodeFieldVisibility } from '@/lib/nodeVisibility'
import { NodeInspectorPanel } from '@/features/editor/components/NodeInspectorPanel'
import { applyNodePropertyChangeToData } from '@/features/editor/Sidebar'
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/components/model-chooser/useModelOptions', () => ({
    useModelOptions: () => ({ projectPath: null, payload: { models: [{ provider: 'openai', id: 'gpt-5.5', display: 'gpt-5.5', is_default: false, supported_reasoning_efforts: ['high'] }], providers: { codex: { status: 'available', error: null } } } }),
}))

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
          rule_id: 'switch_writer',
          severity: 'error',
          message: 'context.count has no upstream writer.',
          node_id: 'node_a',
        },
        {
          rule_id: 'fork_structure',
          severity: 'error',
          message: 'join must be a fan_in node.',
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

    expect(nodeDiagnostics.config).toHaveLength(2)
    expect(nodeDiagnostics.prompt).toHaveLength(1)
    expect(nodeDiagnostics.label).toHaveLength(1)
  })

  it('maps route diagnostics to the decision fields of the edge they name', () => {
    const diagnostics = [
      {
        rule_id: 'route_description',
        severity: 'error' as const,
        message: 'Route approve needs a description.',
        edge: ['review', 'done'] as [string, string],
        decision: 'approve',
      },
      {
        rule_id: 'decision_id',
        severity: 'error' as const,
        message: 'Decision Waive is not a valid decision ID.',
        edge: ['review', 'done'] as [string, string],
        decision: 'Waive',
      },
    ]

    const approve = resolveEdgeFieldDiagnostics(diagnostics, 'review', 'done', 'approve')
    expect(approve.description).toHaveLength(1)
    expect(approve.decision).toBeUndefined()
    const waive = resolveEdgeFieldDiagnostics(diagnostics, 'review', 'done', 'Waive')
    expect(waive.decision).toHaveLength(1)
    expect(waive.description).toBeUndefined()
  })

  it('renders fork structure and branch-error tolerance in the node inspector', () => {
    const props = panelProps('parallel')
    render(
      <NodeInspectorPanel
        {...props}
        selectedNodeFork={{ branches: ['lens_a', 'lens_b'], join: 'fan_in' }}
      />,
    )

    expect(screen.getByTestId('parallel-fork-structure')).toHaveTextContent('branches: lens_a, lens_b')
    expect(screen.getByTestId('parallel-fork-structure')).toHaveTextContent('join: fan_in')
    for (const legacy of ['Join Policy', 'K Threshold', 'Quorum Threshold', 'Error Policy']) {
      expect(screen.queryByText(legacy)).not.toBeInTheDocument()
    }
    fireEvent.click(screen.getByRole('checkbox', { name: 'Tolerate Branch Execution Errors' }))
    expect(props.onPropertyChange).toHaveBeenCalledWith('config.tolerate_branch_execution_errors', true)
  })

  it('edits switch, exit, tool and subflow config through typed config fields', () => {
    const switchProps = panelProps('switch', { config: { kind: 'switch', on: 'context.count', cases: { '0': 'zero', '"0"': 'text_zero' }, otherwise: 'other' } })
    const view = render(<NodeInspectorPanel {...switchProps} />)
    expect(screen.getByRole('textbox', { name: 'Switch On' })).toHaveValue('context.count')
    const cases = screen.getByRole('textbox', { name: 'Cases' })
    expect(cases).toHaveValue('0 = zero\n"0" = text_zero')
    fireEvent.change(cases, { target: { value: '0 = zero\n"0" = text_zero\ntrue = zero' } })
    expect(switchProps.onPropertyChange).toHaveBeenLastCalledWith('config.cases', { '0': 'zero', '"0"': 'text_zero', 'true': 'zero' })
    fireEvent.change(screen.getByRole('textbox', { name: 'Otherwise Decision' }), { target: { value: 'fallback' } })
    expect(switchProps.onPropertyChange).toHaveBeenLastCalledWith('config.otherwise', 'fallback')
    expectLabelsAndReferences()

    const exitProps = panelProps('exit', { config: { kind: 'exit', result: { name: 'blocked', outcome: 'failure' } } })
    view.rerender(<NodeInspectorPanel {...exitProps} />)
    expect(screen.getByRole('textbox', { name: 'Result Name' })).toHaveValue('blocked')
    expect(screen.getByRole('combobox', { name: 'Run Outcome' })).toHaveValue('failure')
    fireEvent.change(screen.getByRole('combobox', { name: 'Run Outcome' }), { target: { value: 'success' } })
    expect(exitProps.onPropertyChange).toHaveBeenLastCalledWith('config.result', { name: 'blocked', outcome: 'success' })

    const toolProps = panelProps('tool', { config: { kind: 'tool', command: 'make', exit_codes: { '0': 'passed', '1': 'rejected' } } })
    view.rerender(<NodeInspectorPanel {...toolProps} />)
    expect(screen.getByRole('textbox', { name: 'Exit Codes' })).toHaveValue('0 = passed\n1 = rejected')
    fireEvent.change(screen.getByRole('combobox', { name: 'Decision Adapter' }), { target: { value: 'decision_field' } })
    expect(toolProps.onPropertyChange).toHaveBeenCalledWith('config.exit_codes', '')
    expect(toolProps.onPropertyChange).toHaveBeenCalledWith('config.decision_field', 'decision')

    const subflowProps = panelProps('subflow', { config: { kind: 'subflow', flow_ref: 'child', results: { completed: 'child_completed' } } })
    view.rerender(<NodeInspectorPanel {...subflowProps} />)
    fireEvent.change(screen.getByRole('textbox', { name: 'Child Results' }), { target: { value: 'completed = child_completed\nblocked = child_blocked' } })
    expect(subflowProps.onPropertyChange).toHaveBeenLastCalledWith('config.results', { completed: 'child_completed', blocked: 'child_blocked' })
    expectLabelsAndReferences()
  })

  it('writes config fields into the typed config and resets it when the kind changes', () => {
    expect(
      applyNodePropertyChangeToData({ kind: 'switch', config: { kind: 'switch', on: 'context.a' } }, 'config.otherwise', 'other'),
    ).toEqual({ kind: 'switch', config: { kind: 'switch', on: 'context.a', otherwise: 'other' } })
    expect(
      applyNodePropertyChangeToData({ kind: 'parallel', config: { kind: 'parallel', tolerate_branch_execution_errors: true } }, 'config.tolerate_branch_execution_errors', false),
    ).toEqual({ kind: 'parallel', config: { kind: 'parallel' } })
    expect(
      applyNodePropertyChangeToData({ kind: 'agent_task', config: { kind: 'agent_task', prompt: 'Do it' } }, 'kind', 'switch'),
    ).toMatchObject({ kind: 'switch', shape: 'diamond', config: { kind: 'switch' } })
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
      config: { kind: 'subflow' },
      flow_ref: 'child',
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
        selectedNodeFork={{ branches: [], join: null }}
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

function panelProps(kind = 'agent_task', data: Record<string, unknown> = {}): ComponentProps<typeof NodeInspectorPanel> {
  return {
    selectedNodeId: 'node',
    selectedNode: { id: 'node', position: { x: 0, y: 0 }, data: { kind, label: 'Implement', ...data } },
    visibility: getNodeFieldVisibility(getHandlerType(kind)),
    readsContextDraft: '', readsContextError: null,
    writesContextDraft: '', writesContextError: null,
    showAdvanced: true, nodeFieldDiagnostics: {}, selectedNodeExtensionEntries: [],
    selectedNodeToolHookPreWarning: null, selectedNodeToolHookPostWarning: null,
    selectedNodeFork: { branches: [], join: null },
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
      ['Session', 'session'],
      ['Class', 'class']]],
    ['tool', [['Tool Command', 'tool.command'], ['Pre Hook Override', 'tool.hooks.pre'],
      ['Post Hook Override', 'tool.hooks.post'], ['Artifact Paths', 'tool.artifacts.paths'],
      ['Stdout Artifact', 'tool.artifacts.stdout'], ['Stderr Artifact', 'tool.artifacts.stderr']]],
    ['parallel', [['Max Parallel', 'max_parallel']]],
    ['subflow', [['Child Flow Reference', 'flow_ref'], ['Manager Poll Interval', 'manager.poll_interval'],
      ['Manager Max Cycles', 'manager.max_cycles'], ['Manager Stop Condition', 'manager.stop_condition'],
      ['Manager Actions', 'manager.actions'], ['Manager Steer Cooldown', 'manager.steer_cooldown']]],
  ])('edits named %s fields, including advanced settings', (kind, fields) => {
    const props = panelProps(kind)
    const view = render(<NodeInspectorPanel {...props} />)
    for (const [name, key] of [['Label', 'label'], ...fields]) {
      fireEvent.change(screen.getByRole('textbox', { name, exact: true }), { target: { value: 'edited' } })
      expect(props.onPropertyChange).toHaveBeenCalledWith(key, 'edited')
    }
    fireEvent.change(screen.getByRole('combobox', { name: 'Node Kind' }), { target: { value: 'tool' } })
    expect(props.onPropertyChange).toHaveBeenCalledWith('kind', 'tool')
    if (kind === 'agent_task') {
      fireEvent.click(screen.getByRole('button', { name: /^Model:/ }))
      fireEvent.click(within(screen.getByRole('group', { name: 'openai', exact: true })).getByRole('option', { name: 'gpt-5.5' }))
      expect(props.onPropertyChange).toHaveBeenCalledWith('llm_provider', 'openai')
      expect(props.onPropertyChange).toHaveBeenCalledWith('llm_model', 'gpt-5.5')
      // Effort applies to an explicitly chosen model, so the node must hold the choice first.
      view.rerender(<NodeInspectorPanel {...props} selectedNode={{ ...props.selectedNode!, data: { ...props.selectedNode!.data, llm_provider: 'openai', llm_model: 'gpt-5.5' } }} />)
      fireEvent.click(screen.getByRole('button', { name: 'High', exact: true }))
      expect(props.onPropertyChange).toHaveBeenCalledWith('reasoning_effort', 'high')
      for (const name of ['Goal Gate', 'Auto Status', 'Allow Partial', 'Retry Target', 'Fallback Retry Target']) {
        expect(screen.queryByLabelText(name)).not.toBeInTheDocument()
      }
    }
    expectLabelsAndReferences()
  })

  it('shows the stored session only on agent_task nodes', () => {
    const props = panelProps()
    const { rerender } = render(<NodeInspectorPanel {...props} selectedNode={{ ...props.selectedNode!, data: { ...props.selectedNode!.data, session: 'work' } }} />)
    expect(screen.getByRole('textbox', { name: 'Session' })).toHaveValue('work')
    for (const kind of ['tool', 'parallel', 'subflow', 'fan_in']) {
      rerender(<NodeInspectorPanel {...panelProps(kind)} />)
      expect(screen.queryByRole('textbox', { name: 'Session' })).not.toBeInTheDocument()
    }
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
    const error = { rule_id: 'response_contract', severity: 'error' as const, message: 'contracts.response must be decision_envelope.' }
    const warning = { rule_id: 'test', severity: 'warning' as const, message: 'Prompt is recommended.' }
    props.nodeFieldDiagnostics = { prompt: [warning], config: [error] }
    const { rerender } = render(<NodeInspectorPanel {...props} />)
    const prompt = screen.getByRole('textbox', { name: 'Prompt Instruction' })
    expect(prompt).toHaveAccessibleDescription(warning.message)
    expect(prompt).not.toHaveAttribute('aria-invalid')
    expect(screen.getByTestId('node-field-diagnostics-config')).toHaveTextContent(error.message)
    expect(props.renderFieldDiagnostics).toHaveBeenCalledWith('node', 'config', props.nodeFieldDiagnostics, 'node-field-diagnostics-config')
    expectLabelsAndReferences()
    rerender(<NodeInspectorPanel {...props} nodeFieldDiagnostics={{}} />)
    expect(prompt).not.toHaveAttribute('aria-describedby')
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
