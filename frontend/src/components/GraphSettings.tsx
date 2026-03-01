import { useCallback, useEffect, useRef, useState } from 'react'
import { useReactFlow } from '@xyflow/react'
import { useStore } from '@/store'
import { generateDot } from '@/lib/dotUtils'
import { getModelSuggestions, LLM_PROVIDER_OPTIONS } from '@/lib/llmSuggestions'
import { GRAPH_FIDELITY_OPTIONS } from '@/lib/graphAttrValidation'
import { saveFlowContent } from '@/lib/flowPersistence'
import { InspectorScaffold } from './InspectorScaffold'

interface GraphSettingsProps {
    inline?: boolean
}

export function GraphSettings({ inline = false }: GraphSettingsProps) {
    const [isOpen, setIsOpen] = useState(false)
    const [showAdvancedGraphAttrs, setShowAdvancedGraphAttrs] = useState(false)
    const activeFlow = useStore((state) => state.activeFlow)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const graphAttrs = useStore((state) => state.graphAttrs)
    const graphAttrErrors = useStore((state) => state.graphAttrErrors)
    const updateGraphAttr = useStore((state) => state.updateGraphAttr)
    const model = useStore((state) => state.model)
    const setModel = useStore((state) => state.setModel)
    const workingDir = useStore((state) => state.workingDir)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const viewMode = useStore((state) => state.viewMode)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const { getNodes, getEdges, setNodes } = useReactFlow()
    const saveTimer = useRef<number | null>(null)
    const hasPendingSave = useRef(false)
    const flowProviderFallback = graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || ''
    const canApplyDefaults = !!activeProjectPath && !!activeFlow && viewMode === 'editor'

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
        if (!activeProjectPath || !activeFlow) return
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
                    <label className="text-xs font-medium text-foreground">Model</label>
                    <input
                        value={model}
                        onChange={(event) => setModel(event.target.value)}
                        className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        placeholder="codex default"
                    />
                </div>
                <div className="space-y-1">
                    <label className="text-xs font-medium text-foreground">Working Directory</label>
                    <input
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
                    <div className="space-y-1">
                        <label className="text-xs font-medium text-foreground">Goal</label>
                        <input
                            value={graphAttrs.goal || ''}
                            onChange={(event) => updateGraphAttr('goal', event.target.value)}
                            className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        />
                    </div>
                    <div className="space-y-1">
                        <label className="text-xs font-medium text-foreground">Label</label>
                        <input
                            value={graphAttrs.label || ''}
                            onChange={(event) => updateGraphAttr('label', event.target.value)}
                            className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        />
                    </div>
                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1">
                            <label className="text-xs font-medium text-foreground">Default Max Retry</label>
                            <input
                                value={graphAttrs.default_max_retry ?? ''}
                                onChange={(event) => updateGraphAttr('default_max_retry', event.target.value)}
                                type="number"
                                min={0}
                                step={1}
                                inputMode="numeric"
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            />
                            {graphAttrErrors.default_max_retry && (
                                <p className="text-[11px] text-destructive">
                                    {graphAttrErrors.default_max_retry}
                                </p>
                            )}
                        </div>
                        <div className="space-y-1">
                            <label className="text-xs font-medium text-foreground">Default Fidelity</label>
                            <input
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
                            {graphAttrErrors.default_fidelity && (
                                <p className="text-[11px] text-destructive">
                                    {graphAttrErrors.default_fidelity}
                                </p>
                            )}
                        </div>
                    </div>
                    <button
                        type="button"
                        data-testid="graph-advanced-toggle"
                        onClick={() => setShowAdvancedGraphAttrs((current) => !current)}
                        className="h-8 w-full rounded-md border border-border bg-background px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
                    >
                        {showAdvancedGraphAttrs ? 'Hide Advanced Fields' : 'Show Advanced Fields'}
                    </button>
                    {showAdvancedGraphAttrs && (
                        <div className="space-y-3 rounded-md border border-border/80 bg-background/40 p-3">
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Model Stylesheet</label>
                                <textarea
                                    value={graphAttrs.model_stylesheet || ''}
                                    onChange={(event) => updateGraphAttr('model_stylesheet', event.target.value)}
                                    className="h-20 w-full resize-none rounded-md border border-input bg-background px-2 py-1 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                            </div>
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Retry Target</label>
                                <input
                                    value={graphAttrs.retry_target || ''}
                                    onChange={(event) => updateGraphAttr('retry_target', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                            </div>
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Fallback Retry Target</label>
                                <input
                                    value={graphAttrs.fallback_retry_target || ''}
                                    onChange={(event) => updateGraphAttr('fallback_retry_target', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                            </div>
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Stack Child Dotfile</label>
                                <input
                                    value={graphAttrs['stack.child_dotfile'] || ''}
                                    onChange={(event) => updateGraphAttr('stack.child_dotfile', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    placeholder="child/flow.dot"
                                />
                            </div>
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Stack Child Workdir</label>
                                <input
                                    value={graphAttrs['stack.child_workdir'] || ''}
                                    onChange={(event) => updateGraphAttr('stack.child_workdir', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    placeholder="/abs/path/to/child"
                                />
                            </div>
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Tool Hooks Pre</label>
                                <input
                                    value={graphAttrs['tool_hooks.pre'] || ''}
                                    onChange={(event) => updateGraphAttr('tool_hooks.pre', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                            </div>
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Tool Hooks Post</label>
                                <input
                                    value={graphAttrs['tool_hooks.post'] || ''}
                                    onChange={(event) => updateGraphAttr('tool_hooks.post', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 font-mono text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                            </div>
                        </div>
                    )}
                </div>
            </div>

            <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                LLM Defaults (Flow Snapshot)
            </div>
            <div className="mt-3 space-y-3">
                <div className="space-y-1">
                    <label className="text-xs font-medium text-foreground">Default LLM Provider</label>
                    <input
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
                    <label className="text-xs font-medium text-foreground">Default LLM Model</label>
                    <input
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
                    <label className="text-xs font-medium text-foreground">Default Reasoning Effort</label>
                    <select
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
                        className="h-8 rounded-md border border-border px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
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
                        className="h-8 rounded-md border border-border px-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
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
                className="inline-flex h-9 items-center gap-2 rounded-md border border-border bg-background/90 px-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground shadow-sm hover:text-foreground"
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
