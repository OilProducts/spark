import type { ReactNode } from 'react'

import type { ExtensionAttrEntry } from '@/lib/extensionAttrs'
import type { LaunchInputDefinition } from '@/lib/flowContracts'
import { GRAPH_FIDELITY_OPTIONS } from '@/lib/graphAttrValidation'
import { getLlmSelectionOptions, getModelSuggestions, splitLlmSelection, type LlmProfileMetadata } from '@/lib/llmSuggestions'
import type { FlowDefinitionMetadata, FlowMetadataErrors, UiDefaults } from '@/store'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
    Field,
    FieldDescription,
    FieldError,
    FieldLabel,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import { AdvancedKeyValueEditor } from '../AdvancedKeyValueEditor'
import { LaunchInputsEditor } from '../LaunchInputsEditor'
import type { FlowExecutionLockResponse, FlowLaunchPolicy } from '../../services/graphLaunchPolicy'

export const FLOW_METADATA_HELP: Record<string, string> = {
    title: 'Human-friendly flow title stored in the FlowDefinition.',
    description: 'Short flow description stored in the FlowDefinition.',
    inputs: 'Structured launch-time context fields Spark should collect before starting a run.',
    result_node: 'Optional node whose response becomes the run result.',
    result_summary_enabled: 'When true, summarize the selected run result before display.',
    result_summary_prompt: 'Optional prompt used when result summarization is enabled.',
    goal: 'Primary stated goal for the flow. Handlers can read it as shared run context.',
    max_retries: 'Used only when a node omits retry.max_retries. Node retry settings take precedence.',
    fidelity: 'Default runtime fidelity when node runtime settings do not set it explicitly.',
}

export const CORE_FLOW_METADATA_KEYS = new Set<string>([
    'schema_version',
    'id',
    'title',
    'description',
    'inputs',
    'result_node',
    'result_summary_enabled',
    'result_summary_prompt',
    'goal',
    'max_retries',
    'fidelity',
    'llm_model',
    'llm_provider',
    'llm_profile',
    'reasoning_effort',
])

export const FLOW_LAUNCH_POLICY_LABELS: Record<FlowLaunchPolicy, string> = {
    agent_requestable: 'Agent Requestable',
    trigger_only: 'Trigger Only',
    disabled: 'Disabled',
}

function GraphSettingsSectionIntro({
    title,
    description,
    action,
}: {
    title: string
    description?: string | null
    action?: ReactNode
}) {
    return (
        <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 space-y-1">
                <h3 className="text-sm font-semibold text-foreground">{title}</h3>
                {description ? (
                    <p className="text-xs leading-5 text-muted-foreground">{description}</p>
                ) : null}
            </div>
            {action ? <div className="shrink-0">{action}</div> : null}
        </div>
    )
}

function GraphSettingsField({
    label,
    htmlFor,
    helper,
    error,
    className,
    children,
}: {
    label: string
    htmlFor?: string
    helper?: string | null
    error?: string | null
    className?: string
    children: ReactNode
}) {
    return (
        <Field className={className}>
            <FieldLabel htmlFor={htmlFor}>{label}</FieldLabel>
            {children}
            {helper ? <FieldDescription className="text-[11px]">{helper}</FieldDescription> : null}
            {error ? <FieldError className="text-[11px]">{error}</FieldError> : null}
        </Field>
    )
}

const GRAPH_SETTINGS_NOTICE_TONE_CLASS_NAME: Record<
    'neutral' | 'warning' | 'error' | 'success',
    string
> = {
    neutral: 'border-border/70 bg-muted/20 text-muted-foreground',
    warning: 'border-amber-500/40 bg-amber-500/10 text-amber-800',
    error: 'border-destructive/40 bg-destructive/10 text-destructive',
    success: 'border-emerald-500/40 bg-emerald-500/10 text-emerald-800',
}

function GraphSettingsNotice({
    tone = 'neutral',
    className,
    children,
    ...props
}: React.ComponentProps<typeof Alert> & {
    tone?: 'neutral' | 'warning' | 'error' | 'success'
}) {
    return (
        <Alert
            className={cn('px-3 py-2', GRAPH_SETTINGS_NOTICE_TONE_CLASS_NAME[tone], className)}
            {...props}
        >
            <AlertDescription className="text-inherit">{children}</AlertDescription>
        </Alert>
    )
}

interface GraphRunConfigurationSectionProps {
    model: string
    workingDir: string
    setModel: (value: string) => void
    setWorkingDir: (value: string) => void
}

export function GraphRunConfigurationSection({
    model,
    workingDir,
    setModel,
    setWorkingDir,
}: GraphRunConfigurationSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Run Configuration"
                description="Editor-scoped runtime defaults used while authoring and previewing this flow."
            />
            <div className="space-y-3">
                <GraphSettingsField label="Model" htmlFor="graph-run-model">
                    <Input
                        id="graph-run-model"
                        value={model}
                        onChange={(event) => setModel(event.target.value)}
                        className="h-8 text-xs"
                        placeholder="codex default"
                    />
                </GraphSettingsField>
                <GraphSettingsField label="Working Directory" htmlFor="graph-run-working-directory">
                    <Input
                        id="graph-run-working-directory"
                        value={workingDir}
                        onChange={(event) => setWorkingDir(event.target.value)}
                        className="h-8 font-mono text-xs"
                        placeholder="./test-app"
                    />
                </GraphSettingsField>
            </div>
        </section>
    )
}

interface GraphMetadataSectionProps {
    flowMetadata: FlowDefinitionMetadata
    updateFlowMetadata: (key: keyof FlowDefinitionMetadata, value: string) => void
}

export function GraphMetadataSection({
    flowMetadata,
    updateFlowMetadata,
}: GraphMetadataSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Flow Metadata"
                description="Human-facing title and description stored in the FlowDefinition."
            />
            <div className="space-y-3">
                <GraphSettingsField
                    label="Title"
                    htmlFor="graph-attr-spark-title"
                    helper={FLOW_METADATA_HELP.title}
                >
                    <Input
                        id="graph-attr-spark-title"
                        value={flowMetadata.title || ''}
                        onChange={(event) => updateFlowMetadata('title', event.target.value)}
                        className="h-8 text-xs"
                        placeholder="Implement From Plan File"
                    />
                </GraphSettingsField>
                <GraphSettingsField
                    label="Description"
                    htmlFor="graph-attr-spark-description"
                    helper={FLOW_METADATA_HELP.description}
                >
                    <Textarea
                        id="graph-attr-spark-description"
                        value={flowMetadata.description || ''}
                        onChange={(event) => updateFlowMetadata('description', event.target.value)}
                        rows={3}
                        className="min-h-20 px-2 py-1 text-xs"
                        placeholder="Snapshot a plan file, implement it, and iterate until complete."
                    />
                </GraphSettingsField>
            </div>
        </section>
    )
}

interface GraphLaunchInputsSectionProps {
    launchInputDrafts: LaunchInputDefinition[]
    launchInputDraftError: string | null
    onLaunchInputDefinitionsChange: (entries: LaunchInputDefinition[]) => void
}

export function GraphLaunchInputsSection({
    launchInputDrafts,
    launchInputDraftError,
    onLaunchInputDefinitionsChange,
}: GraphLaunchInputsSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Launch Inputs"
                description="Define the structured fields Spark should collect before a run starts."
            />
            <LaunchInputsEditor
                entries={launchInputDrafts}
                error={launchInputDraftError}
                onChange={onLaunchInputDefinitionsChange}
            />
        </section>
    )
}

interface GraphResultSectionProps {
    flowMetadata: FlowDefinitionMetadata
    nodes: Array<{ id: string; data?: { label?: unknown; kind?: unknown } }>
    updateFlowMetadata: (key: keyof FlowDefinitionMetadata, value: string) => void
}

export function GraphResultSection({
    flowMetadata,
    nodes,
    updateFlowMetadata,
}: GraphResultSectionProps) {
    const summaryEnabled = String(flowMetadata.result_summary_enabled || '').toLowerCase() === 'true'
    const selectableNodes = nodes.filter((node) => {
        const kind = typeof node.data?.kind === 'string' ? node.data.kind : ''
        return kind !== 'start' && kind !== 'exit'
    })
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Run Result"
                description="Choose the node response Spark should surface after a run completes."
            />
            <div className="space-y-3">
                <GraphSettingsField
                    label="Result Node"
                    htmlFor="graph-attr-spark-result-node"
                    helper={FLOW_METADATA_HELP.result_node}
                >
                    <NativeSelect
                        id="graph-attr-spark-result-node"
                        value={flowMetadata.result_node || ''}
                        onChange={(event) => updateFlowMetadata('result_node', event.target.value)}
                        className="h-8 text-xs"
                    >
                        <option value="">Infer from final node</option>
                        {selectableNodes.map((node) => {
                            const label = typeof node.data?.label === 'string' && node.data.label.trim()
                                ? node.data.label.trim()
                                : node.id
                            return (
                                <option key={node.id} value={node.id}>
                                    {label}
                                </option>
                            )
                        })}
                    </NativeSelect>
                </GraphSettingsField>
                <div className="flex items-center gap-2">
                    <Checkbox
                        id="graph-attr-spark-result-summary-enabled"
                        checked={summaryEnabled}
                        onCheckedChange={(checked) => {
                            updateFlowMetadata('result_summary_enabled', checked ? 'true' : '')
                        }}
                    />
                    <Label htmlFor="graph-attr-spark-result-summary-enabled" className="text-xs">
                        Summarize result
                    </Label>
                </div>
                {summaryEnabled ? (
                    <GraphSettingsField
                        label="Summary Prompt"
                        htmlFor="graph-attr-spark-result-summary-prompt"
                        helper={FLOW_METADATA_HELP.result_summary_prompt}
                    >
                        <Textarea
                            id="graph-attr-spark-result-summary-prompt"
                            value={flowMetadata.result_summary_prompt || ''}
                            onChange={(event) => updateFlowMetadata('result_summary_prompt', event.target.value)}
                            rows={4}
                            className="min-h-24 px-2 py-1 text-xs"
                            placeholder="Use Spark's default prompt"
                        />
                    </GraphSettingsField>
                ) : null}
            </div>
        </section>
    )
}

interface GraphExecutionDefaultsSectionProps {
    flowMetadata: FlowDefinitionMetadata
    flowMetadataErrors: FlowMetadataErrors
    renderFieldDiagnostics: (field: string, testId: string) => ReactNode
    updateFlowMetadata: (key: keyof FlowDefinitionMetadata, value: string) => void
}

export function GraphExecutionDefaultsSection({
    flowMetadata,
    flowMetadataErrors,
    renderFieldDiagnostics,
    updateFlowMetadata,
}: GraphExecutionDefaultsSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Execution Defaults"
                description="FlowDefinition defaults that shape retry behavior and baseline run context."
            />
            <GraphSettingsNotice
                data-testid="flow-metadata-help"
                className="text-[11px]"
            >
                <p>FlowDefinition defaults are used when node runtime settings omit a value.</p>
                <p>Leave blank to omit the field from YAML output.</p>
            </GraphSettingsNotice>
            <div className="space-y-3">
                <div className="space-y-1">
                    <GraphSettingsField
                        label="Goal"
                        htmlFor="graph-attr-goal"
                        helper={FLOW_METADATA_HELP.goal}
                    >
                        <Input
                            id="graph-attr-goal"
                            value={flowMetadata.goal || ''}
                            onChange={(event) => updateFlowMetadata('goal', event.target.value)}
                            className="h-8 text-xs"
                        />
                    </GraphSettingsField>
                </div>
                <div className="grid grid-cols-2 gap-3">
                    <div className="space-y-1">
                        <GraphSettingsField
                            label="Default Max Retries"
                            htmlFor="graph-attr-default-max-retries"
                            helper={FLOW_METADATA_HELP.max_retries}
                            error={flowMetadataErrors.max_retries}
                        >
                            <Input
                                id="graph-attr-default-max-retries"
                                type="number"
                                min={0}
                                step={1}
                                inputMode="numeric"
                                value={flowMetadata.max_retries ?? ''}
                                onChange={(event) => updateFlowMetadata('max_retries', event.target.value)}
                                className="h-8 text-xs"
                            />
                        </GraphSettingsField>
                        {renderFieldDiagnostics('max_retries', 'graph-field-diagnostics-max_retries')}
                    </div>
                    <div className="space-y-1">
                        <GraphSettingsField
                            label="Default Fidelity"
                            htmlFor="graph-attr-default-fidelity"
                            helper={FLOW_METADATA_HELP.fidelity}
                            error={flowMetadataErrors.fidelity}
                        >
                            <Input
                                id="graph-attr-default-fidelity"
                                value={flowMetadata.fidelity || ''}
                                onChange={(event) => updateFlowMetadata('fidelity', event.target.value)}
                                list="graph-fidelity-options"
                                className="h-8 text-xs"
                                placeholder="full"
                            />
                            <datalist id="graph-fidelity-options">
                                {GRAPH_FIDELITY_OPTIONS.map((option) => (
                                    <option key={option} value={option} />
                                ))}
                            </datalist>
                        </GraphSettingsField>
                        {renderFieldDiagnostics('fidelity', 'graph-field-diagnostics-fidelity')}
                    </div>
                </div>
            </div>
        </section>
    )
}

interface GraphLaunchPolicySectionProps {
    activeFlow: string | null
    launchPolicy: FlowLaunchPolicy
    executionLock: FlowExecutionLockResponse | null
    executionLockEnabled: boolean
    launchPolicyLoadState: 'idle' | 'loading' | 'ready' | 'error'
    launchPolicySaveState: 'idle' | 'saving' | 'saved' | 'error'
    launchPolicyStatusMessage: string
    onLaunchPolicyChange: (policy: FlowLaunchPolicy) => void | Promise<void>
    onExecutionLockEnabledChange: (enabled: boolean) => void
    onExecutionLockKeyChange: (value: string) => void
    onExecutionLockKeyCommit: () => void | Promise<void>
}

export function GraphLaunchPolicySection({
    activeFlow,
    launchPolicy,
    executionLock,
    executionLockEnabled,
    launchPolicyLoadState,
    launchPolicySaveState,
    launchPolicyStatusMessage,
    onLaunchPolicyChange,
    onExecutionLockEnabledChange,
    onExecutionLockKeyChange,
    onExecutionLockKeyCommit,
}: GraphLaunchPolicySectionProps) {
    const controlsDisabled = !activeFlow || launchPolicyLoadState !== 'ready' || launchPolicySaveState === 'saving'
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Launch Policy"
                description="Workspace-level launch behavior for this flow catalog entry."
            />
            <GraphSettingsField label="Launch Policy" htmlFor="graph-launch-policy">
                <NativeSelect
                    id="graph-launch-policy"
                    value={launchPolicy}
                    onChange={(event) => void onLaunchPolicyChange(event.target.value as FlowLaunchPolicy)}
                    disabled={controlsDisabled}
                    className="h-8 text-xs"
                >
                    {Object.entries(FLOW_LAUNCH_POLICY_LABELS).map(([value, label]) => (
                        <option key={value} value={value}>
                            {label}
                        </option>
                    ))}
                </NativeSelect>
            </GraphSettingsField>
            <div className="space-y-3 rounded-md border border-border/70 bg-muted/10 p-3">
                <Label htmlFor="graph-execution-lock-enabled" className="flex items-end gap-2 text-sm">
                    <Checkbox
                        id="graph-execution-lock-enabled"
                        checked={executionLockEnabled}
                        onCheckedChange={(checked) => onExecutionLockEnabledChange(checked === true)}
                        disabled={controlsDisabled}
                    />
                    <span className="text-xs font-medium text-foreground">Enable execution lock</span>
                </Label>
                <GraphSettingsField
                    label="Lock Scope"
                    htmlFor="graph-execution-lock-scope"
                    helper="Workspace launch admission policy, not YAML flow semantics."
                >
                    <NativeSelect
                        id="graph-execution-lock-scope"
                        value={executionLock?.scope ?? 'project'}
                        disabled
                        className="h-8 text-xs"
                    >
                        <option value="project">Project</option>
                    </NativeSelect>
                </GraphSettingsField>
                <GraphSettingsField label="Lock Key" htmlFor="graph-execution-lock-key">
                    <Input
                        id="graph-execution-lock-key"
                        value={executionLock?.key ?? ''}
                        onChange={(event) => onExecutionLockKeyChange(event.target.value)}
                        onBlur={() => void onExecutionLockKeyCommit()}
                        disabled={controlsDisabled || !executionLockEnabled}
                        className="h-8 text-xs"
                    />
                </GraphSettingsField>
                <GraphSettingsField label="Conflict Policy" htmlFor="graph-execution-lock-conflict-policy">
                    <NativeSelect
                        id="graph-execution-lock-conflict-policy"
                        value={executionLock?.conflict_policy ?? 'queue'}
                        disabled
                        className="h-8 text-xs"
                    >
                        <option value="queue">Queue</option>
                    </NativeSelect>
                </GraphSettingsField>
            </div>
            <GraphSettingsNotice
                data-testid="graph-launch-policy-status"
                className="text-[11px]"
            >
                {launchPolicyStatusMessage}
            </GraphSettingsNotice>
        </section>
    )
}

interface GraphAdvancedAttrsSectionProps {
    showAdvancedFlowMetadata: boolean
    flowMetadataExtensionEntries: ExtensionAttrEntry[]
    setShowAdvancedFlowMetadata: (value: boolean | ((current: boolean) => boolean)) => void
    onFlowMetadataExtensionValueChange: (key: string, value: string) => void
    onFlowMetadataExtensionRemove: (key: string) => void
    onFlowMetadataExtensionAdd: (key: string, value: string) => void
}

export function GraphAdvancedAttrsSection({
    showAdvancedFlowMetadata,
    flowMetadataExtensionEntries,
    setShowAdvancedFlowMetadata,
    onFlowMetadataExtensionValueChange,
    onFlowMetadataExtensionRemove,
    onFlowMetadataExtensionAdd,
}: GraphAdvancedAttrsSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Extension Metadata"
                description="Non-core FlowDefinition metadata stored under the flow metadata map."
                action={(
                    <Button
                        type="button"
                        data-testid="graph-advanced-toggle"
                        variant="outline"
                        size="sm"
                        className="h-8 px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground"
                        onClick={() => setShowAdvancedFlowMetadata((current) => !current)}
                    >
                        {showAdvancedFlowMetadata ? 'Hide Advanced Fields' : 'Show Advanced Fields'}
                    </Button>
                )}
            />
            {showAdvancedFlowMetadata ? (
                <div className="space-y-3 rounded-md border border-border/80 bg-background/40 p-3">
                    <AdvancedKeyValueEditor
                        testIdPrefix="graph"
                        entries={flowMetadataExtensionEntries}
                        onValueChange={onFlowMetadataExtensionValueChange}
                        onRemove={onFlowMetadataExtensionRemove}
                        onAdd={onFlowMetadataExtensionAdd}
                        reservedKeys={CORE_FLOW_METADATA_KEYS}
                    />
                </div>
            ) : (
                <GraphSettingsNotice className="text-[11px]">
                    Extension metadata stays available for non-core FlowDefinition annotations.
                </GraphSettingsNotice>
            )}
        </section>
    )
}

interface GraphLlmDefaultsSectionProps {
    canApplyDefaults: boolean
    flowProviderFallback: string
    flowMetadata: FlowDefinitionMetadata
    uiDefaults: UiDefaults
    llmProfiles: LlmProfileMetadata[]
    applyDefaultsToNodes: () => void
    updateFlowMetadata: (key: keyof FlowDefinitionMetadata, value: string) => void
}

export function GraphLlmDefaultsSection({
    canApplyDefaults,
    flowProviderFallback,
    flowMetadata,
    uiDefaults,
    llmProfiles,
    applyDefaultsToNodes,
    updateFlowMetadata,
}: GraphLlmDefaultsSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Model Defaults"
                description="Flow-local LLM defaults layered on top of the current global snapshot."
            />
            <div className="space-y-3">
                <GraphSettingsField label="Default LLM Provider" htmlFor="graph-default-llm-provider">
                    <Input
                        id="graph-default-llm-provider"
                        value={flowMetadata.llm_profile || flowMetadata.llm_provider || ''}
                        onChange={(event) => {
                            const selection = splitLlmSelection(event.target.value, llmProfiles)
                            updateFlowMetadata('llm_provider', selection.llm_provider)
                            updateFlowMetadata('llm_profile', selection.llm_profile)
                        }}
                        list="flow-llm-provider-options"
                        className="h-8 text-xs"
                        placeholder={uiDefaults.llm_provider ? `Snapshot: ${uiDefaults.llm_provider}` : 'Snapshot of global default'}
                    />
                    <datalist id="flow-llm-provider-options">
                        {getLlmSelectionOptions(llmProfiles).map((provider) => (
                            <option key={provider} value={provider} />
                        ))}
                    </datalist>
                </GraphSettingsField>
                <GraphSettingsField label="Default LLM Model" htmlFor="graph-default-llm-model">
                    <Input
                        id="graph-default-llm-model"
                        value={flowMetadata.llm_model || ''}
                        onChange={(event) => updateFlowMetadata('llm_model', event.target.value)}
                        list="flow-llm-model-options"
                        className="h-8 text-xs"
                        placeholder={uiDefaults.llm_model ? `Snapshot: ${uiDefaults.llm_model}` : 'Snapshot of global default'}
                    />
                    <datalist id="flow-llm-model-options">
                        {getModelSuggestions(flowMetadata.llm_profile || flowProviderFallback, llmProfiles).map((modelOption) => (
                            <option key={modelOption} value={modelOption} />
                        ))}
                    </datalist>
                </GraphSettingsField>
                <GraphSettingsField label="Default Reasoning Effort" htmlFor="graph-default-reasoning-effort">
                    <NativeSelect
                        id="graph-default-reasoning-effort"
                        value={flowMetadata.reasoning_effort || ''}
                        onChange={(event) => updateFlowMetadata('reasoning_effort', event.target.value)}
                        className="h-8 text-xs"
                    >
                        <option value="">Use global default</option>
                        <option value="low">Low</option>
                        <option value="medium">Medium</option>
                        <option value="high">High</option>
                        <option value="xhigh">XHigh</option>
                    </NativeSelect>
                </GraphSettingsField>
                <div className="flex items-center justify-between gap-2">
                    <Button
                        type="button"
                        onClick={applyDefaultsToNodes}
                        disabled={!canApplyDefaults}
                        variant="outline"
                        size="sm"
                        className="h-8 px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground"
                        title={canApplyDefaults ? 'Apply current flow defaults to every node.' : 'Switch to the editor to apply defaults.'}
                    >
                        Apply To Nodes
                    </Button>
                    <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="h-8 px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground"
                        onClick={() => {
                            updateFlowMetadata('llm_provider', uiDefaults.llm_provider)
                            updateFlowMetadata('llm_profile', uiDefaults.llm_profile)
                            updateFlowMetadata('llm_model', uiDefaults.llm_model)
                            updateFlowMetadata('reasoning_effort', uiDefaults.reasoning_effort)
                        }}
                    >
                        Reset From Global
                    </Button>
                </div>
            </div>
        </section>
    )
}
