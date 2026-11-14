import type { ReactNode } from 'react'

import type { ExtensionAttrEntry } from '@/lib/extensionAttrs'
import type { LaunchInputDefinition } from '@/lib/flowContracts'
import { ModelChooser } from '@/components/model-chooser/ModelChooser'
import type { FlowDefinitionMetadata, FlowMetadataErrors, UiDefaults } from '@/store'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
    Field,
    FieldDescription,
    FieldError,
    FieldLabel,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import { AdvancedKeyValueEditor } from '../AdvancedKeyValueEditor'
import { LaunchInputsEditor } from '../LaunchInputsEditor'
import { openTrigger } from '@/app/useAttention'
import type { TriggerResponse } from '@/lib/api/triggersApi'

export const FLOW_METADATA_HELP: Record<string, string> = {
    inputs: 'Structured launch-time context fields Spark should collect before starting a run.',
    result_node: 'The step whose response Spark shows as the run result.',
    goal: 'Primary stated goal for the flow. Handlers can read it as shared run context.',
    max_retries: 'Used only when a step omits retry.max_retries. Step retry settings take precedence.',
}

// The flow metadata key the runtime reads to pick the run result's source step.
export const RESULT_NODE_KEY = 'spark.result_node'

export const CORE_FLOW_METADATA_KEYS = new Set<string>([
    'schema_version',
    'id',
    'title',
    'description',
    'execution_lock',
    'inputs',
    RESULT_NODE_KEY,
    'goal',
    'max_retries',
    'llm_model',
    'llm_provider',
    'llm_profile',
    'reasoning_effort',
    'thinking',
    'thinking_budget_tokens',
    'reasoning_mode',
    'reasoning_summary',

])

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
            {helper ? <FieldDescription className="text-xs">{helper}</FieldDescription> : null}
            {error ? <FieldError className="text-xs">{error}</FieldError> : null}
        </Field>
    )
}

function GraphSettingsNotice({
    className,
    children,
    ...props
}: React.ComponentProps<typeof Alert>) {
    return (
        <Alert
            className={cn('border-border/70 px-3 py-2 text-muted-foreground', className)}
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
                title="Running from the editor"
                description="What the Run… button starts with. These aren't saved in the flow."
            />
            <div className="space-y-3">
                <GraphSettingsField label="Model" htmlFor="graph-run-model">
                    <Input
                        id="graph-run-model"
                        value={model}
                        onChange={(event) => setModel(event.target.value)}
                        className="h-8 text-sm"
                        placeholder="The flow's own choice"
                    />
                </GraphSettingsField>
                <GraphSettingsField label="Working directory" htmlFor="graph-run-working-directory">
                    <Input
                        id="graph-run-working-directory"
                        value={workingDir}
                        onChange={(event) => setWorkingDir(event.target.value)}
                        className="h-8 font-mono text-sm"
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
                title="About this flow"
                description="How the flow is named and described wherever people pick it."
            />
            <div className="space-y-3">
                <GraphSettingsField
                    label="Title"
                    htmlFor="graph-attr-spark-title"
                >
                    <Input
                        id="graph-attr-spark-title"
                        value={flowMetadata.title || ''}
                        onChange={(event) => updateFlowMetadata('title', event.target.value)}
                        className="h-8 text-sm"
                        placeholder="The name shown in the flow list"
                    />
                </GraphSettingsField>
                <GraphSettingsField
                    label="Description"
                    htmlFor="graph-attr-spark-description"
                >
                    <Textarea
                        id="graph-attr-spark-description"
                        value={flowMetadata.description || ''}
                        onChange={(event) => updateFlowMetadata('description', event.target.value)}
                        rows={3}
                        className="min-h-20 px-2 py-1 text-sm"
                        placeholder="What the flow does, shown when choosing one to run"
                    />
                </GraphSettingsField>
                <GraphSettingsField
                    label="One at a time (lock name)"
                    htmlFor="graph-attr-execution-lock-key"
                    helper="Flows that share this name never run at the same time in a project; later runs wait their turn. Leave empty to allow overlapping runs."
                >
                    <Input
                        id="graph-attr-execution-lock-key"
                        value={typeof flowMetadata.execution_lock === 'string' ? flowMetadata.execution_lock : ''}
                        onChange={(event) => updateFlowMetadata('execution_lock', event.target.value)}
                        className="h-8 text-sm"
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
    /** Triggers that start this flow without a required input, which each of their firings then fails on. */
    triggersMissingInputs?: { trigger: TriggerResponse; missing: string[] }[]
}

export function GraphLaunchInputsSection({
    launchInputDrafts,
    launchInputDraftError,
    onLaunchInputDefinitionsChange,
    triggersMissingInputs = [],
}: GraphLaunchInputsSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Launch inputs"
                description="Fields Spark collects as context values before a run starts."
            />
            {triggersMissingInputs.length > 0 ? (
                <Alert data-testid="graph-launch-inputs-trigger-gaps" className="border-warning/40 py-2">
                    <AlertDescription className="text-xs text-foreground">
                        {/* Said of the runs a trigger would start, so it holds for one that is turned off too. */}
                        <p>{triggersMissingInputs.length === 1 ? 'A trigger doesn’t set a required input, so a run it starts fails:' : 'Triggers don’t set a required input, so a run they start fails:'}</p>
                        <ul className="mt-1 space-y-0.5">
                            {triggersMissingInputs.map(({ trigger, missing }) => (
                                <li key={`${trigger.project_path}\u0000${trigger.id}`}>
                                    <Button type="button" variant="link" className="h-auto p-0 text-xs" onClick={() => openTrigger(trigger.project_path, trigger.id)}>{trigger.name}</Button>
                                    <span className="text-muted-foreground"> needs {missing.join(', ')}{trigger.enabled ? null : ' · turned off'}</span>
                                </li>
                            ))}
                        </ul>
                    </AlertDescription>
                </Alert>
            ) : null}
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
    nodes: Array<{ id: string; data?: { label?: unknown; kind?: unknown; config?: unknown } }>
    updateFlowMetadata: (key: keyof FlowDefinitionMetadata, value: string) => void
}

const nodeDisplayLabel = (node: GraphResultSectionProps['nodes'][number]) => (
    typeof node.data?.label === 'string' && node.data.label.trim() ? node.data.label.trim() : node.id
)

export function GraphResultSection({
    flowMetadata,
    nodes,
    updateFlowMetadata,
}: GraphResultSectionProps) {
    const resultNode = typeof flowMetadata[RESULT_NODE_KEY] === 'string' ? flowMetadata[RESULT_NODE_KEY] : ''
    const selectableNodes = nodes.filter((node) => {
        const kind = typeof node.data?.kind === 'string' ? node.data.kind : ''
        return kind !== 'start' && kind !== 'exit'
    })
    const summarizingExits = nodes.filter((node) => (
        node.data?.kind === 'exit'
        && (node.data.config as { result_summary?: unknown } | undefined)?.result_summary === true
    ))
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Run result"
                description="What Spark shows as a run's result when it ends."
            />
            <div className="space-y-3">
                <GraphSettingsField
                    label="Result step"
                    htmlFor="graph-attr-spark-result-node"
                    helper={FLOW_METADATA_HELP.result_node}
                >
                    <NativeSelect
                        id="graph-attr-spark-result-node"
                        value={resultNode}
                        onChange={(event) => updateFlowMetadata(RESULT_NODE_KEY, event.target.value)}
                        className="h-8 text-sm"
                    >
                        <option value="">The last step before the exit</option>
                        {selectableNodes.map((node) => (
                            <option key={node.id} value={node.id}>
                                {nodeDisplayLabel(node)}
                            </option>
                        ))}
                    </NativeSelect>
                </GraphSettingsField>
                <p data-testid="graph-result-summary-exits" className="text-xs leading-5 text-muted-foreground">
                    {summarizingExits.length > 0
                        ? `Runs that end at ${summarizingExits.map(nodeDisplayLabel).join(', ')} get an agent-written summary instead. Turn this on or off on each exit step.`
                        : 'To have an agent summarize the run instead, select an exit step and turn on Summarize the run here.'}
                </p>
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
                title="Execution defaults"
            />
            <GraphSettingsNotice
                data-testid="flow-metadata-help"
                className="text-xs"
            >
                <p>Leave blank to omit the field from the flow.</p>
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
                            className="h-8 text-sm"
                        />
                    </GraphSettingsField>
                </div>
                <div className="grid grid-cols-2 gap-3">
                    <div className="space-y-1">
                        <GraphSettingsField
                            label="Default max retries"
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
                                className="h-8 text-sm"
                            />
                        </GraphSettingsField>
                        {renderFieldDiagnostics('max_retries', 'graph-field-diagnostics-max_retries')}
                    </div>
                </div>
            </div>
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
                title="Extension metadata"
                action={(
                    <Button
                        type="button"
                        data-testid="graph-advanced-toggle"
                        variant="outline"
                        size="sm"
                        className="h-8 px-2 text-xs"
                        onClick={() => setShowAdvancedFlowMetadata((current) => !current)}
                    >
                        {showAdvancedFlowMetadata ? 'Hide advanced fields' : 'Show advanced fields'}
                    </Button>
                )}
            />
            {showAdvancedFlowMetadata ? (
                <AdvancedKeyValueEditor
                    testIdPrefix="graph"
                    entries={flowMetadataExtensionEntries}
                    onValueChange={onFlowMetadataExtensionValueChange}
                    onRemove={onFlowMetadataExtensionRemove}
                    onAdd={onFlowMetadataExtensionAdd}
                    reservedKeys={CORE_FLOW_METADATA_KEYS}
                />
            ) : null}
        </section>
    )
}

const MODEL_DEFAULT_KEYS = ['llm_provider', 'llm_profile', 'llm_model', 'reasoning_effort', 'thinking', 'thinking_budget_tokens', 'reasoning_mode', 'reasoning_summary'] as const satisfies readonly (keyof FlowDefinitionMetadata)[]

interface GraphLlmDefaultsSectionProps {
    canApplyDefaults: boolean
    flowMetadata: FlowDefinitionMetadata
    uiDefaults: UiDefaults
    projectPath: string | null
    applyDefaultsToNodes: () => void
    updateFlowMetadata: (key: keyof FlowDefinitionMetadata, value: string) => void
}

export function GraphLlmDefaultsSection({
    canApplyDefaults,
    flowMetadata,
    uiDefaults,
    projectPath,
    applyDefaultsToNodes,
    updateFlowMetadata,
}: GraphLlmDefaultsSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Model defaults"
            />
            <div className="space-y-3">
                <ModelChooser projectPath={projectPath} inheritLabel="Workspace default"
                    inherited={{ provider: uiDefaults.llm_provider || null, llm_profile: uiDefaults.llm_profile || null, model: uiDefaults.llm_model || null, reasoning_effort: uiDefaults.reasoning_effort || null, thinking: uiDefaults.thinking || null, thinking_budget_tokens: Number(uiDefaults.thinking_budget_tokens) || null, reasoning_mode: uiDefaults.reasoning_mode || null, reasoning_summary: uiDefaults.reasoning_summary || null }}
                    value={{ provider: flowMetadata.llm_provider || null, llm_profile: flowMetadata.llm_profile || null,
                        model: flowMetadata.llm_model || null, reasoning_effort: flowMetadata.reasoning_effort || null, thinking: flowMetadata.thinking || null, thinking_budget_tokens: Number(flowMetadata.thinking_budget_tokens) || null, reasoning_mode: flowMetadata.reasoning_mode || null, reasoning_summary: flowMetadata.reasoning_summary || null }}
                    onChange={(value) => {
                        updateFlowMetadata('llm_provider', value.provider || '')
                        updateFlowMetadata('llm_profile', value.llm_profile || '')
                        updateFlowMetadata('llm_model', value.model || '')
                        updateFlowMetadata('reasoning_effort', value.reasoning_effort || '')
                        updateFlowMetadata('thinking', value.thinking == null ? '' : String(value.thinking))
                        updateFlowMetadata('thinking_budget_tokens', value.thinking_budget_tokens == null ? '' : String(value.thinking_budget_tokens))
                        updateFlowMetadata('reasoning_mode', value.reasoning_mode == null ? '' : String(value.reasoning_mode))
                        updateFlowMetadata('reasoning_summary', value.reasoning_summary == null ? '' : String(value.reasoning_summary))

                    }} />
                <div className="flex flex-wrap items-center justify-between gap-2">
                    <Button
                        type="button"
                        onClick={applyDefaultsToNodes}
                        disabled={!canApplyDefaults}
                        variant="outline"
                        size="sm"
                        className="h-8 px-2 text-xs"
                        title={canApplyDefaults ? 'Apply current flow defaults to every step.' : 'Switch to the editor to apply defaults.'}
                    >
                        Apply to steps
                    </Button>
                    {/* Clears the flow's own choice, so it follows the workspace default as that changes. */}
                    <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="h-8 px-2 text-xs"
                        disabled={MODEL_DEFAULT_KEYS.every((key) => !flowMetadata[key])}
                        title="Stop setting a model for this flow and follow the workspace default."
                        onClick={() => MODEL_DEFAULT_KEYS.forEach((key) => updateFlowMetadata(key, ''))}
                    >
                        Use workspace default
                    </Button>
                </div>
            </div>
        </section>
    )
}
