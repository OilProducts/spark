import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useNodes, useReactFlow } from '@xyflow/react'
import { useStore, type DiagnosticEntry } from '@/store'
import { generateDot } from '@/lib/dotUtils'
import { getModelSuggestions, LLM_PROVIDER_OPTIONS } from '@/lib/llmSuggestions'
import { GRAPH_FIDELITY_OPTIONS, getToolHookCommandWarning } from '@/lib/graphAttrValidation'
import { resolveGraphFieldDiagnostics } from '@/lib/inspectorFieldDiagnostics'
import { saveFlowContent } from '@/lib/flowPersistence'
import { resolveModelStylesheetPreview, type ModelValueSource } from '@/lib/modelStylesheetPreview'
import { toExtensionAttrEntries } from '@/lib/extensionAttrs'
import { InspectorScaffold } from './InspectorScaffold'
import { StylesheetEditor } from './StylesheetEditor'
import { AdvancedKeyValueEditor } from './AdvancedKeyValueEditor'

interface GraphSettingsProps {
    inline?: boolean
}

const GRAPH_ATTR_HELP: Record<string, string> = {
    goal: 'Primary graph intent used by handlers that read graph-level goal context.',
    label: 'Display label for graph metadata; does not override node labels.',
    default_max_retry: 'Used only when a node omits max_retries. Node max_retries takes precedence.',
    default_fidelity: 'Default fidelity when node/edge fidelity is not set explicitly.',
    model_stylesheet: 'Selector-based model defaults. Explicit node attrs override stylesheet matches.',
    retry_target: 'Global retry target fallback when nodes do not define retry_target.',
    fallback_retry_target: 'Second fallback when retry_target is unset at node and graph scope.',
    'stack.child_dotfile': 'Child flow DOT path used by manager-loop/stack handlers when relevant.',
    'stack.child_workdir': 'Working directory for child flow execution when stack handlers invoke child runs.',
    'tool_hooks.pre': 'Command run before tool execution unless runtime/node-level override replaces it.',
    'tool_hooks.post': 'Command run after tool execution unless runtime/node-level override replaces it.',
}

const MODEL_VALUE_SOURCE_LABEL: Record<ModelValueSource, string> = {
    node: 'node',
    stylesheet: 'stylesheet',
    graph_default: 'graph default',
    system_default: 'system default',
}

const CORE_GRAPH_ATTR_KEYS = new Set<string>([
    'goal',
    'label',
    'model_stylesheet',
    'default_max_retry',
    'retry_target',
    'fallback_retry_target',
    'default_fidelity',
    'stack.child_dotfile',
    'stack.child_workdir',
    'tool_hooks.pre',
    'tool_hooks.post',
    'ui_default_llm_model',
    'ui_default_llm_provider',
    'ui_default_reasoning_effort',
])

export function GraphSettings({ inline = false }: GraphSettingsProps) {
    const [isOpen, setIsOpen] = useState(false)
    const [showAdvancedGraphAttrs, setShowAdvancedGraphAttrs] = useState(false)
    const activeFlow = useStore((state) => state.activeFlow)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const diagnostics = useStore((state) => state.diagnostics)
    const graphAttrs = useStore((state) => state.graphAttrs)
    const graphAttrErrors = useStore((state) => state.graphAttrErrors)
    const setGraphAttrs = useStore((state) => state.setGraphAttrs)
    const updateGraphAttr = useStore((state) => state.updateGraphAttr)
    const model = useStore((state) => state.model)
    const setModel = useStore((state) => state.setModel)
    const workingDir = useStore((state) => state.workingDir)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const viewMode = useStore((state) => state.viewMode)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const { getNodes, getEdges, setNodes } = useReactFlow()
    const flowNodes = useNodes()
    const saveTimer = useRef<number | null>(null)
    const hasPendingSave = useRef(false)
    const autosaveScopeRef = useRef<string | null>(null)
    const flowProviderFallback = graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || ''
    const canApplyDefaults = !!activeProjectPath && !!activeFlow && viewMode === 'editor'
    const toolHookPreWarning = getToolHookCommandWarning(graphAttrs['tool_hooks.pre'] || '')
    const toolHookPostWarning = getToolHookCommandWarning(graphAttrs['tool_hooks.post'] || '')
    const stylesheetDiagnostics = diagnostics.filter((diag) => diag.rule_id === 'stylesheet_syntax')
    const hasStylesheetValue = Boolean(graphAttrs.model_stylesheet?.trim())
    const showStylesheetFeedback = hasStylesheetValue || stylesheetDiagnostics.length > 0
    const graphFieldDiagnostics = useMemo(() => resolveGraphFieldDiagnostics(diagnostics), [diagnostics])
    const graphExtensionEntries = useMemo(
        () => toExtensionAttrEntries(graphAttrs as Record<string, unknown>, CORE_GRAPH_ATTR_KEYS),
        [graphAttrs],
    )
    const stylesheetPreview = useMemo(() => {
        return resolveModelStylesheetPreview(
            graphAttrs.model_stylesheet || '',
            flowNodes.map((node) => ({
                id: node.id,
                class: typeof node.data?.class === 'string' ? node.data.class : '',
                llm_model: typeof node.data?.llm_model === 'string' ? node.data.llm_model : '',
                llm_provider: typeof node.data?.llm_provider === 'string' ? node.data.llm_provider : '',
                reasoning_effort: typeof node.data?.reasoning_effort === 'string' ? node.data.reasoning_effort : '',
            })),
            {
                llm_model: graphAttrs.ui_default_llm_model || uiDefaults.llm_model || '',
                llm_provider: graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || '',
                reasoning_effort: graphAttrs.ui_default_reasoning_effort || uiDefaults.reasoning_effort || 'high',
            },
        )
    }, [
        graphAttrs.model_stylesheet,
        graphAttrs.ui_default_llm_model,
        graphAttrs.ui_default_llm_provider,
        graphAttrs.ui_default_reasoning_effort,
        flowNodes,
        uiDefaults.llm_model,
        uiDefaults.llm_provider,
        uiDefaults.reasoning_effort,
    ])

    const flushPendingSave = useCallback(() => {
        if (!activeProjectPath || !activeFlow || !hasPendingSave.current) return
        hasPendingSave.current = false
        const dot = generateDot(activeFlow, getNodes(), getEdges(), graphAttrs)
        void saveFlowContent(activeFlow, dot)
    }, [activeProjectPath, activeFlow, getNodes, getEdges, graphAttrs])

    const applyDefaultsToNodes = () => {
        if (!activeProjectPath || !activeFlow) return
        const defaultModel = graphAttrs.ui_default_llm_model || uiDefaults.llm_model || ''
        const defaultProvider = graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || ''
        const defaultReasoning = graphAttrs.ui_default_reasoning_effort || uiDefaults.reasoning_effort || ''

        const currentNodes = getNodes()
        if (currentNodes.length === 0) return

        const updatedNodes = currentNodes.map((node) => ({
            ...node,
            data: {
                ...node.data,
                llm_model: defaultModel,
                llm_provider: defaultProvider,
                reasoning_effort: defaultReasoning,
            },
        }))

        setNodes(updatedNodes)

        const dot = generateDot(activeFlow, updatedNodes, getEdges(), graphAttrs)
        void saveFlowContent(activeFlow, dot)
    }

    useEffect(() => {
        if (!activeProjectPath || !activeFlow) {
            autosaveScopeRef.current = null
            return
        }
        const autosaveScope = `${activeProjectPath}::${activeFlow}`
        if (autosaveScopeRef.current !== autosaveScope) {
            autosaveScopeRef.current = autosaveScope
            return
        }
        hasPendingSave.current = true
        if (saveTimer.current) {
            window.clearTimeout(saveTimer.current)
        }
        saveTimer.current = window.setTimeout(() => {
            hasPendingSave.current = false
            const dot = generateDot(activeFlow, getNodes(), getEdges(), graphAttrs)
            void saveFlowContent(activeFlow, dot)
        }, 200)

        return () => {
            if (saveTimer.current) {
                window.clearTimeout(saveTimer.current)
            }
        }
    }, [activeProjectPath, activeFlow, graphAttrs, getNodes, getEdges])

    useEffect(() => {
        const handleBeforeUnload = () => {
            flushPendingSave()
        }
        window.addEventListener('beforeunload', handleBeforeUnload)
        return () => {
            window.removeEventListener('beforeunload', handleBeforeUnload)
            flushPendingSave()
        }
    }, [flushPendingSave])

    const renderFieldDiagnostics = (field: string, testId: string) => {
        const diagnosticsForField = graphFieldDiagnostics[field] || []
        if (diagnosticsForField.length === 0) {
            return null
        }
        return (
            <div
                data-testid={testId}
                className="rounded-md border border-border/80 bg-muted/20 px-2 py-1"
            >
                <div className="space-y-1">
                    {diagnosticsForField.map((diag: DiagnosticEntry, index: number) => {
                        const severityClassName = diag.severity === 'error'
                            ? 'text-destructive'
                            : diag.severity === 'warning'
                                ? 'text-amber-800'
                                : 'text-sky-700'
                        return (
                            <p key={`${field}-${diag.rule_id}-${index}`} className={`text-[11px] ${severityClassName}`}>
                                {diag.message}
                            </p>
                        )
                    })}
                </div>
            </div>
        )
    }

    const handleGraphExtensionValueChange = (key: string, value: string) => {
        setGraphAttrs({
            ...graphAttrs,
            [key]: value,
        })
    }

    const handleGraphExtensionRemove = (key: string) => {
        const nextGraphAttrs = { ...graphAttrs } as Record<string, unknown>
        delete nextGraphAttrs[key]
        setGraphAttrs(nextGraphAttrs)
    }

    const handleGraphExtensionAdd = (key: string, value: string) => {
        setGraphAttrs({
            ...graphAttrs,
            [key]: value,
        })
    }

    const inspectorContent = (
        <InspectorScaffold
            scopeLabel="Graph"
            title="Settings"
            description="Use the same inspect-edit flow as node and edge inspectors."
            entityLabel="Flow"
            entityValue={activeFlow || undefined}
        >
            <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Run Configuration
            </div>
            <div className="mt-3 space-y-3">
                <div className="space-y-1">
                    <label htmlFor="graph-run-model" className="text-xs font-medium text-foreground">
                        Model
                    </label>
                    <input
                        id="graph-run-model"
                        value={model}
                        onChange={(event) => setModel(event.target.value)}
                        className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        placeholder="codex default"
                    />
                </div>
                <div className="space-y-1">
                    <label htmlFor="graph-run-working-directory" className="text-xs font-medium text-foreground">
                        Working Directory
                    </label>
                    <input
                        id="graph-run-working-directory"
                        value={workingDir}
                        onChange={(event) => setWorkingDir(event.target.value)}
                        className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        placeholder="./test-app"
                    />
                </div>
            </div>

            <div data-testid="graph-structured-form">
                <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                    Graph Attributes
                </div>
                <div className="mt-3 space-y-3">
                    <div
                        data-testid="graph-attrs-help"
                        className="rounded-md border border-border/80 bg-muted/20 px-2 py-1 text-[11px] text-muted-foreground"
                    >
                        <p>Graph attributes are baseline defaults. Explicit node and edge attrs win when both are set.</p>
                        <p>Leave blank to omit this attr from DOT output.</p>
                    </div>
                    <div className="space-y-1">
                        <label htmlFor="graph-attr-goal" className="text-xs font-medium text-foreground">
                            Goal
                        </label>
                        <input
                            id="graph-attr-goal"
                            value={graphAttrs.goal || ''}
                            onChange={(event) => updateGraphAttr('goal', event.target.value)}
                            className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        />
                        <p data-testid="graph-attr-help-goal" className="text-[11px] text-muted-foreground">
                            {GRAPH_ATTR_HELP.goal}
                        </p>
                    </div>
                    <div className="space-y-1">
                        <label htmlFor="graph-attr-label" className="text-xs font-medium text-foreground">
                            Label
                        </label>
                        <input
                            id="graph-attr-label"
                            value={graphAttrs.label || ''}
                            onChange={(event) => updateGraphAttr('label', event.target.value)}
                            className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        />
                        <p data-testid="graph-attr-help-label" className="text-[11px] text-muted-foreground">
                            {GRAPH_ATTR_HELP.label}
                        </p>
                    </div>
                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1">
                            <label htmlFor="graph-attr-default-max-retry" className="text-xs font-medium text-foreground">
                                Default Max Retry
                            </label>
                            <input
                                id="graph-attr-default-max-retry"
                                value={graphAttrs.default_max_retry ?? ''}
                                onChange={(event) => updateGraphAttr('default_max_retry', event.target.value)}
                                type="number"
                                min={0}
                                step={1}
                                inputMode="numeric"
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            />
                            <p data-testid="graph-attr-help-default_max_retry" className="text-[11px] text-muted-foreground">
                                {GRAPH_ATTR_HELP.default_max_retry}
                            </p>
                            {graphAttrErrors.default_max_retry && (
                                <p className="text-[11px] text-destructive">
                                    {graphAttrErrors.default_max_retry}
                                </p>
                            )}
                        </div>
                        <div className="space-y-1">
                            <label htmlFor="graph-attr-default-fidelity" className="text-xs font-medium text-foreground">
                                Default Fidelity
                            </label>
                            <input
                                id="graph-attr-default-fidelity"
                                value={graphAttrs.default_fidelity || ''}
                                onChange={(event) => updateGraphAttr('default_fidelity', event.target.value)}
                                list="graph-fidelity-options"
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                placeholder="full"
                            />
                            <datalist id="graph-fidelity-options">
                                {GRAPH_FIDELITY_OPTIONS.map((option) => (
                                    <option key={option} value={option} />
                                ))}
                            </datalist>
                            <p data-testid="graph-attr-help-default_fidelity" className="text-[11px] text-muted-foreground">
                                {GRAPH_ATTR_HELP.default_fidelity}
                            </p>
                            {graphAttrErrors.default_fidelity && (
                                <p className="text-[11px] text-destructive">
                                    {graphAttrErrors.default_fidelity}
                                </p>
                            )}
                            {renderFieldDiagnostics('default_fidelity', 'graph-field-diagnostics-default_fidelity')}
                        </div>
                    </div>
                    <button
                        type="button"
                        data-testid="graph-advanced-toggle"
                        onClick={() => setShowAdvancedGraphAttrs((current) => !current)}
                        className="h-8 w-full rounded-md border border-border bg-background px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    >
                        {showAdvancedGraphAttrs ? 'Hide Advanced Fields' : 'Show Advanced Fields'}
                    </button>
                    {showAdvancedGraphAttrs && (
                        <div className="space-y-3 rounded-md border border-border/80 bg-background/40 p-3">
                            <div className="space-y-1">
                                <label htmlFor="graph-model-stylesheet" className="text-xs font-medium text-foreground">
                                    Model Stylesheet
                                </label>
                                <div data-testid="graph-model-stylesheet-editor">
                                    <StylesheetEditor
                                        id="graph-model-stylesheet"
                                        value={graphAttrs.model_stylesheet || ''}
                                        onChange={(value) => updateGraphAttr('model_stylesheet', value)}
                                        ariaLabel="Model Stylesheet"
                                    />
                                </div>
                                <p data-testid="graph-attr-help-model_stylesheet" className="text-[11px] text-muted-foreground">
                                    {GRAPH_ATTR_HELP.model_stylesheet}
                                </p>
                                <p
                                    data-testid="graph-model-stylesheet-selector-guidance"
                                    className="text-[11px] text-muted-foreground"
                                >
                                    Supported selectors: `*`, `.class`, `#id`. End each declaration with `;`.
                                </p>
                                {showStylesheetFeedback && (
                                    <div
                                        data-testid="graph-model-stylesheet-diagnostics"
                                        className="rounded-md border border-border/70 bg-muted/20 px-2 py-1"
                                    >
                                        {stylesheetDiagnostics.length > 0 ? (
                                            <div className="space-y-1">
                                                {stylesheetDiagnostics.map((diag, index) => {
                                                    const severityClassName = diag.severity === 'error'
                                                        ? 'text-destructive'
                                                        : diag.severity === 'warning'
                                                            ? 'text-amber-800'
                                                            : 'text-sky-700'
                                                    return (
                                                        <p
                                                            key={`${diag.rule_id}-${diag.line ?? 'line'}-${index}`}
                                                            className={`text-[11px] ${severityClassName}`}
                                                        >
                                                            {diag.message}
                                                            {diag.line ? ` (line ${diag.line})` : ''}
                                                        </p>
                                                    )
                                                })}
                                            </div>
                                        ) : (
                                            <p className="text-[11px] text-emerald-700">
                                                Stylesheet parse and selector lint checks passed in preview.
                                            </p>
                                        )}
                                    </div>
                                )}
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
                                        <p className="mt-2 text-[11px] text-muted-foreground">
                                            No valid selectors parsed yet.
                                        </p>
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
                                                        reasoning_effort: {node.effective.reasoning_effort.value || '(empty)'} ({MODEL_VALUE_SOURCE_LABEL[node.effective.reasoning_effort.source]})
                                                    </p>
                                                </div>
                                            ))}
                                        </div>
                                    ) : (
                                        <p className="mt-2 text-[11px] text-muted-foreground">
                                            No nodes available yet.
                                        </p>
                                    )}
                                </div>
                            </div>
                            <div className="space-y-1">
                                <label htmlFor="graph-attr-retry-target" className="text-xs font-medium text-foreground">
                                    Retry Target
                                </label>
                                <input
                                    id="graph-attr-retry-target"
                                    value={graphAttrs.retry_target || ''}
                                    onChange={(event) => updateGraphAttr('retry_target', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                                <p data-testid="graph-attr-help-retry_target" className="text-[11px] text-muted-foreground">
                                    {GRAPH_ATTR_HELP.retry_target}
                                </p>
                                {renderFieldDiagnostics('retry_target', 'graph-field-diagnostics-retry_target')}
                            </div>
                            <div className="space-y-1">
                                <label htmlFor="graph-attr-fallback-retry-target" className="text-xs font-medium text-foreground">
                                    Fallback Retry Target
                                </label>
                                <input
                                    id="graph-attr-fallback-retry-target"
                                    value={graphAttrs.fallback_retry_target || ''}
                                    onChange={(event) => updateGraphAttr('fallback_retry_target', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                                <p data-testid="graph-attr-help-fallback_retry_target" className="text-[11px] text-muted-foreground">
                                    {GRAPH_ATTR_HELP.fallback_retry_target}
                                </p>
                                {renderFieldDiagnostics('fallback_retry_target', 'graph-field-diagnostics-fallback_retry_target')}
                            </div>
                            <div className="space-y-1">
                                <label htmlFor="graph-attr-stack-child-dotfile" className="text-xs font-medium text-foreground">
                                    Stack Child Dotfile
                                </label>
                                <input
                                    id="graph-attr-stack-child-dotfile"
                                    value={graphAttrs['stack.child_dotfile'] || ''}
                                    onChange={(event) => updateGraphAttr('stack.child_dotfile', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    placeholder="child/flow.dot"
                                />
                                <p data-testid="graph-attr-help-stack.child_dotfile" className="text-[11px] text-muted-foreground">
                                    {GRAPH_ATTR_HELP['stack.child_dotfile']}
                                </p>
                            </div>
                            <div className="space-y-1">
                                <label htmlFor="graph-attr-stack-child-workdir" className="text-xs font-medium text-foreground">
                                    Stack Child Workdir
                                </label>
                                <input
                                    id="graph-attr-stack-child-workdir"
                                    value={graphAttrs['stack.child_workdir'] || ''}
                                    onChange={(event) => updateGraphAttr('stack.child_workdir', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    placeholder="/abs/path/to/child"
                                />
                                <p data-testid="graph-attr-help-stack.child_workdir" className="text-[11px] text-muted-foreground">
                                    {GRAPH_ATTR_HELP['stack.child_workdir']}
                                </p>
                            </div>
                            <div className="space-y-1">
                                <label htmlFor="graph-attr-tool-hooks-pre" className="text-xs font-medium text-foreground">
                                    Tool Hooks Pre
                                </label>
                                <input
                                    id="graph-attr-tool-hooks-pre"
                                    data-testid="graph-attr-input-tool_hooks.pre"
                                    value={graphAttrs['tool_hooks.pre'] || ''}
                                    onChange={(event) => updateGraphAttr('tool_hooks.pre', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                                <p data-testid="graph-attr-help-tool_hooks.pre" className="text-[11px] text-muted-foreground">
                                    {GRAPH_ATTR_HELP['tool_hooks.pre']}
                                </p>
                                {toolHookPreWarning && (
                                    <p data-testid="graph-attr-warning-tool_hooks.pre" className="text-[11px] text-amber-800">
                                        {toolHookPreWarning}
                                    </p>
                                )}
                            </div>
                            <div className="space-y-1">
                                <label htmlFor="graph-attr-tool-hooks-post" className="text-xs font-medium text-foreground">
                                    Tool Hooks Post
                                </label>
                                <input
                                    id="graph-attr-tool-hooks-post"
                                    data-testid="graph-attr-input-tool_hooks.post"
                                    value={graphAttrs['tool_hooks.post'] || ''}
                                    onChange={(event) => updateGraphAttr('tool_hooks.post', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                                <p data-testid="graph-attr-help-tool_hooks.post" className="text-[11px] text-muted-foreground">
                                    {GRAPH_ATTR_HELP['tool_hooks.post']}
                                </p>
                                {toolHookPostWarning && (
                                    <p data-testid="graph-attr-warning-tool_hooks.post" className="text-[11px] text-amber-800">
                                        {toolHookPostWarning}
                                    </p>
                                )}
                            </div>
                            <AdvancedKeyValueEditor
                                testIdPrefix="graph"
                                entries={graphExtensionEntries}
                                onValueChange={handleGraphExtensionValueChange}
                                onRemove={handleGraphExtensionRemove}
                                onAdd={handleGraphExtensionAdd}
                                reservedKeys={CORE_GRAPH_ATTR_KEYS}
                            />
                        </div>
                    )}
                </div>
            </div>

            <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                LLM Defaults (Flow Snapshot)
            </div>
            <div className="mt-3 space-y-3">
                <div className="space-y-1">
                    <label htmlFor="graph-default-llm-provider" className="text-xs font-medium text-foreground">
                        Default LLM Provider
                    </label>
                    <input
                        id="graph-default-llm-provider"
                        value={graphAttrs.ui_default_llm_provider || ''}
                        onChange={(event) => updateGraphAttr('ui_default_llm_provider', event.target.value)}
                        list="flow-llm-provider-options"
                        className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        placeholder={uiDefaults.llm_provider ? `Snapshot: ${uiDefaults.llm_provider}` : 'Snapshot of global default'}
                    />
                    <datalist id="flow-llm-provider-options">
                        {LLM_PROVIDER_OPTIONS.map((provider) => (
                            <option key={provider} value={provider} />
                        ))}
                    </datalist>
                </div>
                <div className="space-y-1">
                    <label htmlFor="graph-default-llm-model" className="text-xs font-medium text-foreground">
                        Default LLM Model
                    </label>
                    <input
                        id="graph-default-llm-model"
                        value={graphAttrs.ui_default_llm_model || ''}
                        onChange={(event) => updateGraphAttr('ui_default_llm_model', event.target.value)}
                        list="flow-llm-model-options"
                        className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        placeholder={uiDefaults.llm_model ? `Snapshot: ${uiDefaults.llm_model}` : 'Snapshot of global default'}
                    />
                    <datalist id="flow-llm-model-options">
                        {getModelSuggestions(flowProviderFallback).map((modelOption) => (
                            <option key={modelOption} value={modelOption} />
                        ))}
                    </datalist>
                </div>
                <div className="space-y-1">
                    <label htmlFor="graph-default-reasoning-effort" className="text-xs font-medium text-foreground">
                        Default Reasoning Effort
                    </label>
                    <select
                        id="graph-default-reasoning-effort"
                        value={graphAttrs.ui_default_reasoning_effort || ''}
                        onChange={(event) => updateGraphAttr('ui_default_reasoning_effort', event.target.value)}
                        className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    >
                        <option value="">Use global default</option>
                        <option value="low">Low</option>
                        <option value="medium">Medium</option>
                        <option value="high">High</option>
                    </select>
                </div>
                <div className="flex items-center justify-between gap-2">
                    <button
                        type="button"
                        onClick={applyDefaultsToNodes}
                        disabled={!canApplyDefaults}
                        className="h-8 rounded-md border border-border px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
                        title={canApplyDefaults ? 'Apply current flow defaults to every node.' : 'Switch to the editor to apply defaults.'}
                    >
                        Apply To Nodes
                    </button>
                    <button
                        type="button"
                        onClick={() => {
                            updateGraphAttr('ui_default_llm_provider', uiDefaults.llm_provider);
                            updateGraphAttr('ui_default_llm_model', uiDefaults.llm_model);
                            updateGraphAttr('ui_default_reasoning_effort', uiDefaults.reasoning_effort);
                        }}
                        className="h-8 rounded-md border border-border px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    >
                        Reset From Global
                    </button>
                </div>
            </div>
        </InspectorScaffold>
    )

    if (inline) {
        return inspectorContent
    }

    return (
        <div className="absolute right-4 top-4 z-20 flex flex-col items-end">
            <button
                onClick={() => setIsOpen((open) => !open)}
                className="inline-flex h-9 items-center gap-2 rounded-md border border-border bg-background/90 px-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground shadow-sm hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
                Graph Settings
            </button>
            {isOpen && (
                <div className="mt-2 w-80 max-h-[calc(100vh-6rem)] overflow-y-auto rounded-md border border-border bg-card p-4 shadow-lg">
                    {inspectorContent}
                </div>
            )}
        </div>
    )
}
