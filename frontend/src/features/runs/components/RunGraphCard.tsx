import { useEffect, useRef, useState } from 'react'
import {
    Background,
    Controls,
    MarkerType,
    MiniMap,
    ReactFlow,
    ReactFlowProvider,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'

import { isAbortError } from '@/lib/api/shared'
import { useStore } from '@/store'
import {
    buildHydratedFlowGraph,
    CanvasSessionModeProvider,
    ChildFlowExpansionToggle,
    edgeTypes,
    layoutWithElk,
    nodeTypes,
    nowMs,
} from '@/features/workflow-canvas'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import type { RunRecord } from '../model/shared'
import { loadRunGraphPreview } from '../services/runGraphTransport'
import { RunSectionToggleButton } from './RunSectionToggleButton'

type RunGraphCanvasInnerProps = {
    run: RunRecord
    refreshToken: number
}

function RunGraphCanvasInner({
    run,
    refreshToken,
}: RunGraphCanvasInnerProps) {
    const replaceRunGraphAttrs = useStore((state) => state.replaceRunGraphAttrs)
    const setRunDiagnostics = useStore((state) => state.setRunDiagnostics)
    const clearRunDiagnostics = useStore((state) => state.clearRunDiagnostics)
    const runDetailSession = useStore((state) => state.runDetailSessionsByRunId[run.run_id] ?? null)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const activeLoadRef = useRef(0)
    const nodes = runDetailSession?.graphNodes ?? []
    const edges = runDetailSession?.graphEdges ?? []
    const lastLayoutMs = runDetailSession?.graphLastLayoutMs ?? 0

    useEffect(() => {
        const loadId = activeLoadRef.current + 1
        activeLoadRef.current = loadId
        const controller = new AbortController()
        let cancelled = false
        const isCurrentLoad = () => !cancelled && activeLoadRef.current === loadId

        replaceRunGraphAttrs({})
        clearRunDiagnostics()
        updateRunDetailSession(run.run_id, {
            graphStatus: 'loading',
            graphError: null,
            graphNodes: [],
            graphEdges: [],
            graphLastLayoutMs: 0,
        })

        const startLoad = async () => {
            try {
                const preview = await loadRunGraphPreview(
                    run.run_id,
                    { signal: controller.signal },
                    { expandChildren: runDetailSession?.expandChildFlows ?? false },
                )
                if (!isCurrentLoad()) {
                    return
                }

                if (preview.diagnostics) {
                    setRunDiagnostics(preview.diagnostics)
                } else {
                    clearRunDiagnostics()
                }

                const hydratedGraph = buildHydratedFlowGraph(
                    run.flow_name || run.run_id,
                    preview,
                    {
                        llm_model: '',
                        llm_provider: '',
                        llm_profile: '',
                        reasoning_effort: '',
                    },
                    undefined,
                    { expandChildren: runDetailSession?.expandChildFlows ?? false },
                )
                if (!hydratedGraph) {
                    updateRunDetailSession(run.run_id, {
                        graphStatus: 'error',
                        graphError: 'Run graph preview did not include a renderable graph.',
                    })
                    return
                }

                const layoutStart = nowMs()
                const laidOutGraph = await layoutWithElk(hydratedGraph.nodes, hydratedGraph.edges)
                if (!isCurrentLoad()) {
                    return
                }

                replaceRunGraphAttrs(hydratedGraph.graphAttrs)
                updateRunDetailSession(run.run_id, {
                    graphStatus: 'ready',
                    graphError: null,
                    graphNodes: laidOutGraph.nodes,
                    graphEdges: laidOutGraph.edges,
                    graphLastLayoutMs: Math.max(0, nowMs() - layoutStart),
                })
            } catch (error) {
                if (controller.signal.aborted || isAbortError(error)) {
                    return
                }
                console.error(error)
                if (!isCurrentLoad()) {
                    return
                }
                updateRunDetailSession(run.run_id, {
                    graphStatus: 'error',
                    graphError: error instanceof Error ? error.message : 'Unable to load the run graph preview.',
                    graphNodes: [],
                    graphEdges: [],
                    graphLastLayoutMs: 0,
                })
            }
        }

        void startLoad()

        return () => {
            cancelled = true
            controller.abort()
        }
    }, [
        clearRunDiagnostics,
        refreshToken,
        replaceRunGraphAttrs,
        run.flow_name,
        run.run_id,
        runDetailSession?.expandChildFlows,
        setRunDiagnostics,
        updateRunDetailSession,
    ])

    return (
        <div data-testid="run-graph-canvas" className="h-[32rem] overflow-hidden rounded-md border border-border/80 bg-background">
            <ReactFlow
                nodes={nodes}
                edges={edges}
                fitView
                nodesDraggable={false}
                nodesConnectable={false}
                elementsSelectable={true}
                nodeTypes={nodeTypes}
                edgeTypes={edgeTypes}
                defaultEdgeOptions={{
                    markerEnd: {
                        type: MarkerType.ArrowClosed,
                    },
                }}
                deleteKeyCode={null}
                multiSelectionKeyCode={null}
                proOptions={{ hideAttribution: true }}
            >
                <MiniMap pannable zoomable />
                <Controls showInteractive={false} />
                <Background gap={24} size={1} />
            </ReactFlow>
            <div className="border-t border-border/70 bg-muted/20 px-3 py-2 text-[11px] text-muted-foreground">
                Last layout: {Math.round(lastLayoutMs)}ms
            </div>
        </div>
    )
}

export function RunGraphCard({ run }: { run: RunRecord }) {
    const diagnostics = useStore((state) => state.runDiagnostics)
    const [refreshToken, setRefreshToken] = useState(0)
    const runDetailSession = useStore((state) => state.runDetailSessionsByRunId[run.run_id] ?? null)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const collapsed = runDetailSession?.isGraphCollapsed ?? true
    const graphStatus = runDetailSession?.graphStatus ?? 'idle'
    const graphError = runDetailSession?.graphError ?? null
    const expandChildFlows = runDetailSession?.expandChildFlows ?? false
    const hasRenderableGraph = (runDetailSession?.graphNodes.length ?? 0) > 0 || (runDetailSession?.graphEdges.length ?? 0) > 0

    return (
        <Card data-testid="run-graph-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Run Graph</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Stored run snapshot reference for the selected workflow. This card is a structural
                            reference, not the primary live-status surface.
                        </p>
                    </div>
                    <div className="flex items-center gap-2">
                        <Button
                            onClick={() => setRefreshToken((current) => current + 1)}
                            data-testid="run-graph-refresh-button"
                            variant="outline"
                            size="xs"
                        >
                            {graphStatus === 'loading' ? 'Refreshing…' : 'Refresh'}
                        </Button>
                        <ChildFlowExpansionToggle
                            expanded={expandChildFlows}
                            onChange={(nextExpanded) => updateRunDetailSession(run.run_id, { expandChildFlows: nextExpanded })}
                            testId="run-child-flow-toggle"
                        />
                        <RunSectionToggleButton
                            collapsed={collapsed}
                            onToggle={() => updateRunDetailSession(run.run_id, { isGraphCollapsed: !collapsed })}
                            testId="run-graph-toggle-button"
                        />
                    </div>
                </div>
            </CardHeader>
            {!collapsed ? (
                <CardContent className="space-y-3 px-4">
                {graphStatus !== 'ready' && !graphError ? (
                    <Alert
                        data-testid="run-graph-loading"
                        className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                    >
                        <AlertDescription className="text-inherit">
                            Restoring run graph…
                        </AlertDescription>
                    </Alert>
                ) : null}
                {graphError ? (
                    <Alert
                        data-testid="run-graph-error"
                        className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive"
                    >
                        <AlertDescription className="text-inherit">{graphError}</AlertDescription>
                    </Alert>
                ) : null}
                {diagnostics.length > 0 ? (
                    <div data-testid="run-graph-diagnostics" className="rounded-md border border-border/80 bg-muted/20 p-3">
                        <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                            Graph diagnostics
                        </p>
                        <ul className="mt-2 space-y-2 text-sm">
                            {diagnostics.slice(0, 6).map((diagnostic, index) => (
                                <li key={`${diagnostic.rule_id}-${diagnostic.node_id || 'graph'}-${index}`} className="rounded border border-border/80 bg-background/80 px-3 py-2">
                                    <div className="flex flex-wrap items-center gap-2 text-xs uppercase tracking-wide text-muted-foreground">
                                        <span>{diagnostic.severity}</span>
                                        <span>{diagnostic.rule_id}</span>
                                        {diagnostic.node_id ? <span>{diagnostic.node_id}</span> : null}
                                    </div>
                                    <p className="mt-1 text-sm text-foreground">{diagnostic.message}</p>
                                </li>
                            ))}
                        </ul>
                    </div>
                ) : null}
                {graphStatus === 'ready' && !hasRenderableGraph ? (
                    <div className="flex h-[28rem] items-center justify-center rounded-md border border-dashed border-border bg-muted/20">
                        <Empty className="text-sm text-muted-foreground">
                            <EmptyHeader>
                                <EmptyDescription>
                                    No run graph preview is available for this run.
                                </EmptyDescription>
                            </EmptyHeader>
                        </Empty>
                    </div>
                ) : null}
                {!graphError && (graphStatus !== 'ready' || hasRenderableGraph) ? (
                    <CanvasSessionModeProvider mode="runs">
                        <ReactFlowProvider>
                            <RunGraphCanvasInner
                                run={run}
                                refreshToken={refreshToken}
                            />
                        </ReactFlowProvider>
                    </CanvasSessionModeProvider>
                ) : null}
                </CardContent>
            ) : null}
        </Card>
    )
}
