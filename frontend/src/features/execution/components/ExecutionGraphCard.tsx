import { useEffect, useMemo, useState } from 'react'
import {
    Background,
    Controls,
    MarkerType,
    MiniMap,
    ReactFlow,
    ReactFlowProvider,
    applyEdgeChanges,
    applyNodeChanges,
    useEdgesState,
    useNodesState,
    type Edge,
    type EdgeChange,
    type Node,
    type NodeChange,
    type NodeMouseHandler,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'

import {
    CanvasSessionModeProvider,
    ChildFlowExpansionToggle,
    edgeTypes,
    layoutWithElk,
    nodeTypes,
    nowMs,
    type HydratedFlowGraph,
} from '@/features/workflow-canvas'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { useStore } from '@/store'
import { RunSectionToggleButton } from '@/features/runs/components/RunSectionToggleButton'

type ExecutionGraphCanvasProps = {
    hydratedGraph: HydratedFlowGraph | null
    isContinuationMode: boolean
    selectedStartNodeId: string | null
    onSelectStartNode: (nodeId: string) => void
}

function applySelectedNode(nodes: Node[], selectedNodeId: string | null): Node[] {
    return nodes.map((node) => ({
        ...node,
        selected: selectedNodeId === node.id,
    }))
}

function ExecutionGraphCanvas({
    hydratedGraph,
    isContinuationMode,
    selectedStartNodeId,
    onSelectStartNode,
}: ExecutionGraphCanvasProps) {
    const [nodes, setNodes] = useNodesState<Node>([])
    const [edges, setEdges] = useEdgesState<Edge>([])
    const [lastLayoutMs, setLastLayoutMs] = useState(0)

    useEffect(() => {
        let cancelled = false

        const loadLayout = async () => {
            if (!hydratedGraph) {
                setNodes([])
                setEdges([])
                setLastLayoutMs(0)
                return
            }

            const layoutStart = nowMs()
            const laidOutGraph = await layoutWithElk(hydratedGraph.nodes, hydratedGraph.edges)
            if (cancelled) {
                return
            }

            setLastLayoutMs(Math.max(0, nowMs() - layoutStart))
            setNodes(applySelectedNode(laidOutGraph.nodes, selectedStartNodeId))
            setEdges(laidOutGraph.edges)
        }

        void loadLayout()
        return () => {
            cancelled = true
        }
    }, [hydratedGraph, selectedStartNodeId, setEdges, setNodes])

    const onNodesChange = (changes: NodeChange<Node>[]) => {
        setNodes((currentNodes) => applyNodeChanges(changes, currentNodes))
    }

    const onEdgesChange = (changes: EdgeChange<Edge>[]) => {
        setEdges((currentEdges) => applyEdgeChanges(changes, currentEdges))
    }

    const onNodeClick = useMemo<NodeMouseHandler<Node> | undefined>(() => {
        if (!isContinuationMode) {
            return undefined
        }
        return (_event, node) => {
            onSelectStartNode(node.id)
            setNodes((currentNodes) => applySelectedNode(currentNodes, node.id))
        }
    }, [isContinuationMode, onSelectStartNode, setNodes])

    if (nodes.length === 0 && edges.length === 0) {
        return (
            <div className="flex h-[28rem] items-center justify-center rounded-md border border-dashed border-border bg-muted/20">
                <Empty className="text-sm text-muted-foreground">
                    <EmptyHeader>
                        <EmptyDescription>Flow graph will appear once the preview loads.</EmptyDescription>
                    </EmptyHeader>
                </Empty>
            </div>
        )
    }

    return (
        <div data-testid="execution-graph-canvas" className="h-[32rem] overflow-hidden rounded-md border border-border/80 bg-background">
            <ReactFlow
                nodes={nodes}
                edges={edges}
                onNodesChange={onNodesChange}
                onEdgesChange={onEdgesChange}
                onNodeClick={onNodeClick}
                fitView
                nodesDraggable={false}
                nodesConnectable={false}
                elementsSelectable={isContinuationMode}
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

interface ExecutionGraphCardProps {
    hydratedGraph: HydratedFlowGraph | null
    isLoading: boolean
    loadError: string | null
    isContinuationMode: boolean
    expandChildFlows: boolean
    sourceMode: 'snapshot' | 'flow_name' | null
    selectedStartNodeId: string | null
    onSelectStartNode: (nodeId: string) => void
}

export function ExecutionGraphCard({
    hydratedGraph,
    isLoading,
    loadError,
    isContinuationMode,
    expandChildFlows,
    sourceMode,
    selectedStartNodeId,
    onSelectStartNode,
}: ExecutionGraphCardProps) {
    const diagnostics = useStore((state) => state.executionDiagnostics)
    const collapsed = useStore((state) => state.executionGraphCollapsed)
    const updateExecutionSession = useStore((state) => state.updateExecutionSession)

    const description = isContinuationMode
        ? `Pick the restart node on the ${sourceMode === 'flow_name' ? 'installed flow override' : 'source-run snapshot'} graph.`
        : 'Read-only preview of the selected execution flow.'

    return (
        <Card data-testid="execution-graph-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Flow Graph</h3>
                        <p className="text-xs leading-5 text-muted-foreground">{description}</p>
                    </div>
                    <div className="flex items-center gap-2">
                        {isContinuationMode && selectedStartNodeId ? (
                            <span
                                data-testid="execution-continuation-selected-node"
                                className="rounded-full border border-border/80 bg-muted/30 px-2 py-1 font-mono text-[11px] text-muted-foreground"
                            >
                                Start: {selectedStartNodeId}
                            </span>
                        ) : null}
                        <ChildFlowExpansionToggle
                            expanded={expandChildFlows}
                            onChange={(nextExpanded) => updateExecutionSession({ executionExpandChildFlows: nextExpanded })}
                            testId="execution-child-flow-toggle"
                        />
                        <RunSectionToggleButton
                            collapsed={collapsed}
                            onToggle={() => updateExecutionSession({ executionGraphCollapsed: !collapsed })}
                            testId="execution-graph-toggle-button"
                        />
                    </div>
                </div>
            </CardHeader>
            {!collapsed ? (
                <CardContent className="space-y-3 px-4">
                    {isLoading ? (
                        <Alert
                            data-testid="execution-graph-loading"
                            className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                        >
                            <AlertDescription className="text-inherit">
                                Loading graph preview…
                            </AlertDescription>
                        </Alert>
                    ) : null}
                    {loadError ? (
                        <Alert
                            data-testid="execution-graph-error"
                            className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive"
                        >
                            <AlertDescription className="text-inherit">{loadError}</AlertDescription>
                        </Alert>
                    ) : null}
                    {isContinuationMode ? (
                        <Alert
                            data-testid="execution-continuation-warning"
                            className="border-amber-500/40 bg-amber-500/10 px-3 py-2 text-amber-800"
                        >
                            <AlertDescription className="text-inherit">
                                Some nodes depend on upstream state captured in the source run and may fail when
                                restarted cold.
                            </AlertDescription>
                        </Alert>
                    ) : null}
                    {diagnostics.length > 0 ? (
                        <div data-testid="execution-graph-diagnostics" className="rounded-md border border-border/80 bg-muted/20 p-3">
                            <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                Graph diagnostics
                            </p>
                            <ul className="mt-2 space-y-2 text-sm">
                                {diagnostics.slice(0, 6).map((diagnostic, index) => (
                                    <li
                                        key={`${diagnostic.rule_id}-${diagnostic.node_id || 'graph'}-${index}`}
                                        className="rounded border border-border/80 bg-background/80 px-3 py-2"
                                    >
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
                    <CanvasSessionModeProvider mode="execution">
                        <ReactFlowProvider>
                            <ExecutionGraphCanvas
                                hydratedGraph={hydratedGraph}
                                isContinuationMode={isContinuationMode}
                                selectedStartNodeId={selectedStartNodeId}
                                onSelectStartNode={onSelectStartNode}
                            />
                        </ReactFlowProvider>
                    </CanvasSessionModeProvider>
                </CardContent>
            ) : null}
        </Card>
    )
}
