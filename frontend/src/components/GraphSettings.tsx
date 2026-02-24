import { useEffect, useRef, useState } from 'react'
import { useReactFlow } from '@xyflow/react'
import { useStore } from '@/store'
import { generateDot } from '@/lib/dotUtils'
import { getModelSuggestions, LLM_PROVIDER_OPTIONS } from '@/lib/llmSuggestions'

export function GraphSettings() {
    const [isOpen, setIsOpen] = useState(false)
    const activeFlow = useStore((state) => state.activeFlow)
    const graphAttrs = useStore((state) => state.graphAttrs)
    const updateGraphAttr = useStore((state) => state.updateGraphAttr)
    const model = useStore((state) => state.model)
    const setModel = useStore((state) => state.setModel)
    const workingDir = useStore((state) => state.workingDir)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const setUiDefault = useStore((state) => state.setUiDefault)
    const { getNodes, getEdges } = useReactFlow()
    const saveTimer = useRef<number | null>(null)
    const flowProviderFallback = graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || ''

    useEffect(() => {
        if (!activeFlow) return
        if (saveTimer.current) {
            window.clearTimeout(saveTimer.current)
        }
        saveTimer.current = window.setTimeout(() => {
            const dot = generateDot(activeFlow, getNodes(), getEdges(), graphAttrs)
            fetch('/api/flows', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ name: activeFlow, content: dot }),
            }).catch(console.error)
        }, 200)

        return () => {
            if (saveTimer.current) {
                window.clearTimeout(saveTimer.current)
            }
        }
    }, [activeFlow, graphAttrs, getNodes, getEdges])

    return (
        <div className="absolute right-4 top-4 z-20 flex flex-col items-end">
            <button
                onClick={() => setIsOpen((open) => !open)}
                className="inline-flex h-9 items-center gap-2 rounded-md border border-border bg-background/90 px-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground shadow-sm hover:text-foreground"
            >
                Graph Settings
            </button>
            {isOpen && (
                <div className="mt-2 w-80 rounded-md border border-border bg-card p-4 shadow-lg">
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

                    <div className="mt-4 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                        UI Defaults (Global)
                    </div>
                    <div className="mt-3 space-y-3">
                        <div className="space-y-1">
                            <label className="text-xs font-medium text-foreground">Default LLM Provider</label>
                            <input
                                value={uiDefaults.llm_provider}
                                onChange={(event) => setUiDefault('llm_provider', event.target.value)}
                                list="global-llm-provider-options"
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                placeholder="openai"
                            />
                            <datalist id="global-llm-provider-options">
                                {LLM_PROVIDER_OPTIONS.map((provider) => (
                                    <option key={provider} value={provider} />
                                ))}
                            </datalist>
                        </div>
                        <div className="space-y-1">
                            <label className="text-xs font-medium text-foreground">Default LLM Model</label>
                            <input
                                value={uiDefaults.llm_model}
                                onChange={(event) => setUiDefault('llm_model', event.target.value)}
                                list="global-llm-model-options"
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                placeholder="gpt-5.2"
                            />
                            <datalist id="global-llm-model-options">
                                {getModelSuggestions(uiDefaults.llm_provider).map((modelOption) => (
                                    <option key={modelOption} value={modelOption} />
                                ))}
                            </datalist>
                        </div>
                        <div className="space-y-1">
                            <label className="text-xs font-medium text-foreground">Default Reasoning Effort</label>
                            <select
                                value={uiDefaults.reasoning_effort}
                                onChange={(event) => setUiDefault('reasoning_effort', event.target.value)}
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            >
                                <option value="">Use handler default</option>
                                <option value="low">Low</option>
                                <option value="medium">Medium</option>
                                <option value="high">High</option>
                            </select>
                        </div>
                    </div>

                    <div className="mt-4 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
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
                        <div className="space-y-1">
                            <label className="text-xs font-medium text-foreground">Model Stylesheet</label>
                            <textarea
                                value={graphAttrs.model_stylesheet || ''}
                                onChange={(event) => updateGraphAttr('model_stylesheet', event.target.value)}
                                className="h-20 w-full resize-none rounded-md border border-input bg-background px-2 py-1 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            />
                        </div>
                        <div className="grid grid-cols-2 gap-3">
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Default Max Retry</label>
                                <input
                                    value={graphAttrs.default_max_retry ?? ''}
                                    onChange={(event) => updateGraphAttr('default_max_retry', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                            </div>
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Default Fidelity</label>
                                <input
                                    value={graphAttrs.default_fidelity || ''}
                                    onChange={(event) => updateGraphAttr('default_fidelity', event.target.value)}
                                    className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    placeholder="full"
                                />
                            </div>
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
                    </div>

                    <div className="mt-4 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                        UI Defaults (Flow)
                    </div>
                    <div className="mt-3 space-y-3">
                        <div className="space-y-1">
                            <label className="text-xs font-medium text-foreground">Default LLM Provider</label>
                            <input
                                value={graphAttrs.ui_default_llm_provider || ''}
                                onChange={(event) => updateGraphAttr('ui_default_llm_provider', event.target.value)}
                                list="flow-llm-provider-options"
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                placeholder={uiDefaults.llm_provider ? `Global: ${uiDefaults.llm_provider}` : 'Uses global default'}
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
                                placeholder={uiDefaults.llm_model ? `Global: ${uiDefaults.llm_model}` : 'Uses global default'}
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
                    </div>
                </div>
            )}
        </div>
    )
}
