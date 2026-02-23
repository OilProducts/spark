import { useEffect, useRef, useState } from 'react'
import { useReactFlow } from '@xyflow/react'
import { useStore } from '@/store'
import { generateDot } from '@/lib/dotUtils'

export function GraphSettings() {
    const [isOpen, setIsOpen] = useState(false)
    const activeFlow = useStore((state) => state.activeFlow)
    const graphAttrs = useStore((state) => state.graphAttrs)
    const updateGraphAttr = useStore((state) => state.updateGraphAttr)
    const model = useStore((state) => state.model)
    const setModel = useStore((state) => state.setModel)
    const workingDir = useStore((state) => state.workingDir)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const { getNodes, getEdges } = useReactFlow()
    const saveTimer = useRef<number | null>(null)

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
        <div className="absolute right-4 top-4 z-20">
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
                </div>
            )}
        </div>
    )
}
