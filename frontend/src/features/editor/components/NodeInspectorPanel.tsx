import { useStore } from '@/store'
import { useEffect, useId, useState, type ReactNode } from 'react'
import type { Node } from '@xyflow/react'

import { ModelChooser } from '@/components/model-chooser/ModelChooser'
import { Well } from '@/components/app/well'
import type { DiagnosticEntry } from '@/store'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'
import { DEFAULT_EXIT_RESULT } from '@/lib/canonicalFlowModel'
import { formatDecisionMap, parseDecisionMap } from '@/lib/flowContracts'
import { setInvalidDraft } from '@/lib/flowPersistence'
import type { NodePropertyValue } from '../Sidebar'
import { AdvancedKeyValueEditor } from './AdvancedKeyValueEditor'
import { ContextKeyListEditor } from './ContextKeyListEditor'
import { InspectorEmptyState, InspectorScaffold } from './InspectorScaffold'

type VisibilityConfig = {
    showPrompt: boolean
    showToolCommand: boolean
    showParallelOptions: boolean
    showManagerOptions: boolean
    showAdvanced: boolean
    showGeneralAdvanced: boolean
    showLlmSettings: boolean
}

type ExtensionEntry = {
    key: string
    value: string
}

interface NodeInspectorPanelProps {
    selectedNodeId: string | null
    selectedNode?: Node
    visibility: VisibilityConfig
    readsContextDraft: string
    readsContextError: string | null
    writesContextDraft: string
    writesContextError: string | null
    showAdvanced: boolean
    nodeFieldDiagnostics: Record<string, DiagnosticEntry[]>
    selectedNodeExtensionEntries: ExtensionEntry[]
    selectedNodeToolHookPreWarning: string | null
    selectedNodeToolHookPostWarning: string | null
    selectedNodeFork: { branches: string[]; join: string | null }
    onPropertyChange: (key: string, value: NodePropertyValue) => void
    onOpenGraphChildSettings: () => void
    onReadsContextChange: (value: string) => void
    onWritesContextChange: (value: string) => void
    onSetShowAdvanced: (value: boolean | ((current: boolean) => boolean)) => void
    onNodeExtensionValueChange: (key: string, value: string) => void
    onNodeExtensionRemove: (key: string) => void
    onNodeExtensionAdd: (key: string, value: string) => void
    renderFieldDiagnostics: (
        scope: 'node' | 'edge',
        field: string,
        fieldDiagnostics: Record<string, DiagnosticEntry[]>,
        testId: string,
    ) => ReactNode
}

const isDefaultEnabledBoolean = (value: unknown) => value !== false && value !== 'false'
const NODE_KIND_OPTIONS = [
    { value: 'start', label: 'Start' },
    { value: 'exit', label: 'Exit' },
    { value: 'agent_task', label: 'Agent Task' },
    { value: 'human_gate', label: 'Human Gate' },
    { value: 'switch', label: 'Switch' },
    { value: 'parallel', label: 'Parallel' },
    { value: 'fan_in', label: 'Fan In' },
    { value: 'tool', label: 'Tool' },
    { value: 'subflow', label: 'Subflow' },
]

export function NodeInspectorPanel({
    selectedNodeId,
    selectedNode,
    visibility,
    readsContextDraft,
    readsContextError,
    writesContextDraft,
    writesContextError,
    showAdvanced,
    nodeFieldDiagnostics,
    selectedNodeExtensionEntries,
    selectedNodeToolHookPreWarning,
    selectedNodeToolHookPostWarning,
    selectedNodeFork,
    onPropertyChange,
    onOpenGraphChildSettings,
    onReadsContextChange,
    onWritesContextChange,
    onSetShowAdvanced,
    onNodeExtensionValueChange,
    onNodeExtensionRemove,
    onNodeExtensionAdd,
    renderFieldDiagnostics,
}: NodeInspectorPanelProps) {
    const id = useId()
    const flowMetadata = useStore((state) => state.flowMetadata)
    const kind = (selectedNode?.data?.kind as string) || 'agent_task'
    const config = (selectedNode?.data?.config && typeof selectedNode.data.config === 'object'
        ? selectedNode.data.config
        : {}) as Record<string, unknown>
    const exitResult = (config.result && typeof config.result === 'object'
        ? config.result
        : DEFAULT_EXIT_RESULT) as { name?: string; outcome?: string }
    const toolAdapter = typeof config.decision_field === 'string' ? 'decision_field' : 'exit_codes'
    return (
        <div className="flex-1 overflow-y-auto px-5 pb-5 pt-3">
            <InspectorScaffold
                scopeLabel="Node"
                title="Configuration"
                entityLabel="Node ID"
                entityValue={selectedNodeId || undefined}
            >
                {!selectedNodeId ? (
                    <InspectorEmptyState message="Select a component on the canvas to inspect and edit its properties." />
                ) : (
                    <div data-testid="node-structured-form" className="space-y-4">
                        <div className="space-y-1.5">
                            <Label htmlFor={`${id}-label`}>Label</Label>
                            <Input
                                id={`${id}-label`}
                                value={(selectedNode?.data?.label as string) || ''}
                                onChange={(event) => onPropertyChange('label', event.target.value)}
                            />
                        </div>

                        <div className="space-y-1.5">
                            <Label htmlFor={`${id}-node-kind`}>Node Kind</Label>
                            <NativeSelect
                                id={`${id}-node-kind`}
                                value={(selectedNode?.data?.kind as string) || 'agent_task'}
                                onChange={(event) => onPropertyChange('kind', event.target.value)}
                            >
                                {NODE_KIND_OPTIONS.map((option) => (
                                    <option key={option.value} value={option.value}>
                                        {option.label}
                                    </option>
                                ))}
                            </NativeSelect>
                        </div>

                        {visibility.showPrompt ? (
                            <div className="flex h-48 flex-col space-y-1.5">
                                <Label htmlFor={`${id}-prompt-instruction`}>Prompt Instruction</Label>
                                <Textarea
                                    id={`${id}-prompt-instruction`}
                                    aria-describedby={nodeFieldDiagnostics.prompt?.length ? `${id}-prompt-diagnostics` : undefined}
                                    aria-invalid={nodeFieldDiagnostics.prompt?.some((diagnostic) => diagnostic.severity === 'error') || undefined}
                                    value={(selectedNode?.data?.prompt as string) || ''}
                                    onChange={(event) => onPropertyChange('prompt', event.target.value)}
                                    className="flex-1 resize-none font-mono text-sm"
                                    placeholder="Enter system prompt instructions..."
                                />
                                {nodeFieldDiagnostics.prompt?.length ? (
                                    <div id={`${id}-prompt-diagnostics`}>
                                        {renderFieldDiagnostics('node', 'prompt', nodeFieldDiagnostics, 'node-field-diagnostics-prompt')}
                                    </div>
                                ) : null}
                            </div>
                        ) : null}

                        {(selectedNode?.data?.shape as string) !== 'Mdiamond' && (selectedNode?.data?.shape as string) !== 'Msquare' ? (
                            <div className="space-y-3">
                                <ContextKeyListEditor
                                    testId="node-reads-context-editor"
                                    title="Reads Context"
                                    description="Declare the `context.*` keys this node expects to consume from launch state or earlier stages."
                                    value={readsContextDraft}
                                    error={readsContextError}
                                    onChange={onReadsContextChange}
                                />
                                <ContextKeyListEditor
                                    testId="node-writes-context-editor"
                                    title="Writes Context"
                                    description="Declare the `context.*` keys this node is expected to produce for later stages."
                                    value={writesContextDraft}
                                    error={writesContextError}
                                    onChange={onWritesContextChange}
                                />
                            </div>
                        ) : null}

                        {visibility.showToolCommand ? (
                            <div className="space-y-1.5">
                                <Label htmlFor={`${id}-tool-command`}>Tool Command</Label>
                                <Input
                                    id={`${id}-tool-command`}
                                    value={(selectedNode?.data?.['tool.command'] as string) || ''}
                                    onChange={(event) => onPropertyChange('tool.command', event.target.value)}
                                    className="font-mono text-sm"
                                    placeholder="e.g. cargo test -p spark-cli"
                                />
                            </div>
                        ) : null}

                        {visibility.showParallelOptions ? (
                            <>
                                <Well data-testid="parallel-fork-structure" className="space-y-1 px-3 py-3 text-sm">
                                    <p className="font-semibold text-foreground">Fork structure</p>
                                    <p className="break-all"><span className="font-mono">branches</span>: {selectedNodeFork.branches.join(', ') || '(none)'}</p>
                                    <p className="break-all"><span className="font-mono">join</span>: {selectedNodeFork.join || '(unset)'}</p>
                                    <p className="text-xs text-muted-foreground">Edit by connecting edges from this node; mark the join edge in its inspector.</p>
                                </Well>
                                <div className="flex items-center gap-2">
                                    <Checkbox
                                        id={`${id}-tolerate-branch-errors`}
                                        checked={config.tolerate_branch_execution_errors === true}
                                        onCheckedChange={(checked) => onPropertyChange('config.tolerate_branch_execution_errors', checked === true)}
                                    />
                                    <Label htmlFor={`${id}-tolerate-branch-errors`} className="text-sm font-medium">
                                        Tolerate Branch Execution Errors
                                    </Label>
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-max-parallel`}>Max Parallel</Label>
                                    <Input
                                        id={`${id}-max-parallel`}
                                        value={(selectedNode?.data?.max_parallel as number | string | undefined) ?? 4}
                                        onChange={(event) => onPropertyChange('max_parallel', event.target.value)}
                                    />
                                </div>
                            </>
                        ) : null}

                        {kind === 'switch' ? (
                            <>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-switch-on`}>Switch On</Label>
                                    <Input
                                        id={`${id}-switch-on`}
                                        value={typeof config.on === 'string' ? config.on : ''}
                                        onChange={(event) => onPropertyChange('config.on', event.target.value)}
                                        className="font-mono text-sm"
                                        placeholder="context.example.key"
                                    />
                                </div>
                                <DecisionMapEditor
                                    key={`${selectedNodeId}-cases`}
                                    label="Cases"
                                    help={'One `value = decision` per line. Values are JSON scalars: 0 is a number, "0" a string.'}
                                    value={config.cases}
                                    onChange={(map) => onPropertyChange('config.cases', map)}
                                />
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-switch-otherwise`}>Otherwise Decision</Label>
                                    <Input
                                        id={`${id}-switch-otherwise`}
                                        value={typeof config.otherwise === 'string' ? config.otherwise : ''}
                                        onChange={(event) => onPropertyChange('config.otherwise', event.target.value)}
                                        className="font-mono text-sm"
                                    />
                                </div>
                            </>
                        ) : null}

                        {kind === 'exit' ? (
                            <>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-exit-result-name`}>Result Name</Label>
                                    <Input
                                        id={`${id}-exit-result-name`}
                                        value={exitResult.name ?? ''}
                                        onChange={(event) => onPropertyChange('config.result', { ...exitResult, name: event.target.value })}
                                        className="font-mono text-sm"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-exit-result-outcome`}>Run Outcome</Label>
                                    <NativeSelect
                                        id={`${id}-exit-result-outcome`}
                                        value={exitResult.outcome === 'failure' ? 'failure' : 'success'}
                                        onChange={(event) => onPropertyChange('config.result', { ...exitResult, outcome: event.target.value })}
                                    >
                                        <option value="success">Success</option>
                                        <option value="failure">Failure</option>
                                    </NativeSelect>
                                </div>
                            </>
                        ) : null}

                        {kind === 'tool' ? (
                            <>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-tool-adapter`}>Decision Adapter</Label>
                                    <NativeSelect
                                        id={`${id}-tool-adapter`}
                                        value={toolAdapter}
                                        onChange={(event) => {
                                            if (event.target.value === 'decision_field') {
                                                onPropertyChange('config.exit_codes', '')
                                                onPropertyChange('config.decision_field', 'decision')
                                            } else {
                                                onPropertyChange('config.decision_field', '')
                                            }
                                        }}
                                    >
                                        <option value="exit_codes">Exit codes</option>
                                        <option value="decision_field">JSON decision field</option>
                                    </NativeSelect>
                                </div>
                                {toolAdapter === 'decision_field' ? (
                                    <div className="space-y-1.5">
                                        <Label htmlFor={`${id}-tool-decision-field`}>Decision Field</Label>
                                        <Input
                                            id={`${id}-tool-decision-field`}
                                            value={config.decision_field as string}
                                            onChange={(event) => onPropertyChange('config.decision_field', event.target.value)}
                                            className="font-mono text-sm"
                                        />
                                    </div>
                                ) : (
                                    <DecisionMapEditor
                                        key={`${selectedNodeId}-exit-codes`}
                                        label="Exit Codes"
                                        help="One `exit code = decision` per line."
                                        value={config.exit_codes}
                                        onChange={(map) => onPropertyChange('config.exit_codes', map)}
                                    />
                                )}
                            </>
                        ) : null}

                        {nodeFieldDiagnostics.config?.length
                            ? renderFieldDiagnostics('node', 'config', nodeFieldDiagnostics, 'node-field-diagnostics-config')
                            : null}

                        {visibility.showManagerOptions ? (
                            <>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-child-flow-reference`}>Child Flow Reference</Label>
                                    <Input
                                        id={`${id}-child-flow-reference`}
                                        aria-describedby={`${id}-child-flow-help`}
                                        value={((selectedNode?.data?.flow_ref as string) || (selectedNode?.data?.['stack.child_flow_ref'] as string)) || ''}
                                        onChange={(event) => onPropertyChange('flow_ref', event.target.value)}
                                        placeholder="child"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-manager-poll-interval`}>Manager Poll Interval</Label>
                                    <Input
                                        id={`${id}-manager-poll-interval`}
                                        value={(selectedNode?.data?.['manager.poll_interval'] as string) || ''}
                                        onChange={(event) => onPropertyChange('manager.poll_interval', event.target.value)}
                                        placeholder="25ms"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-manager-max-cycles`}>Manager Max Cycles</Label>
                                    <Input
                                        id={`${id}-manager-max-cycles`}
                                        value={(selectedNode?.data?.['manager.max_cycles'] as number | string | undefined) ?? ''}
                                        onChange={(event) => onPropertyChange('manager.max_cycles', event.target.value)}
                                        placeholder="3"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-manager-stop-condition`}>Manager Stop Condition</Label>
                                    <Input
                                        id={`${id}-manager-stop-condition`}
                                        value={(selectedNode?.data?.['manager.stop_condition'] as string) || ''}
                                        onChange={(event) => onPropertyChange('manager.stop_condition', event.target.value)}
                                        placeholder='child.outcome == "success"'
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-manager-actions`}>Manager Actions</Label>
                                    <Input
                                        id={`${id}-manager-actions`}
                                        value={(selectedNode?.data?.['manager.actions'] as string) || ''}
                                        onChange={(event) => onPropertyChange('manager.actions', event.target.value)}
                                        placeholder="observe,steer"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-manager-steer-cooldown`}>Manager Steer Cooldown</Label>
                                    <Input
                                        id={`${id}-manager-steer-cooldown`}
                                        data-testid="node-attr-input-manager.steer_cooldown"
                                        value={(selectedNode?.data?.['manager.steer_cooldown'] as string) || ''}
                                        onChange={(event) => onPropertyChange('manager.steer_cooldown', event.target.value)}
                                        placeholder="2s"
                                    />
                                </div>
                                <div className="flex items-center gap-2">
                                    <Checkbox
                                        id={`${id}-stack-child-autostart`}
                                        data-testid="node-attr-checkbox-stack.child_autostart"
                                        checked={isDefaultEnabledBoolean(selectedNode?.data?.['stack.child_autostart'])}
                                        onCheckedChange={(checked) => onPropertyChange('stack.child_autostart', checked === true)}
                                    />
                                    <Label htmlFor={`${id}-stack-child-autostart`} className="text-sm font-medium">
                                        Start Child Automatically
                                    </Label>
                                </div>
                                <Well data-testid="manager-child-linkage" className="space-y-2 px-3 py-3">
                                    <div>
                                        <p className="text-sm font-semibold text-foreground">
                                            Child flow linkage
                                        </p>
                                        <p id={`${id}-child-flow-help`} className="mt-1 text-xs text-muted-foreground">
                                            Subflow nodes use this node's <code>flow_ref</code> config and optional input map.
                                        </p>
                                    </div>
                                    <div className="space-y-1 text-sm text-foreground">
                                        <p className="break-all"><span className="font-mono">flow_ref</span>: {((selectedNode?.data?.flow_ref as string) || '(unset)')}</p>
                                    </div>
                                    <Button
                                        type="button"
                                        data-testid="manager-open-child-settings"
                                        variant="outline"
                                        size="xs"
                                        onClick={onOpenGraphChildSettings}
                                    >
                                        Open Flow Settings
                                    </Button>
                                </Well>
                            </>
                        ) : null}

                        {kind === 'subflow' ? (
                            <DecisionMapEditor
                                key={`${selectedNodeId}-results`}
                                label="Child Results"
                                help="One `child result name = decision` per line."
                                value={config.results}
                                onChange={(map) => onPropertyChange('config.results', map)}
                            />
                        ) : null}

                        {visibility.showAdvanced ? (
                            <Button
                                type="button"
                                variant="outline"
                                size="sm"
                                className="w-full text-xs"
                                onClick={() => onSetShowAdvanced((current) => !current)}
                            >
                                {showAdvanced ? 'Hide Advanced' : 'Show Advanced'}
                            </Button>
                        ) : null}

                        {visibility.showAdvanced && showAdvanced ? (
                            <div className="space-y-4">
                                {visibility.showGeneralAdvanced ? (
                                    <>
                                        <div className="grid grid-cols-2 gap-3">
                                            <div className="space-y-1.5">
                                                <Label htmlFor={`${id}-max-retries`}>Max Retries</Label>
                                                <Input
                                                    id={`${id}-max-retries`}
                                                    value={(selectedNode?.data?.max_retries as number | string | undefined) ?? ''}
                                                    onChange={(event) => onPropertyChange('max_retries', event.target.value)}
                                                />
                                            </div>
                                            <div className="space-y-1.5">
                                                <Label htmlFor={`${id}-timeout`}>Timeout</Label>
                                                <Input
                                                    id={`${id}-timeout`}
                                                    value={(selectedNode?.data?.timeout as string) || ''}
                                                    onChange={(event) => onPropertyChange('timeout', event.target.value)}
                                                    placeholder="900s"
                                                />
                                            </div>
                                        </div>
                                        {visibility.showToolCommand ? (
                                            <>
                                                <div className="space-y-1.5">
                                                    <Label htmlFor={`${id}-pre-hook-override`}>Pre Hook Override</Label>
                                                    <Input
                                                        id={`${id}-pre-hook-override`}
                                                        aria-describedby={selectedNodeToolHookPreWarning ? `${id}-pre-hook-warning` : undefined}
                                                        data-testid="node-attr-input-tool.hooks.pre"
                                                        value={(selectedNode?.data?.['tool.hooks.pre'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.hooks.pre', event.target.value)}
                                                        className="font-mono text-sm"
                                                        placeholder="e.g. ./hooks/pre.sh"
                                                    />
                                                    {selectedNodeToolHookPreWarning ? (
                                                        <p id={`${id}-pre-hook-warning`} data-testid="node-attr-warning-tool.hooks.pre" className="text-xs text-warning">
                                                            {selectedNodeToolHookPreWarning}
                                                        </p>
                                                    ) : null}
                                                </div>
                                                <div className="space-y-1.5">
                                                    <Label htmlFor={`${id}-post-hook-override`}>Post Hook Override</Label>
                                                    <Input
                                                        id={`${id}-post-hook-override`}
                                                        aria-describedby={selectedNodeToolHookPostWarning ? `${id}-post-hook-warning` : undefined}
                                                        data-testid="node-attr-input-tool.hooks.post"
                                                        value={(selectedNode?.data?.['tool.hooks.post'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.hooks.post', event.target.value)}
                                                        className="font-mono text-sm"
                                                        placeholder="e.g. ./hooks/post.sh"
                                                    />
                                                    {selectedNodeToolHookPostWarning ? (
                                                        <p id={`${id}-post-hook-warning`} data-testid="node-attr-warning-tool.hooks.post" className="text-xs text-warning">
                                                            {selectedNodeToolHookPostWarning}
                                                        </p>
                                                    ) : null}
                                                </div>
                                                <div className="space-y-1.5">
                                                    <Label htmlFor={`${id}-artifact-paths`}>Artifact Paths</Label>
                                                    <Input
                                                        id={`${id}-artifact-paths`}
                                                        data-testid="node-attr-input-tool.artifacts.paths"
                                                        value={(selectedNode?.data?.['tool.artifacts.paths'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.artifacts.paths', event.target.value)}
                                                        className="font-mono text-sm"
                                                        placeholder="e.g. dist/**,reports/*.json"
                                                    />
                                                </div>
                                                <div className="space-y-1.5">
                                                    <Label htmlFor={`${id}-stdout-artifact`}>Stdout Artifact</Label>
                                                    <Input
                                                        id={`${id}-stdout-artifact`}
                                                        data-testid="node-attr-input-tool.artifacts.stdout"
                                                        value={(selectedNode?.data?.['tool.artifacts.stdout'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.artifacts.stdout', event.target.value)}
                                                        className="font-mono text-sm"
                                                        placeholder="e.g. stdout.txt"
                                                    />
                                                </div>
                                                <div className="space-y-1.5">
                                                    <Label htmlFor={`${id}-stderr-artifact`}>Stderr Artifact</Label>
                                                    <Input
                                                        id={`${id}-stderr-artifact`}
                                                        data-testid="node-attr-input-tool.artifacts.stderr"
                                                        value={(selectedNode?.data?.['tool.artifacts.stderr'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.artifacts.stderr', event.target.value)}
                                                        className="font-mono text-sm"
                                                        placeholder="e.g. stderr.txt"
                                                    />
                                                </div>
                                            </>
                                        ) : null}
                                        {selectedNode?.data?.kind === 'agent_task' ? (
                                            <div className="space-y-1.5">
                                                <Label htmlFor={`${id}-session`}>Session</Label>
                                                <Input
                                                    id={`${id}-session`}
                                                    value={(selectedNode?.data?.session as string) || ''}
                                                    onChange={(event) => onPropertyChange('session', event.target.value)}
                                                    placeholder="Leave empty for a fresh session"
                                                />
                                            </div>
                                        ) : null}
                                        <div className="space-y-1.5">
                                            <Label htmlFor={`${id}-class`}>Class</Label>
                                            <Input
                                                id={`${id}-class`}
                                                value={(selectedNode?.data?.class as string) || ''}
                                                onChange={(event) => onPropertyChange('class', event.target.value)}
                                            />
                                        </div>
                                    </>
                                ) : null}

                                {visibility.showLlmSettings ? (
                                    <ModelChooser projectPath={null} inheritLabel="Graph default"
                                            inherited={{ provider: flowMetadata.llm_provider || null, llm_profile: flowMetadata.llm_profile || null, model: flowMetadata.llm_model || null, reasoning_effort: flowMetadata.reasoning_effort || null, thinking: flowMetadata.thinking || null, thinking_budget_tokens: Number(flowMetadata.thinking_budget_tokens) || null, reasoning_mode: flowMetadata.reasoning_mode || null, reasoning_summary: flowMetadata.reasoning_summary || null }}
                                        value={{ provider: (selectedNode?.data?.llm_provider as string) || null,
                                            llm_profile: (selectedNode?.data?.llm_profile as string) || null,
                                            model: (selectedNode?.data?.llm_model as string) || null,
                                            reasoning_effort: (selectedNode?.data?.reasoning_effort as string) || null, thinking: (selectedNode?.data?.thinking as string) || null, thinking_budget_tokens: Number(selectedNode?.data?.thinking_budget_tokens) || null, reasoning_mode: (selectedNode?.data?.reasoning_mode as string) || null, reasoning_summary: (selectedNode?.data?.reasoning_summary as string) || null }}
                                        onChange={(value) => {
                                            onPropertyChange('llm_provider', value.provider || '')
                                            onPropertyChange('llm_profile', value.llm_profile || '')
                                            onPropertyChange('llm_model', value.model || '')
                                            onPropertyChange('reasoning_effort', value.reasoning_effort || '')
                        onPropertyChange('thinking', value.thinking == null ? '' : String(value.thinking))
                        onPropertyChange('thinking_budget_tokens', value.thinking_budget_tokens == null ? '' : String(value.thinking_budget_tokens))
                        onPropertyChange('reasoning_mode', value.reasoning_mode == null ? '' : String(value.reasoning_mode))
                        onPropertyChange('reasoning_summary', value.reasoning_summary == null ? '' : String(value.reasoning_summary))

                                        }} />
                                ) : null}

                            </div>
                        ) : null}

                        <AdvancedKeyValueEditor
                            testIdPrefix="node"
                            entries={selectedNodeExtensionEntries}
                            onValueChange={onNodeExtensionValueChange}
                            onRemove={onNodeExtensionRemove}
                            onAdd={onNodeExtensionAdd}
                            reservedKeys={new Set([
                                'label',
                                'kind',
                                'config',
                                'context',
                                'contracts',
                                'runtime',
                                'manager',
                                'retry',
                                'execution',
                                'ui',
                                'extensions',
                                'shape',
                                'flow_ref',
                                'input_map',
                                'output_map',
                                'routes',
                                'prompt',
                                'tool.command',
                                'tool.hooks.pre',
                                'tool.hooks.post',
                                'tool.artifacts.paths',
                                'tool.artifacts.stdout',
                                'tool.artifacts.stderr',
                                'max_parallel',
                                'type',
                                'max_retries',
                                'session',
                                'class',
                                'timeout',
                                'llm_model',
                                'llm_provider',
                                'reasoning_effort',
                                'manager.poll_interval',
                                'manager.max_cycles',
                                'manager.stop_condition',
                                'manager.actions',
                                'manager.steer_cooldown',
                                'stack.child_autostart',
                                'human.default_choice',
                                'spark.reads_context',
                                'spark.writes_context',
                            ])}
                        />
                    </div>
                )}
            </InspectorScaffold>
        </div>
    )
}

// Edits a `key = decision` map as text. The draft is local so a line being
// typed is not lost; the node config changes only while the whole draft is
// valid, and an invalid draft blocks saving until it is fixed.
function DecisionMapEditor({
    label,
    help,
    value,
    onChange,
}: {
    label: string
    help: string
    value: unknown
    onChange: (map: Record<string, string>) => void
}) {
    const id = useId()
    const [draft, setDraft] = useState(() => formatDecisionMap(value))
    const errors = parseDecisionMap(draft).errors
    const error = errors.length > 0 ? `${label}: ${errors.join(' ')}` : null
    useEffect(() => {
        setInvalidDraft(id, error)
        return () => setInvalidDraft(id, null)
    }, [id, error])
    return (
        <div className="space-y-1.5">
            <Label htmlFor={`${id}-map`}>{label}</Label>
            <Textarea
                id={`${id}-map`}
                aria-describedby={error ? `${id}-help ${id}-error` : `${id}-help`}
                aria-invalid={error ? true : undefined}
                value={draft}
                onChange={(event) => {
                    setDraft(event.target.value)
                    const parsed = parseDecisionMap(event.target.value)
                    if (parsed.errors.length === 0) onChange(parsed.map)
                }}
                rows={3}
                className="font-mono text-sm"
            />
            <p id={`${id}-help`} className="text-xs text-muted-foreground">{help}</p>
            {error ? (
                <p id={`${id}-error`} role="alert" className="text-xs text-destructive">
                    {errors.join(' ')} Not saved until fixed.
                </p>
            ) : null}
        </div>
    )
}
