import { useId, type ReactNode } from 'react'
import type { Node } from '@xyflow/react'

import { useLlmProfiles } from '@/lib/useLlmProfiles'
import { getLlmSelectionOptions, getModelSuggestions, splitLlmSelection } from '@/lib/llmSuggestions'
import type { DiagnosticEntry } from '@/store'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'
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
    onPropertyChange: (key: string, value: string | boolean) => void
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

const isTrue = (value: unknown) => value === true || value === 'true'
const isDefaultEnabledBoolean = (value: unknown) => value !== false && value !== 'false'
const NODE_KIND_OPTIONS = [
    { value: 'start', label: 'Start' },
    { value: 'exit', label: 'Exit' },
    { value: 'agent_task', label: 'Agent Task' },
    { value: 'human_gate', label: 'Human Gate' },
    { value: 'conditional', label: 'Conditional' },
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
    const llmProfiles = useLlmProfiles()
    const selectedProfile = (selectedNode?.data?.llm_profile as string) || ''
    const selectedProvider = (selectedNode?.data?.llm_provider as string) || ''
    const selectedJoinPolicy = (selectedNode?.data?.join_policy as string) || 'wait_all'
    return (
        <div className="flex-1 overflow-y-auto px-5 pb-5 pt-3">
            <InspectorScaffold
                scopeLabel="Node"
                title="Configuration"
                description="Use the same inspect-edit flow as graph and edge inspectors."
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
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-join-policy`}>Join Policy</Label>
                                    <NativeSelect
                                        id={`${id}-join-policy`}
                                        value={selectedJoinPolicy}
                                        onChange={(event) => onPropertyChange('join_policy', event.target.value)}
                                    >
                                        <option value="wait_all">Wait All</option>
                                        <option value="first_success">First Success</option>
                                        <option value="k_of_n">K of N</option>
                                        <option value="quorum">Quorum</option>
                                    </NativeSelect>
                                </div>
                                {selectedJoinPolicy === 'k_of_n' ? (
                                    <div className="space-y-1.5">
                                        <Label htmlFor={`${id}-k-threshold`}>K Threshold</Label>
                                        <Input
                                            id={`${id}-k-threshold`}
                                            data-testid="node-attr-input-join_k"
                                            value={(selectedNode?.data?.join_k as number | string | undefined) ?? ''}
                                            onChange={(event) => onPropertyChange('join_k', event.target.value)}
                                            placeholder="2"
                                        />
                                    </div>
                                ) : null}
                                {selectedJoinPolicy === 'quorum' ? (
                                    <div className="space-y-1.5">
                                        <Label htmlFor={`${id}-quorum-threshold`}>Quorum Threshold</Label>
                                        <Input
                                            id={`${id}-quorum-threshold`}
                                            data-testid="node-attr-input-join_quorum"
                                            value={(selectedNode?.data?.join_quorum as number | string | undefined) ?? ''}
                                            onChange={(event) => onPropertyChange('join_quorum', event.target.value)}
                                            placeholder="0.5"
                                        />
                                    </div>
                                ) : null}
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-error-policy`}>Error Policy</Label>
                                    <NativeSelect
                                        id={`${id}-error-policy`}
                                        value={(selectedNode?.data?.error_policy as string) || 'continue'}
                                        onChange={(event) => onPropertyChange('error_policy', event.target.value)}
                                    >
                                        <option value="continue">Continue</option>
                                        <option value="fail_fast">Fail Fast</option>
                                        <option value="ignore">Ignore</option>
                                    </NativeSelect>
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

                        {visibility.showManagerOptions ? (
                            <>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-child-flow-reference`}>Child Flow Reference</Label>
                                    <Input
                                        id={`${id}-child-flow-reference`}
                                        aria-describedby={`${id}-child-flow-help`}
                                        value={((selectedNode?.data?.flow_ref as string) || (selectedNode?.data?.['stack.child_flow_ref'] as string)) || ''}
                                        onChange={(event) => onPropertyChange('flow_ref', event.target.value)}
                                        placeholder="child.yaml"
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
                                <div
                                    data-testid="manager-child-linkage"
                                    className="space-y-2 rounded-md border border-border/80 bg-muted/20 px-3 py-2"
                                >
                                    <div>
                                        <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                            Child Flow Linkage
                                        </p>
                                        <p id={`${id}-child-flow-help`} className="mt-1 text-xs text-muted-foreground">
                                            Subflow nodes use this node's <code>flow_ref</code> config and optional input map.
                                        </p>
                                    </div>
                                    <div className="space-y-1 text-sm text-foreground">
                                        <p><span className="font-mono">flow_ref</span>: {((selectedNode?.data?.flow_ref as string) || '(unset)')}</p>
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
                                </div>
                            </>
                        ) : null}

                        {visibility.showAdvanced ? (
                            <Button
                                type="button"
                                variant="outline"
                                size="sm"
                                className="w-full text-xs font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
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
                                        <div className="flex items-center gap-2">
                                            <Checkbox
                                                id={`${id}-goal-gate`}
                                                aria-describedby={nodeFieldDiagnostics.goal_gate?.length ? `${id}-goal_gate-diagnostics` : undefined}
                                                aria-invalid={nodeFieldDiagnostics.goal_gate?.some((diagnostic) => diagnostic.severity === 'error') || undefined}
                                                checked={isTrue(selectedNode?.data?.goal_gate)}
                                                onCheckedChange={(checked) => onPropertyChange('goal_gate', checked === true)}
                                            />
                                            <Label htmlFor={`${id}-goal-gate`} className="text-sm font-medium">
                                                Goal Gate
                                            </Label>
                                        </div>
                                        {nodeFieldDiagnostics.goal_gate?.length ? (
                                            <div id={`${id}-goal_gate-diagnostics`}>
                                                {renderFieldDiagnostics('node', 'goal_gate', nodeFieldDiagnostics, 'node-field-diagnostics-goal_gate')}
                                            </div>
                                        ) : null}
                                        <div className="space-y-1.5">
                                            <Label htmlFor={`${id}-retry-target`}>Retry Target</Label>
                                            <Input
                                                id={`${id}-retry-target`}
                                                aria-describedby={nodeFieldDiagnostics.retry_target?.length ? `${id}-retry_target-diagnostics` : undefined}
                                                aria-invalid={nodeFieldDiagnostics.retry_target?.some((diagnostic) => diagnostic.severity === 'error') || undefined}
                                                value={(selectedNode?.data?.retry_target as string) || ''}
                                                onChange={(event) => onPropertyChange('retry_target', event.target.value)}
                                            />
                                            {nodeFieldDiagnostics.retry_target?.length ? (
                                                <div id={`${id}-retry_target-diagnostics`}>
                                                    {renderFieldDiagnostics('node', 'retry_target', nodeFieldDiagnostics, 'node-field-diagnostics-retry_target')}
                                                </div>
                                            ) : null}
                                        </div>
                                        <div className="space-y-1.5">
                                            <Label htmlFor={`${id}-fallback-retry-target`}>Fallback Retry Target</Label>
                                            <Input
                                                id={`${id}-fallback-retry-target`}
                                                aria-describedby={nodeFieldDiagnostics.fallback_retry_target?.length ? `${id}-fallback_retry_target-diagnostics` : undefined}
                                                aria-invalid={nodeFieldDiagnostics.fallback_retry_target?.some((diagnostic) => diagnostic.severity === 'error') || undefined}
                                                value={(selectedNode?.data?.fallback_retry_target as string) || ''}
                                                onChange={(event) => onPropertyChange('fallback_retry_target', event.target.value)}
                                            />
                                            {nodeFieldDiagnostics.fallback_retry_target?.length ? (
                                                <div id={`${id}-fallback_retry_target-diagnostics`}>
                                                    {renderFieldDiagnostics('node', 'fallback_retry_target', nodeFieldDiagnostics, 'node-field-diagnostics-fallback_retry_target')}
                                                </div>
                                            ) : null}
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
                                        <div className="grid grid-cols-2 gap-3">
                                            <div className="space-y-1.5">
                                                <Label htmlFor={`${id}-fidelity`}>Fidelity</Label>
                                                <Input
                                                    id={`${id}-fidelity`}
                                                    aria-describedby={nodeFieldDiagnostics.fidelity?.length ? `${id}-fidelity-diagnostics` : undefined}
                                                    aria-invalid={nodeFieldDiagnostics.fidelity?.some((diagnostic) => diagnostic.severity === 'error') || undefined}
                                                    value={(selectedNode?.data?.fidelity as string) || ''}
                                                    onChange={(event) => onPropertyChange('fidelity', event.target.value)}
                                                    placeholder="full"
                                                />
                                                {nodeFieldDiagnostics.fidelity?.length ? (
                                                    <div id={`${id}-fidelity-diagnostics`}>
                                                        {renderFieldDiagnostics('node', 'fidelity', nodeFieldDiagnostics, 'node-field-diagnostics-fidelity')}
                                                    </div>
                                                ) : null}
                                            </div>
                                            <div className="space-y-1.5">
                                                <Label htmlFor={`${id}-thread-id`}>Thread ID</Label>
                                                <Input
                                                    id={`${id}-thread-id`}
                                                    value={(selectedNode?.data?.thread_id as string) || ''}
                                                    onChange={(event) => onPropertyChange('thread_id', event.target.value)}
                                                />
                                            </div>
                                        </div>
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
                                    <>
                                        <div className="grid grid-cols-2 gap-3">
                                            <div className="space-y-1.5">
                                                <Label htmlFor={`${id}-llm-model`}>LLM Model</Label>
                                                <Input
                                                    id={`${id}-llm-model`}
                                                    value={(selectedNode?.data?.llm_model as string) || ''}
                                                    onChange={(event) => onPropertyChange('llm_model', event.target.value)}
                                                    list={`${id}-llm-model-options`}
                                                />
                                                <datalist id={`${id}-llm-model-options`}>
                                                    {getModelSuggestions(selectedProfile || selectedProvider, llmProfiles).map((model) => (
                                                        <option key={model} value={model} />
                                                    ))}
                                                </datalist>
                                            </div>
                                            <div className="space-y-1.5">
                                                <Label htmlFor={`${id}-llm-provider`}>LLM Provider</Label>
                                                <Input
                                                    id={`${id}-llm-provider`}
                                                    value={((selectedNode?.data?.llm_profile as string) || (selectedNode?.data?.llm_provider as string)) || ''}
                                                    onChange={(event) => {
                                                        const selection = splitLlmSelection(event.target.value, llmProfiles)
                                                        onPropertyChange('llm_provider', selection.llm_provider)
                                                        onPropertyChange('llm_profile', selection.llm_profile)
                                                    }}
                                                    list={`${id}-llm-provider-options`}
                                                />
                                                <datalist id={`${id}-llm-provider-options`}>
                                                    {getLlmSelectionOptions(llmProfiles).map((provider) => (
                                                        <option key={provider} value={provider} />
                                                    ))}
                                                </datalist>
                                            </div>
                                        </div>
                                        <div className="space-y-1.5">
                                            <Label htmlFor={`${id}-reasoning-effort`}>Reasoning Effort</Label>
                                            <Input
                                                id={`${id}-reasoning-effort`}
                                                value={(selectedNode?.data?.reasoning_effort as string) || ''}
                                                onChange={(event) => onPropertyChange('reasoning_effort', event.target.value)}
                                                placeholder="high"
                                            />
                                        </div>
                                    </>
                                ) : null}

                                {visibility.showGeneralAdvanced ? (
                                    <div className="flex items-center gap-4">
                                        <div className="flex items-center gap-2">
                                            <Checkbox
                                                id={`${id}-node-auto-status`}
                                                checked={isTrue(selectedNode?.data?.auto_status)}
                                                onCheckedChange={(checked) => onPropertyChange('auto_status', checked === true)}
                                            />
                                            <Label htmlFor={`${id}-node-auto-status`} className="text-sm font-medium">
                                                Auto Status
                                            </Label>
                                        </div>
                                        <div className="flex items-center gap-2">
                                            <Checkbox
                                                id={`${id}-node-allow-partial`}
                                                checked={isTrue(selectedNode?.data?.allow_partial)}
                                                onCheckedChange={(checked) => onPropertyChange('allow_partial', checked === true)}
                                            />
                                            <Label htmlFor={`${id}-node-allow-partial`} className="text-sm font-medium">
                                                Allow Partial
                                            </Label>
                                        </div>
                                    </div>
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
                                'decisions',
                                'prompt',
                                'tool.command',
                                'tool.hooks.pre',
                                'tool.hooks.post',
                                'tool.artifacts.paths',
                                'tool.artifacts.stdout',
                                'tool.artifacts.stderr',
                                'join_policy',
                                'join_k',
                                'join_quorum',
                                'error_policy',
                                'max_parallel',
                                'type',
                                'max_retries',
                                'goal_gate',
                                'retry_target',
                                'fallback_retry_target',
                                'fidelity',
                                'thread_id',
                                'class',
                                'timeout',
                                'llm_model',
                                'llm_provider',
                                'reasoning_effort',
                                'auto_status',
                                'allow_partial',
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
