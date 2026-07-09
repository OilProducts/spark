import { useEffect, useMemo, useRef, useState } from 'react'
import {
    Background,
    Controls,
    MarkerType,
    MiniMap,
    ReactFlow,
    ReactFlowProvider,
    type Node,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'

import { isAbortError } from '@/lib/api/shared'
import { useStore, type NodeStatus } from '@/store'
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

const MIN_GRAPH_PANE_HEIGHT = 280
const MAX_GRAPH_PANE_HEIGHT = 960
const GRAPH_PANE_KEYBOARD_STEP = 32

const clampGraphPaneHeight = (height: number) => (
    Math.min(MAX_GRAPH_PANE_HEIGHT, Math.max(MIN_GRAPH_PANE_HEIGHT, Math.round(height)))
)

const EMPTY_GRAPH_NODES: Node[] = []

type RunGraphCanvasInnerProps = {
    run: RunRecord
    refreshToken: number
    nodeStatusesById: Record<string, NodeStatus>
    selectedNodeId: string | null
    onSelectNode: (nodeId: string | null) => void
    paneHeight: number | 'fill'
}

function RunGraphCanvasInner({
    run,
    refreshToken,
    nodeStatusesById,
    selectedNodeId,
    onSelectNode,
    paneHeight,
}: RunGraphCanvasInnerProps) {
    const replaceRunGraphAttrs = useStore((state) => state.replaceRunGraphAttrs)
    const setRunDiagnostics = useStore((state) => state.setRunDiagnostics)
    const clearRunDiagnostics = useStore((state) => state.clearRunDiagnostics)
    const runDetailSession = useStore((state) => state.runDetailSessionsByRunId[run.run_id] ?? null)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const activeLoadRef = useRef(0)
    const nodes = runDetailSession?.graphNodes ?? EMPTY_GRAPH_NODES
    const edges = runDetailSession?.graphEdges ?? []

    const decoratedNodes = useMemo(() => (
        nodes.map((node) => {
            const status = nodeStatusesById[node.id] ?? 'idle'
            const selected = node.id === selectedNodeId
            if ((node.data?.status ?? 'idle') === status && Boolean(node.selected) === selected) {
                return node
            }
            return {
                ...node,
                selected,
                data: {
                    ...node.data,
                    status,
                },
            }
        })
    ), [nodes, nodeStatusesById, selectedNodeId])

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
        <div
            data-testid="run-graph-canvas"
            className={
                paneHeight === 'fill'
                    ? 'min-h-0 flex-1 overflow-hidden rounded-md border border-border/80 bg-background'
                    : 'overflow-hidden rounded-md border border-border/80 bg-background'
            }
            style={paneHeight === 'fill' ? undefined : { height: `${paneHeight}px` }}
        >
            <ReactFlow
                nodes={decoratedNodes}
                edges={edges}
                fitView
                fitViewOptions={{ padding: 0.15 }}
                nodesDraggable={false}
                nodesConnectable={false}
                elementsSelectable={true}
                nodeTypes={nodeTypes}
                edgeTypes={edgeTypes}
                onNodeClick={(_, node: Node) => {
                    onSelectNode(node.id === selectedNodeId ? null : node.id)
                }}
                onPaneClick={() => {
                    onSelectNode(null)
                }}
                defaultEdgeOptions={{
                    markerEnd: {
                        type: MarkerType.ArrowClosed,
                    },
                }}
                deleteKeyCode={null}
                multiSelectionKeyCode={null}
                proOptions={{ hideAttribution: true }}
            >
                {decoratedNodes.length > 10 && (paneHeight === 'fill' || paneHeight >= 480) ? (
                    <MiniMap pannable zoomable position="top-right" />
                ) : null}
                <Controls showInteractive={false} />
                <Background gap={24} size={1} />
            </ReactFlow>
        </div>
    )
}

interface RunGraphCardProps {
    run: RunRecord
    nodeStatusesById: Record<string, NodeStatus>
    selectedNodeId: string | null
    onSelectNode: (nodeId: string | null) => void
    /** Fill the parent column instead of using the session pane height. */
    fillHeight?: boolean
}

export function RunGraphCard({
    run,
    nodeStatusesById,
    selectedNodeId,
    onSelectNode,
    fillHeight = false,
}: RunGraphCardProps) {
    const diagnostics = useStore((state) => state.runDiagnostics)
    const [refreshToken, setRefreshToken] = useState(0)
    const runDetailSession = useStore((state) => state.runDetailSessionsByRunId[run.run_id] ?? null)
    const updateRunDetailSession = useStore((state) => state.updateRunDetailSession)
    const graphStatus = runDetailSession?.graphStatus ?? 'idle'
    const graphError = runDetailSession?.graphError ?? null
    const expandChildFlows = runDetailSession?.expandChildFlows ?? false
    const paneHeight = clampGraphPaneHeight(runDetailSession?.graphPaneHeight ?? 512)
    const hasRenderableGraph = (runDetailSession?.graphNodes.length ?? 0) > 0 || (runDetailSession?.graphEdges.length ?? 0) > 0
    const resizeStateRef = useRef<{ startY: number; startHeight: number } | null>(null)

    const setPaneHeight = (height: number) => {
        updateRunDetailSession(run.run_id, { graphPaneHeight: clampGraphPaneHeight(height) })
    }

    return (
        <Card
            data-testid="run-graph-panel"
            className={fillHeight ? 'flex h-full min-h-0 flex-col gap-2 py-3' : 'gap-2 py-3'}
        >
            <CardHeader className="gap-1 px-4">
                <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
                    <h3
                        className="shrink-0 whitespace-nowrap text-sm font-semibold text-foreground"
                        title="Live node states for the selected run. Click a node to focus its activity; click the background to clear the selection."
                    >
                        Run Graph
                    </h3>
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
                    </div>
                </div>
            </CardHeader>
            <CardContent
                className={fillHeight ? 'flex min-h-0 flex-1 flex-col gap-2 px-4' : 'space-y-2 px-4'}
            >
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
                                nodeStatusesById={nodeStatusesById}
                                selectedNodeId={selectedNodeId}
                                onSelectNode={onSelectNode}
                                paneHeight={fillHeight ? 'fill' : paneHeight}
                            />
                        </ReactFlowProvider>
                    </CanvasSessionModeProvider>
                ) : null}
                {fillHeight ? null : (
                <div
                    data-testid="run-graph-resize-handle"
                    role="separator"
                    aria-label="Resize run graph"
                    aria-orientation="horizontal"
                    tabIndex={0}
                    onPointerDown={(event) => {
                        event.preventDefault()
                        event.currentTarget.setPointerCapture(event.pointerId)
                        resizeStateRef.current = { startY: event.clientY, startHeight: paneHeight }
                    }}
                    onPointerMove={(event) => {
                        const resizeState = resizeStateRef.current
                        if (!resizeState) {
                            return
                        }
                        setPaneHeight(resizeState.startHeight + (event.clientY - resizeState.startY))
                    }}
                    onPointerUp={(event) => {
                        resizeStateRef.current = null
                        event.currentTarget.releasePointerCapture(event.pointerId)
                    }}
                    onKeyDown={(event) => {
                        if (event.key === 'ArrowUp') {
                            event.preventDefault()
                            setPaneHeight(paneHeight - GRAPH_PANE_KEYBOARD_STEP)
                        } else if (event.key === 'ArrowDown') {
                            event.preventDefault()
                            setPaneHeight(paneHeight + GRAPH_PANE_KEYBOARD_STEP)
                        }
                    }}
                    className="group flex h-3 cursor-row-resize items-center justify-center rounded-sm hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                >
                    <span className="h-1 w-12 rounded-full bg-border transition-colors group-hover:bg-muted-foreground/70" />
                </div>
                )}
            </CardContent>
        </Card>
    )
}
