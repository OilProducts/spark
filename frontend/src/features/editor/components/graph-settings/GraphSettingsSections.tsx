import type { ReactNode } from 'react'

import type { ExtensionAttrEntry } from '@/lib/extensionAttrs'
import type { LaunchInputDefinition } from '@/lib/flowContracts'
import { GRAPH_FIDELITY_OPTIONS } from '@/lib/graphAttrValidation'
import { getLlmSelectionOptions, getModelSuggestions, splitLlmSelection, type LlmProfileMetadata } from '@/lib/llmSuggestions'
import type { ModelStylesheetPreview, ModelValueSource } from '@/lib/modelStylesheetPreview'
import type { DiagnosticEntry, GraphAttrErrors, GraphAttrs, UiDefaults } from '@/store'
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
import { StylesheetEditor } from '../StylesheetEditor'
import type { FlowLaunchPolicy } from '../../services/graphLaunchPolicy'

export const GRAPH_ATTR_HELP: Record<string, string> = {
    'spark.title': 'Human-friendly flow title stored in the DOT metadata.',
    'spark.description': 'Short flow description stored in the DOT metadata.',
    'spark.launch_inputs': 'Structured launch-time context fields Spark should collect before starting a run.',
    goal: 'Primary stated goal for the flow. Handlers can read it as shared run context.',
    label: 'Display label for graph metadata; does not override node labels.',
    default_max_retries: 'Used only when a node omits max_retries. Node max_retries takes precedence.',
    default_fidelity: 'Default fidelity when node/edge fidelity is not set explicitly.',
    model_stylesheet: 'Selector-based model defaults. Explicit node attrs override stylesheet matches.',
    retry_target: 'Global retry target fallback when nodes do not define retry_target.',
    fallback_retry_target: 'Second fallback when retry_target is unset at node and graph scope.',
    'stack.child_dotfile': 'Child flow DOT path used by manager-loop/stack handlers when relevant.',
    'stack.child_workdir': 'Working directory for child flow execution when stack handlers invoke child runs.',
    'tool.hooks.pre': 'Command run before tool execution unless runtime/node-level override replaces it.',
    'tool.hooks.post': 'Command run after tool execution unless runtime/node-level override replaces it.',
}

export const MODEL_VALUE_SOURCE_LABEL: Record<ModelValueSource, string> = {
    node: 'node',
    stylesheet: 'stylesheet',
    graph_default: 'graph default',
    system_default: 'system default',
}

export const CORE_GRAPH_ATTR_KEYS = new Set<string>([
    'spark.title',
    'spark.description',
    'spark.launch_inputs',
    'goal',
    'label',
    'model_stylesheet',
    'default_max_retries',
    'retry_target',
    'fallback_retry_target',
    'default_fidelity',
    'stack.child_dotfile',
    'stack.child_workdir',
    'tool.hooks.pre',
    'tool.hooks.post',
    'ui_default_llm_model',
    'ui_default_llm_provider',
    'ui_default_llm_profile',
    'ui_default_reasoning_effort',
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
    graphAttrs: GraphAttrs
    updateGraphAttr: (key: keyof GraphAttrs, value: string) => void
}

export function GraphMetadataSection({
    graphAttrs,
    updateGraphAttr,
}: GraphMetadataSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Graph Metadata"
                description="Human-facing title and description stored in DOT metadata for the flow."
            />
            <div className="space-y-3">
                <GraphSettingsField
                    label="Title"
                    htmlFor="graph-attr-spark-title"
                    helper={GRAPH_ATTR_HELP['spark.title']}
                >
                    <Input
                        id="graph-attr-spark-title"
                        value={graphAttrs['spark.title'] || ''}
                        onChange={(event) => updateGraphAttr('spark.title', event.target.value)}
                        className="h-8 text-xs"
                        placeholder="Implement From Plan File"
                    />
                </GraphSettingsField>
                <GraphSettingsField
                    label="Description"
                    htmlFor="graph-attr-spark-description"
                    helper={GRAPH_ATTR_HELP['spark.description']}
                >
                    <Textarea
                        id="graph-attr-spark-description"
                        value={graphAttrs['spark.description'] || ''}
                        onChange={(event) => updateGraphAttr('spark.description', event.target.value)}
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

interface GraphExecutionDefaultsSectionProps {
    graphAttrs: GraphAttrs
    graphAttrErrors: GraphAttrErrors
    renderFieldDiagnostics: (field: string, testId: string) => ReactNode
    updateGraphAttr: (key: keyof GraphAttrs, value: string) => void
}

export function GraphExecutionDefaultsSection({
    graphAttrs,
    graphAttrErrors,
    renderFieldDiagnostics,
    updateGraphAttr,
}: GraphExecutionDefaultsSectionProps) {
    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Execution Defaults"
                description="Graph-level defaults that shape retry behavior and baseline run context."
            />
            <GraphSettingsNotice
                data-testid="graph-attrs-help"
                className="text-[11px]"
            >
                <p>Graph attributes are baseline defaults. Explicit node and edge attrs win when both are set.</p>
                <p>Leave blank to omit this attr from DOT output.</p>
            </GraphSettingsNotice>
            <div className="space-y-3">
                <div className="space-y-1">
                    <GraphSettingsField
                        label="Goal"
                        htmlFor="graph-attr-goal"
                        helper={GRAPH_ATTR_HELP.goal}
                    >
                        <Input
                            id="graph-attr-goal"
                            value={graphAttrs.goal || ''}
                            onChange={(event) => updateGraphAttr('goal', event.target.value)}
                            className="h-8 text-xs"
                        />
                    </GraphSettingsField>
                </div>
                <GraphSettingsField
                    label="Label"
                    htmlFor="graph-attr-label"
                    helper={GRAPH_ATTR_HELP.label}
                >
                    <Input
                        id="graph-attr-label"
                        value={graphAttrs.label || ''}
                        onChange={(event) => updateGraphAttr('label', event.target.value)}
                        className="h-8 text-xs"
                    />
                </GraphSettingsField>
                <div className="grid grid-cols-2 gap-3">
                    <div className="space-y-1">
                        <GraphSettingsField
                            label="Default Max Retries"
                            htmlFor="graph-attr-default-max-retries"
                            helper={GRAPH_ATTR_HELP.default_max_retries}
                            error={graphAttrErrors.default_max_retries}
                        >
                            <Input
                                id="graph-attr-default-max-retries"
                                type="number"
                                min={0}
                                step={1}
                                inputMode="numeric"
                                value={graphAttrs.default_max_retries ?? ''}
                                onChange={(event) => updateGraphAttr('default_max_retries', event.target.value)}
                                className="h-8 text-xs"
                            />
                        </GraphSettingsField>
                        {renderFieldDiagnostics('default_max_retries', 'graph-field-diagnostics-default_max_retries')}
                    </div>
                    <div className="space-y-1">
                        <GraphSettingsField
                            label="Default Fidelity"
                            htmlFor="graph-attr-default-fidelity"
                            helper={GRAPH_ATTR_HELP.default_fidelity}
                            error={graphAttrErrors.default_fidelity}
                        >
                            <Input
                                id="graph-attr-default-fidelity"
                                value={graphAttrs.default_fidelity || ''}
                                onChange={(event) => updateGraphAttr('default_fidelity', event.target.value)}
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
                        {renderFieldDiagnostics('default_fidelity', 'graph-field-diagnostics-default_fidelity')}
                    </div>
                </div>
            </div>
        </section>
    )
}

interface GraphLaunchPolicySectionProps {
    activeFlow: string | null
    launchPolicy: FlowLaunchPolicy
    launchPolicyLoadState: 'idle' | 'loading' | 'ready' | 'error'
    launchPolicySaveState: 'idle' | 'saving' | 'saved' | 'error'
    launchPolicyStatusMessage: string
    onLaunchPolicyChange: (policy: FlowLaunchPolicy) => void | Promise<void>
}

export function GraphLaunchPolicySection({
    activeFlow,
    launchPolicy,
    launchPolicyLoadState,
    launchPolicySaveState,
    launchPolicyStatusMessage,
    onLaunchPolicyChange,
}: GraphLaunchPolicySectionProps) {
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
                    disabled={!activeFlow || launchPolicyLoadState !== 'ready' || launchPolicySaveState === 'saving'}
                    className="h-8 text-xs"
                >
                    {Object.entries(FLOW_LAUNCH_POLICY_LABELS).map(([value, label]) => (
                        <option key={value} value={value}>
                            {label}
                        </option>
                    ))}
                </NativeSelect>
            </GraphSettingsField>
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
    graphAttrs: GraphAttrs
    showAdvancedGraphAttrs: boolean
    graphExtensionEntries: ExtensionAttrEntry[]
    showStylesheetFeedback: boolean
    stylesheetDiagnostics: DiagnosticEntry[]
    stylesheetPreview: ModelStylesheetPreview
    toolHookPreWarning: string | null
    toolHookPostWarning: string | null
    renderFieldDiagnostics: (field: string, testId: string) => ReactNode
    updateGraphAttr: (key: keyof GraphAttrs, value: string) => void
    setShowAdvancedGraphAttrs: (value: boolean | ((current: boolean) => boolean)) => void
    onGraphExtensionValueChange: (key: string, value: string) => void
    onGraphExtensionRemove: (key: string) => void
    onGraphExtensionAdd: (key: string, value: string) => void
}

export function GraphAdvancedAttrsSection({
    graphAttrs,
    showAdvancedGraphAttrs,
    graphExtensionEntries,
    showStylesheetFeedback,
    stylesheetDiagnostics,
    stylesheetPreview,
    toolHookPreWarning,
    toolHookPostWarning,
    renderFieldDiagnostics,
    updateGraphAttr,
    setShowAdvancedGraphAttrs,
    onGraphExtensionValueChange,
    onGraphExtensionRemove,
    onGraphExtensionAdd,
}: GraphAdvancedAttrsSectionProps) {
    const stylesheetNoticeTone = stylesheetDiagnostics.some((diag) => diag.severity === 'error')
        ? 'error'
        : stylesheetDiagnostics.some((diag) => diag.severity === 'warning')
            ? 'warning'
            : 'success'

    return (
        <section className="space-y-3">
            <GraphSettingsSectionIntro
                title="Advanced Attrs"
                description="Low-frequency graph attrs, stylesheet defaults, and extension metadata."
                action={(
                    <Button
                        type="button"
                        data-testid="graph-advanced-toggle"
                        variant="outline"
                        size="sm"
                        className="h-8 px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground"
                        onClick={() => setShowAdvancedGraphAttrs((current) => !current)}
                    >
                        {showAdvancedGraphAttrs ? 'Hide Advanced Fields' : 'Show Advanced Fields'}
                    </Button>
                )}
            />
            {showAdvancedGraphAttrs ? (
                <div className="space-y-3 rounded-md border border-border/80 bg-background/40 p-3">
                    <div className="space-y-1">
                        <GraphSettingsField
                            label="Model Stylesheet"
                            htmlFor="graph-model-stylesheet"
                            helper={GRAPH_ATTR_HELP.model_stylesheet}
                        >
                            <div data-testid="graph-model-stylesheet-editor">
                                <StylesheetEditor
                                    id="graph-model-stylesheet"
                                    value={graphAttrs.model_stylesheet || ''}
                                    onChange={(value) => updateGraphAttr('model_stylesheet', value)}
                                    ariaLabel="Model Stylesheet"
                                />
                            </div>
                        </GraphSettingsField>
                        <p
                            data-testid="graph-model-stylesheet-selector-guidance"
                            className="text-[11px] text-muted-foreground"
                        >
                            Supported selectors: `*`, `shape`, `.class`, `#id`. End each declaration with `;`.
                        </p>
                        {showStylesheetFeedback ? (
                            <GraphSettingsNotice
                                data-testid="graph-model-stylesheet-diagnostics"
                                tone={stylesheetNoticeTone}
                                className="text-[11px]"
                            >
                                {stylesheetDiagnostics.length > 0 ? (
                                    <div className="space-y-1">
                                        {stylesheetDiagnostics.map((diag, index) => (
                                            <p key={`${diag.rule_id}-${diag.line ?? 'line'}-${index}`}>
                                                {diag.message}
                                                {diag.line ? ` (line ${diag.line})` : ''}
                                            </p>
                                        ))}
                                    </div>
                                ) : (
                                    <p>Stylesheet parse and selector lint checks passed in preview.</p>
                                )}
                            </GraphSettingsNotice>
                        ) : null}
                        <div
                            data-testid="graph-model-stylesheet-selector-preview"
                            className="rounded-md border border-border/70 bg-background px-2 py-2"
                        >
                            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                                Matching selectors
                            </p>
                            {stylesheetPreview.selectorPreview.length > 0 ? (
                                <div className="mt-2 space-y-1">
                                    {stylesheetPreview.selectorPreview.map((entry, index) => (
                                        <p key={`${entry.selector}-${index}`} className="text-[11px] text-foreground">
                                            <span className="font-mono">{entry.selector}</span>
                                            {' -> '}
                                            {entry.matchedNodeIds.length > 0 ? entry.matchedNodeIds.join(', ') : 'No nodes matched'}
                                        </p>
                                    ))}
                                </div>
                            ) : (
                                <p className="mt-2 text-[11px] text-muted-foreground">No valid selectors parsed yet.</p>
                            )}
                        </div>
                        <div
                            data-testid="graph-model-stylesheet-effective-preview"
                            className="rounded-md border border-border/70 bg-background px-2 py-2"
                        >
                            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                                Effective per-node values
                            </p>
                            <p
                                data-testid="graph-model-stylesheet-precedence-guidance"
                                className="mt-1 text-[11px] text-muted-foreground"
                            >
                                Precedence: node attr &gt; stylesheet &gt; graph default &gt; system default.
                            </p>
                            {stylesheetPreview.nodePreview.length > 0 ? (
                                <div className="mt-2 space-y-2">
                                    {stylesheetPreview.nodePreview.map((node) => (
                                        <div key={node.nodeId} className="rounded border border-border/60 bg-muted/10 px-2 py-1">
                                            <p className="text-[11px] text-foreground">
                                                <span className="font-mono">{node.nodeId}</span>
                                                {node.matchedSelectors.length > 0
                                                    ? ` • selectors: ${node.matchedSelectors.join(', ')}`
                                                    : ' • selectors: none'}
                                            </p>
                                            <p className="text-[11px] text-muted-foreground">
                                                llm_model: {node.effective.llm_model.value || '(empty)'} ({MODEL_VALUE_SOURCE_LABEL[node.effective.llm_model.source]})
                                            </p>
                                            <p className="text-[11px] text-muted-foreground">
                                                llm_provider: {node.effective.llm_provider.value || '(empty)'} ({MODEL_VALUE_SOURCE_LABEL[node.effective.llm_provider.source]})
                                            </p>
                                            <p className="text-[11px] text-muted-foreground">
                                                llm_profile: {node.effective.llm_profile.value || '(empty)'} ({MODEL_VALUE_SOURCE_LABEL[node.effective.llm_profile.source]})
                                            </p>
                                            <p className="text-[11px] text-muted-foreground">
                                                reasoning_effort: {node.effective.reasoning_effort.value || '(empty)'} ({MODEL_VALUE_SOURCE_LABEL[node.effective.reasoning_effort.source]})
                                            </p>
                                        </div>
                                    ))}
                                </div>
                            ) : (
                                <p className="mt-2 text-[11px] text-muted-foreground">No nodes available yet.</p>
                            )}
                        </div>
                    </div>
                    <div className="space-y-1">
                        <GraphSettingsField
                            label="Retry Target"
                            htmlFor="graph-attr-retry-target"
                            helper={GRAPH_ATTR_HELP.retry_target}
                        >
                            <Input
                                id="graph-attr-retry-target"
                                value={graphAttrs.retry_target || ''}
                                onChange={(event) => updateGraphAttr('retry_target', event.target.value)}
                                className="h-8 text-xs"
                            />
                        </GraphSettingsField>
                        {renderFieldDiagnostics('retry_target', 'graph-field-diagnostics-retry_target')}
                    </div>
                    <div className="space-y-1">
                        <GraphSettingsField
                            label="Fallback Retry Target"
                            htmlFor="graph-attr-fallback-retry-target"
                            helper={GRAPH_ATTR_HELP.fallback_retry_target}
                        >
                            <Input
                                id="graph-attr-fallback-retry-target"
                                value={graphAttrs.fallback_retry_target || ''}
                                onChange={(event) => updateGraphAttr('fallback_retry_target', event.target.value)}
                                className="h-8 text-xs"
                            />
                        </GraphSettingsField>
                        {renderFieldDiagnostics('fallback_retry_target', 'graph-field-diagnostics-fallback_retry_target')}
                    </div>
                    <GraphSettingsField
                        label="Stack Child Dotfile"
                        htmlFor="graph-attr-stack-child-dotfile"
                        helper={GRAPH_ATTR_HELP['stack.child_dotfile']}
                    >
                        <Input
                            id="graph-attr-stack-child-dotfile"
                            value={graphAttrs['stack.child_dotfile'] || ''}
                            onChange={(event) => updateGraphAttr('stack.child_dotfile', event.target.value)}
                            className="h-8 font-mono text-xs"
                            placeholder="child/flow.dot"
                        />
                    </GraphSettingsField>
                    <GraphSettingsField
                        label="Stack Child Workdir"
                        htmlFor="graph-attr-stack-child-workdir"
                        helper={GRAPH_ATTR_HELP['stack.child_workdir']}
                    >
                        <Input
                            id="graph-attr-stack-child-workdir"
                            value={graphAttrs['stack.child_workdir'] || ''}
                            onChange={(event) => updateGraphAttr('stack.child_workdir', event.target.value)}
                            className="h-8 font-mono text-xs"
                            placeholder="/abs/path/to/child"
                        />
                    </GraphSettingsField>
                    <GraphSettingsField
                        label="Tool Hooks Pre"
                        htmlFor="graph-attr-tool-hooks-pre"
                        helper={GRAPH_ATTR_HELP['tool.hooks.pre']}
                    >
                        <Input
                            id="graph-attr-tool-hooks-pre"
                            data-testid="graph-attr-input-tool.hooks.pre"
                            value={graphAttrs['tool.hooks.pre'] || ''}
                            onChange={(event) => updateGraphAttr('tool.hooks.pre', event.target.value)}
                            className="h-8 font-mono text-xs"
                        />
                    </GraphSettingsField>
                    {toolHookPreWarning ? (
                        <p data-testid="graph-attr-warning-tool.hooks.pre" className="text-[11px] text-amber-800">
                            {toolHookPreWarning}
                        </p>
                    ) : null}
                    <GraphSettingsField
                        label="Tool Hooks Post"
                        htmlFor="graph-attr-tool-hooks-post"
                        helper={GRAPH_ATTR_HELP['tool.hooks.post']}
                    >
                        <Input
                            id="graph-attr-tool-hooks-post"
                            data-testid="graph-attr-input-tool.hooks.post"
                            value={graphAttrs['tool.hooks.post'] || ''}
                            onChange={(event) => updateGraphAttr('tool.hooks.post', event.target.value)}
                            className="h-8 font-mono text-xs"
                        />
                    </GraphSettingsField>
                    {toolHookPostWarning ? (
                        <p data-testid="graph-attr-warning-tool.hooks.post" className="text-[11px] text-amber-800">
                            {toolHookPostWarning}
                        </p>
                    ) : null}
                    <AdvancedKeyValueEditor
                        testIdPrefix="graph"
                        entries={graphExtensionEntries}
                        onValueChange={onGraphExtensionValueChange}
                        onRemove={onGraphExtensionRemove}
                        onAdd={onGraphExtensionAdd}
                        reservedKeys={CORE_GRAPH_ATTR_KEYS}
                    />
                </div>
            ) : (
                <GraphSettingsNotice className="text-[11px]">
                    Advanced attrs stay available for stylesheet defaults, retry fallbacks, stack child linkage, and extension metadata.
                </GraphSettingsNotice>
            )}
        </section>
    )
}

interface GraphLlmDefaultsSectionProps {
    canApplyDefaults: boolean
    flowProviderFallback: string
    graphAttrs: GraphAttrs
    uiDefaults: UiDefaults
    llmProfiles: LlmProfileMetadata[]
    applyDefaultsToNodes: () => void
    updateGraphAttr: (key: keyof GraphAttrs, value: string) => void
}

export function GraphLlmDefaultsSection({
    canApplyDefaults,
    flowProviderFallback,
    graphAttrs,
    uiDefaults,
    llmProfiles,
    applyDefaultsToNodes,
    updateGraphAttr,
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
                        value={graphAttrs.ui_default_llm_profile || graphAttrs.ui_default_llm_provider || ''}
                        onChange={(event) => {
                            const selection = splitLlmSelection(event.target.value, llmProfiles)
                            updateGraphAttr('ui_default_llm_provider', selection.llm_provider)
                            updateGraphAttr('ui_default_llm_profile', selection.llm_profile)
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
                        value={graphAttrs.ui_default_llm_model || ''}
                        onChange={(event) => updateGraphAttr('ui_default_llm_model', event.target.value)}
                        list="flow-llm-model-options"
                        className="h-8 text-xs"
                        placeholder={uiDefaults.llm_model ? `Snapshot: ${uiDefaults.llm_model}` : 'Snapshot of global default'}
                    />
                    <datalist id="flow-llm-model-options">
                        {getModelSuggestions(graphAttrs.ui_default_llm_profile || flowProviderFallback, llmProfiles).map((modelOption) => (
                            <option key={modelOption} value={modelOption} />
                        ))}
                    </datalist>
                </GraphSettingsField>
                <GraphSettingsField label="Default Reasoning Effort" htmlFor="graph-default-reasoning-effort">
                    <NativeSelect
                        id="graph-default-reasoning-effort"
                        value={graphAttrs.ui_default_reasoning_effort || ''}
                        onChange={(event) => updateGraphAttr('ui_default_reasoning_effort', event.target.value)}
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
                            updateGraphAttr('ui_default_llm_provider', uiDefaults.llm_provider)
                            updateGraphAttr('ui_default_llm_profile', uiDefaults.llm_profile)
                            updateGraphAttr('ui_default_llm_model', uiDefaults.llm_model)
                            updateGraphAttr('ui_default_reasoning_effort', uiDefaults.reasoning_effort)
                        }}
                    >
                        Reset From Global
                    </Button>
                </div>
            </div>
        </section>
    )
}
