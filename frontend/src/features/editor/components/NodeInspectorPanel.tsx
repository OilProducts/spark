import { useEffect, useState, type ReactNode } from 'react'
import type { Node } from '@xyflow/react'

import { fetchLlmProfiles } from '@/lib/api/llmProfilesApi'
import { getLlmSelectionOptions, getModelSuggestions, splitLlmSelection, type LlmProfileMetadata } from '@/lib/llmSuggestions'
import { WORKFLOW_NODE_SHAPE_OPTIONS } from '@/lib/workflowNodeShape'
import type { DiagnosticEntry, GraphAttrs } from '@/store'
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
    showHumanDefaultChoice: boolean
    showTypeOverride: boolean
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
    graphAttrs: GraphAttrs
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
    selectedNodeShapeTypeMismatchWarning: string | null
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

export function NodeInspectorPanel({
    selectedNodeId,
    selectedNode,
    graphAttrs,
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
    selectedNodeShapeTypeMismatchWarning,
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
    const [llmProfiles, setLlmProfiles] = useState<LlmProfileMetadata[]>([])
    useEffect(() => {
        void fetchLlmProfiles().then(setLlmProfiles)
    }, [])
    const selectedProfile = (selectedNode?.data?.llm_profile as string) || ''
    const selectedProvider = (selectedNode?.data?.llm_provider as string) || ''
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
                            <Label>Label</Label>
                            <Input
                                value={(selectedNode?.data?.label as string) || ''}
                                onChange={(event) => onPropertyChange('label', event.target.value)}
                            />
                        </div>

                        <div className="space-y-1.5">
                            <Label>Shape / Type</Label>
                            <NativeSelect
                                value={(selectedNode?.data?.shape as string) || 'box'}
                                onChange={(event) => onPropertyChange('shape', event.target.value)}
                            >
                                {WORKFLOW_NODE_SHAPE_OPTIONS.map((option) => (
                                    <option key={option.value} value={option.value}>
                                        {option.label}
                                    </option>
                                ))}
                            </NativeSelect>
                        </div>

                        {visibility.showPrompt ? (
                            <div className="flex h-48 flex-col space-y-1.5">
                                <Label>Prompt Instruction</Label>
                                <Textarea
                                    value={(selectedNode?.data?.prompt as string) || ''}
                                    onChange={(event) => onPropertyChange('prompt', event.target.value)}
                                    className="flex-1 resize-none font-mono text-xs"
                                    placeholder="Enter system prompt instructions..."
                                />
                                {renderFieldDiagnostics('node', 'prompt', nodeFieldDiagnostics, 'node-field-diagnostics-prompt')}
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
                                <Label>Tool Command</Label>
                                <Input
                                    value={(selectedNode?.data?.['tool.command'] as string) || ''}
                                    onChange={(event) => onPropertyChange('tool.command', event.target.value)}
                                    className="font-mono text-xs"
                                    placeholder="e.g. pytest -q"
                                />
                            </div>
                        ) : null}

                        {visibility.showParallelOptions ? (
                            <>
                                <div className="space-y-1.5">
                                    <Label>Join Policy</Label>
                                    <NativeSelect
                                        value={(selectedNode?.data?.join_policy as string) || 'wait_all'}
                                        onChange={(event) => onPropertyChange('join_policy', event.target.value)}
                                    >
                                        <option value="wait_all">Wait All</option>
                                        <option value="first_success">First Success</option>
                                        <option value="k_of_n">K of N</option>
                                        <option value="quorum">Quorum</option>
                                    </NativeSelect>
                                </div>
                                <div className="space-y-1.5">
                                    <Label>Error Policy</Label>
                                    <NativeSelect
                                        value={(selectedNode?.data?.error_policy as string) || 'continue'}
                                        onChange={(event) => onPropertyChange('error_policy', event.target.value)}
                                    >
                                        <option value="continue">Continue</option>
                                        <option value="fail_fast">Fail Fast</option>
                                        <option value="ignore">Ignore</option>
                                    </NativeSelect>
                                </div>
                                <div className="space-y-1.5">
                                    <Label>Max Parallel</Label>
                                    <Input
                                        value={(selectedNode?.data?.max_parallel as number | string | undefined) ?? 4}
                                        onChange={(event) => onPropertyChange('max_parallel', event.target.value)}
                                    />
                                </div>
                            </>
                        ) : null}

                        {visibility.showManagerOptions ? (
                            <>
                                <div className="space-y-1.5">
                                    <Label>Manager Poll Interval</Label>
                                    <Input
                                        value={(selectedNode?.data?.['manager.poll_interval'] as string) || ''}
                                        onChange={(event) => onPropertyChange('manager.poll_interval', event.target.value)}
                                        placeholder="25ms"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <Label>Manager Max Cycles</Label>
                                    <Input
                                        value={(selectedNode?.data?.['manager.max_cycles'] as number | string | undefined) ?? ''}
                                        onChange={(event) => onPropertyChange('manager.max_cycles', event.target.value)}
                                        placeholder="3"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <Label>Manager Stop Condition</Label>
                                    <Input
                                        value={(selectedNode?.data?.['manager.stop_condition'] as string) || ''}
                                        onChange={(event) => onPropertyChange('manager.stop_condition', event.target.value)}
                                        placeholder='child.outcome == "success"'
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <Label>Manager Actions</Label>
                                    <Input
                                        value={(selectedNode?.data?.['manager.actions'] as string) || ''}
                                        onChange={(event) => onPropertyChange('manager.actions', event.target.value)}
                                        placeholder="observe,steer"
                                    />
                                </div>
                                <div
                                    data-testid="manager-child-linkage"
                                    className="space-y-2 rounded-md border border-border/80 bg-muted/20 px-3 py-2"
                                >
                                    <div>
                                        <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                            Child Pipeline Linkage
                                        </p>
                                        <p className="mt-1 text-[11px] text-muted-foreground">
                                            Manager loops use <code>stack.child_dotfile</code> and <code>stack.child_workdir</code> from graph attributes.
                                        </p>
                                    </div>
                                    <div className="space-y-1 text-[11px] text-foreground">
                                        <p><span className="font-mono">stack.child_dotfile</span>: {graphAttrs['stack.child_dotfile'] || '(unset)'}</p>
                                        <p><span className="font-mono">stack.child_workdir</span>: {graphAttrs['stack.child_workdir'] || '(unset)'}</p>
                                    </div>
                                    <Button
                                        type="button"
                                        data-testid="manager-open-child-settings"
                                        variant="outline"
                                        size="xs"
                                        onClick={onOpenGraphChildSettings}
                                    >
                                        Open Graph Child Settings
                                    </Button>
                                </div>
                            </>
                        ) : null}

                        {visibility.showHumanDefaultChoice ? (
                            <div className="space-y-1.5">
                                <Label>Human Default Choice</Label>
                                <Input
                                    value={(selectedNode?.data?.['human.default_choice'] as string) || ''}
                                    onChange={(event) => onPropertyChange('human.default_choice', event.target.value)}
                                    placeholder="target node id"
                                />
                                <p
                                    data-testid="human-default-choice-timeout-guidance"
                                    className="text-xs text-muted-foreground"
                                >
                                    Used when this gate times out without an explicit answer.
                                </p>
                            </div>
                        ) : null}

                        {visibility.showTypeOverride ? (
                            <div className="space-y-1.5">
                                <Label>Handler Type</Label>
                                <Input
                                    value={(selectedNode?.data?.type as string) || ''}
                                    onChange={(event) => onPropertyChange('type', event.target.value)}
                                    list="node-handler-type-options"
                                    placeholder="optional override"
                                />
                                <datalist id="node-handler-type-options">
                                    <option value="start">start</option>
                                    <option value="exit">exit</option>
                                    <option value="codergen">codergen</option>
                                    <option value="wait.human">wait.human</option>
                                    <option value="conditional">conditional</option>
                                    <option value="parallel">parallel</option>
                                    <option value="parallel.fan_in">parallel.fan_in</option>
                                    <option value="tool">tool</option>
                                    <option value="stack.manager_loop">stack.manager_loop</option>
                                </datalist>
                                {selectedNodeShapeTypeMismatchWarning ? (
                                    <p data-testid="node-shape-type-warning" className="text-xs text-amber-800">
                                        {selectedNodeShapeTypeMismatchWarning}
                                    </p>
                                ) : null}
                                {renderFieldDiagnostics('node', 'type', nodeFieldDiagnostics, 'node-field-diagnostics-type')}
                            </div>
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
                                                <Label>Max Retries</Label>
                                                <Input
                                                    value={(selectedNode?.data?.max_retries as number | string | undefined) ?? ''}
                                                    onChange={(event) => onPropertyChange('max_retries', event.target.value)}
                                                />
                                            </div>
                                            <div className="space-y-1.5">
                                                <Label>Timeout</Label>
                                                <Input
                                                    value={(selectedNode?.data?.timeout as string) || ''}
                                                    onChange={(event) => onPropertyChange('timeout', event.target.value)}
                                                    placeholder="900s"
                                                />
                                            </div>
                                        </div>
                                        <div className="flex items-center gap-2">
                                            <Checkbox
                                                id={`goal-gate-${selectedNodeId}`}
                                                checked={isTrue(selectedNode?.data?.goal_gate)}
                                                onCheckedChange={(checked) => onPropertyChange('goal_gate', checked === true)}
                                            />
                                            <Label htmlFor={`goal-gate-${selectedNodeId}`} className="text-sm font-medium">
                                                Goal Gate
                                            </Label>
                                        </div>
                                        {renderFieldDiagnostics('node', 'goal_gate', nodeFieldDiagnostics, 'node-field-diagnostics-goal_gate')}
                                        <div className="space-y-1.5">
                                            <Label>Retry Target</Label>
                                            <Input
                                                value={(selectedNode?.data?.retry_target as string) || ''}
                                                onChange={(event) => onPropertyChange('retry_target', event.target.value)}
                                            />
                                            {renderFieldDiagnostics('node', 'retry_target', nodeFieldDiagnostics, 'node-field-diagnostics-retry_target')}
                                        </div>
                                        <div className="space-y-1.5">
                                            <Label>Fallback Retry Target</Label>
                                            <Input
                                                value={(selectedNode?.data?.fallback_retry_target as string) || ''}
                                                onChange={(event) => onPropertyChange('fallback_retry_target', event.target.value)}
                                            />
                                            {renderFieldDiagnostics('node', 'fallback_retry_target', nodeFieldDiagnostics, 'node-field-diagnostics-fallback_retry_target')}
                                        </div>
                                        {visibility.showToolCommand ? (
                                            <>
                                                <div className="space-y-1.5">
                                                    <Label>Pre Hook Override</Label>
                                                    <Input
                                                        data-testid="node-attr-input-tool.hooks.pre"
                                                        value={(selectedNode?.data?.['tool.hooks.pre'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.hooks.pre', event.target.value)}
                                                        className="font-mono text-xs"
                                                        placeholder="e.g. ./hooks/pre.sh"
                                                    />
                                                    {selectedNodeToolHookPreWarning ? (
                                                        <p data-testid="node-attr-warning-tool.hooks.pre" className="text-xs text-amber-800">
                                                            {selectedNodeToolHookPreWarning}
                                                        </p>
                                                    ) : null}
                                                </div>
                                                <div className="space-y-1.5">
                                                    <Label>Post Hook Override</Label>
                                                    <Input
                                                        data-testid="node-attr-input-tool.hooks.post"
                                                        value={(selectedNode?.data?.['tool.hooks.post'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.hooks.post', event.target.value)}
                                                        className="font-mono text-xs"
                                                        placeholder="e.g. ./hooks/post.sh"
                                                    />
                                                    {selectedNodeToolHookPostWarning ? (
                                                        <p data-testid="node-attr-warning-tool.hooks.post" className="text-xs text-amber-800">
                                                            {selectedNodeToolHookPostWarning}
                                                        </p>
                                                    ) : null}
                                                </div>
                                                <div className="space-y-1.5">
                                                    <Label>Artifact Paths</Label>
                                                    <Input
                                                        data-testid="node-attr-input-tool.artifacts.paths"
                                                        value={(selectedNode?.data?.['tool.artifacts.paths'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.artifacts.paths', event.target.value)}
                                                        className="font-mono text-xs"
                                                        placeholder="e.g. dist/**,reports/*.json"
                                                    />
                                                </div>
                                                <div className="space-y-1.5">
                                                    <Label>Stdout Artifact</Label>
                                                    <Input
                                                        data-testid="node-attr-input-tool.artifacts.stdout"
                                                        value={(selectedNode?.data?.['tool.artifacts.stdout'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.artifacts.stdout', event.target.value)}
                                                        className="font-mono text-xs"
                                                        placeholder="e.g. stdout.txt"
                                                    />
                                                </div>
                                                <div className="space-y-1.5">
                                                    <Label>Stderr Artifact</Label>
                                                    <Input
                                                        data-testid="node-attr-input-tool.artifacts.stderr"
                                                        value={(selectedNode?.data?.['tool.artifacts.stderr'] as string) || ''}
                                                        onChange={(event) => onPropertyChange('tool.artifacts.stderr', event.target.value)}
                                                        className="font-mono text-xs"
                                                        placeholder="e.g. stderr.txt"
                                                    />
                                                </div>
                                            </>
                                        ) : null}
                                        <div className="grid grid-cols-2 gap-3">
                                            <div className="space-y-1.5">
                                                <Label>Fidelity</Label>
                                                <Input
                                                    value={(selectedNode?.data?.fidelity as string) || ''}
                                                    onChange={(event) => onPropertyChange('fidelity', event.target.value)}
                                                    placeholder="full"
                                                />
                                                {renderFieldDiagnostics('node', 'fidelity', nodeFieldDiagnostics, 'node-field-diagnostics-fidelity')}
                                            </div>
                                            <div className="space-y-1.5">
                                                <Label>Thread ID</Label>
                                                <Input
                                                    value={(selectedNode?.data?.thread_id as string) || ''}
                                                    onChange={(event) => onPropertyChange('thread_id', event.target.value)}
                                                />
                                            </div>
                                        </div>
                                        <div className="space-y-1.5">
                                            <Label>Class</Label>
                                            <Input
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
                                                <Label>LLM Model</Label>
                                                <Input
                                                    value={(selectedNode?.data?.llm_model as string) || ''}
                                                    onChange={(event) => onPropertyChange('llm_model', event.target.value)}
                                                    list="llm-model-options-panel"
                                                />
                                                <datalist id="llm-model-options-panel">
                                                    {getModelSuggestions(selectedProfile || selectedProvider, llmProfiles).map((model) => (
                                                        <option key={model} value={model} />
                                                    ))}
                                                </datalist>
                                            </div>
                                            <div className="space-y-1.5">
                                                <Label>LLM Provider</Label>
                                                <Input
                                                    value={((selectedNode?.data?.llm_profile as string) || (selectedNode?.data?.llm_provider as string)) || ''}
                                                    onChange={(event) => {
                                                        const selection = splitLlmSelection(event.target.value, llmProfiles)
                                                        onPropertyChange('llm_provider', selection.llm_provider)
                                                        onPropertyChange('llm_profile', selection.llm_profile)
                                                    }}
                                                    list="llm-provider-options-panel"
                                                />
                                                <datalist id="llm-provider-options-panel">
                                                    {getLlmSelectionOptions(llmProfiles).map((provider) => (
                                                        <option key={provider} value={provider} />
                                                    ))}
                                                </datalist>
                                            </div>
                                        </div>
                                        <div className="space-y-1.5">
                                            <Label>Reasoning Effort</Label>
                                            <Input
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
                                                id={`node-auto-status-${selectedNodeId}`}
                                                checked={isTrue(selectedNode?.data?.auto_status)}
                                                onCheckedChange={(checked) => onPropertyChange('auto_status', checked === true)}
                                            />
                                            <Label htmlFor={`node-auto-status-${selectedNodeId}`} className="text-sm font-medium">
                                                Auto Status
                                            </Label>
                                        </div>
                                        <div className="flex items-center gap-2">
                                            <Checkbox
                                                id={`node-allow-partial-${selectedNodeId}`}
                                                checked={isTrue(selectedNode?.data?.allow_partial)}
                                                onCheckedChange={(checked) => onPropertyChange('allow_partial', checked === true)}
                                            />
                                            <Label htmlFor={`node-allow-partial-${selectedNodeId}`} className="text-sm font-medium">
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
                                'shape',
                                'prompt',
                                'tool.command',
                                'tool.hooks.pre',
                                'tool.hooks.post',
                                'tool.artifacts.paths',
                                'tool.artifacts.stdout',
                                'tool.artifacts.stderr',
                                'join_policy',
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
