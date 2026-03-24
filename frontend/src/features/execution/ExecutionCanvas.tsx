import { useCallback, useEffect, useRef, useState } from 'react'
import {
    Background,
    Controls,
    MiniMap,
    ReactFlow,
    applyEdgeChanges,
    applyNodeChanges,
    useEdgesState,
    useNodesState,
} from '@xyflow/react'
import type { Edge, EdgeChange, Node, NodeChange, OnSelectionChangeParams } from '@xyflow/react'
import '@xyflow/react/dist/style.css'

import { useStore } from '@/store'
import { recordFlowLoadDebug, summarizeDiagnosticsForFlowLoadDebug } from '@/lib/flowLoadDebug'
import {
    fetchFlowPayloadValidated,
    fetchPreviewValidated,
    type PreviewResponsePayload,
} from '@/lib/attractorClient'
import {
    buildHydratedFlowGraph,
    edgeTypes,
    EDGE_CLASS,
    EDGE_INTERACTION_WIDTH,
    EDGE_TYPE,
    layoutWithElk,
    nodeTypes,
    normalizeLegacyDot,
    nowMs,
} from '@/features/workflow-canvas'

type PreviewResponse = PreviewResponsePayload

export function ExecutionCanvas() {
    const executionFlow = useStore((state) => state.executionFlow)
    const nodeStatuses = useStore((state) => state.nodeStatuses)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const replaceExecutionGraphAttrs = useStore((state) => state.replaceExecutionGraphAttrs)
    const setExecutionDiagnostics = useStore((state) => state.setExecutionDiagnostics)
    const clearExecutionDiagnostics = useStore((state) => state.clearExecutionDiagnostics)
    const [nodes, setNodes] = useNodesState<Node>([])
    const [edges, setEdges] = useEdgesState<Edge>([])
    const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
    const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null)
    const [lastLayoutMs, setLastLayoutMs] = useState(0)
    const [lastPreviewMs, setLastPreviewMs] = useState(0)
    const activeFlowLoadIdRef = useRef(0)

    const hydrateFromPreview = useCallback(async (
        preview: PreviewResponse,
        sourceDot?: string,
        debugContext?: { loadId: number | null; source: 'flow-load-source-dot' },
    ) => {
        if (!executionFlow) {
            return false
        }
        const hydratedGraph = buildHydratedFlowGraph(
            executionFlow,
            preview,
            uiDefaults,
            sourceDot,
        )
        if (!hydratedGraph) {
            return false
        }

        recordFlowLoadDebug('hydrate:start', executionFlow, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'flow-load-source-dot',
            sourceDotLength: sourceDot?.length ?? null,
            nodeCount: hydratedGraph.nodes.length,
            edgeCount: hydratedGraph.edges.length,
            graphAttrCount: Object.keys(hydratedGraph.graphAttrs).length,
            session: 'execution',
        })

        replaceExecutionGraphAttrs(hydratedGraph.graphAttrs)

        const layoutStart = nowMs()
        let layoutDurationMs = 0
        let laidOutNodes = hydratedGraph.nodes
        let laidOutEdges = hydratedGraph.edges
        try {
            const layoutGraph = await layoutWithElk(hydratedGraph.nodes, hydratedGraph.edges)
            laidOutNodes = layoutGraph.nodes
            laidOutEdges = layoutGraph.edges
        } catch (error) {
            console.error('ELK layout failed, using fallback positions.', error)
        } finally {
            layoutDurationMs = Math.max(0, nowMs() - layoutStart)
            setLastLayoutMs(layoutDurationMs)
        }

        setNodes(laidOutNodes)
        setEdges(laidOutEdges)
        recordFlowLoadDebug('hydrate:complete', executionFlow, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'flow-load-source-dot',
            nodeCount: laidOutNodes.length,
            edgeCount: laidOutEdges.length,
            layoutMs: layoutDurationMs,
            session: 'execution',
        })
        return true
    }, [
        executionFlow,
        replaceExecutionGraphAttrs,
        setEdges,
        setNodes,
        uiDefaults,
    ])

    const requestPreview = useCallback(async (
        dot: string,
        debugContext?: { loadId: number | null; source: 'flow-load-source-dot' },
    ) => {
        recordFlowLoadDebug('preview:request', executionFlow, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'flow-load-source-dot',
            dotLength: dot.length,
            session: 'execution',
        })
        const previewStart = nowMs()
        const preview = await fetchPreviewValidated(dot)
        const elapsed = Math.max(0, nowMs() - previewStart)
        setLastPreviewMs(elapsed)
        recordFlowLoadDebug('preview:response', executionFlow, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'flow-load-source-dot',
            elapsedMs: elapsed,
            status: preview.status,
            hasGraph: Boolean(preview.graph),
            backendErrorCount: preview.errors?.length ?? 0,
            session: 'execution',
            ...summarizeDiagnosticsForFlowLoadDebug(preview.diagnostics),
        })
        if (preview.diagnostics) {
            setExecutionDiagnostics(preview.diagnostics)
        } else {
            clearExecutionDiagnostics()
        }
        return preview
    }, [clearExecutionDiagnostics, executionFlow, setExecutionDiagnostics])

    useEffect(() => {
        if (!executionFlow) {
            activeFlowLoadIdRef.current += 1
            replaceExecutionGraphAttrs({})
            clearExecutionDiagnostics()
            setNodes([])
            setEdges([])
            setSelectedNodeId(null)
            setSelectedEdgeId(null)
            setLastLayoutMs(0)
            setLastPreviewMs(0)
            return
        }

        const loadId = activeFlowLoadIdRef.current + 1
        activeFlowLoadIdRef.current = loadId
        replaceExecutionGraphAttrs({})
        clearExecutionDiagnostics()

        fetchFlowPayloadValidated(executionFlow)
            .then((data) => {
                const normalizedContent = normalizeLegacyDot(data.content)
                return requestPreview(normalizedContent, {
                    loadId,
                    source: 'flow-load-source-dot',
                }).then((preview) => ({
                    normalizedContent,
                    preview,
                }))
            })
            .then(({ normalizedContent, preview }) => hydrateFromPreview(preview, normalizedContent, {
                loadId,
                source: 'flow-load-source-dot',
            }))
            .catch((error) => {
                console.error(error)
                replaceExecutionGraphAttrs({})
                clearExecutionDiagnostics()
                setNodes([])
                setEdges([])
                setSelectedNodeId(null)
                setSelectedEdgeId(null)
            })
    }, [
        clearExecutionDiagnostics,
        executionFlow,
        hydrateFromPreview,
        replaceExecutionGraphAttrs,
        requestPreview,
        setEdges,
        setNodes,
    ])

    const onNodesChange = useCallback((changes: NodeChange<Node>[]) => {
        setNodes((currentNodes) => applyNodeChanges(changes, currentNodes))
    }, [setNodes])

    const onEdgesChange = useCallback((changes: EdgeChange<Edge>[]) => {
        setEdges((currentEdges) => applyEdgeChanges(changes, currentEdges))
    }, [setEdges])

    const onSelectionChange = useCallback(({ nodes: selectedNodes, edges: selectedEdges }: OnSelectionChangeParams) => {
        const selectedNode = selectedNodes.find((node) => node.selected)
        const selectedEdge = selectedEdges.find((edge) => edge.selected)
        setSelectedNodeId(selectedNode ? selectedNode.id : null)
        setSelectedEdgeId(selectedEdge ? selectedEdge.id : null)
    }, [])

    useEffect(() => {
        setNodes((currentNodes) => currentNodes.map((node) => {
            const nextStatus = nodeStatuses[node.id] || 'idle'
            if (node.data?.status === nextStatus) {
                return node
            }
            return { ...node, data: { ...node.data, status: nextStatus } }
        }))
    }, [nodeStatuses, setNodes])

    useEffect(() => {
        setNodes((currentNodes) =>
            currentNodes.map((node) => {
                const shouldSelect = !selectedEdgeId && node.id === selectedNodeId
                return node.selected === shouldSelect ? node : { ...node, selected: shouldSelect }
            }),
        )
        setEdges((currentEdges) =>
            currentEdges.map((edge) => {
                const shouldSelect = edge.id === selectedEdgeId
                return edge.selected === shouldSelect ? edge : { ...edge, selected: shouldSelect }
            }),
        )
    }, [selectedEdgeId, selectedNodeId, setEdges, setNodes])

    useEffect(() => {
        if (selectedNodeId && !nodes.some((node) => node.id === selectedNodeId)) {
            setSelectedNodeId(null)
        }
        if (selectedEdgeId && !edges.some((edge) => edge.id === selectedEdgeId)) {
            setSelectedEdgeId(null)
        }
    }, [edges, nodes, selectedEdgeId, selectedNodeId])

    if (!executionFlow) {
        return (
            <div
                data-testid="execution-no-flow-state"
                className="flex h-full items-center justify-center p-6"
            >
                <div className="max-w-md rounded-lg border border-dashed border-border bg-background/70 px-6 py-5 text-center shadow-sm">
                    <p className="text-sm font-medium text-foreground">Select a flow to inspect or execute.</p>
                    <p className="mt-2 text-sm text-muted-foreground">
                        Execution keeps its own flow context separate from the editor.
                    </p>
                </div>
            </div>
        )
    }

    return (
        <div className="flow-surface w-full h-full relative">
            <ReactFlow
                className="flow-canvas"
                style={{ background: 'transparent' }}
                nodes={nodes}
                edges={edges}
                onNodesChange={onNodesChange}
                onEdgesChange={onEdgesChange}
                onSelectionChange={onSelectionChange}
                nodeTypes={nodeTypes}
                edgeTypes={edgeTypes}
                defaultEdgeOptions={{
                    type: EDGE_TYPE,
                    className: EDGE_CLASS,
                    interactionWidth: EDGE_INTERACTION_WIDTH,
                }}
                elevateEdgesOnSelect
                fitView
                colorMode="light"
                nodesDraggable={false}
                nodesConnectable={false}
                elementsSelectable
                minZoom={0.1}
                maxZoom={1.5}
            >
                <Controls />
                <MiniMap
                    nodeColor="hsl(var(--muted))"
                    maskColor="hsl(var(--background)/0.5)"
                    className="flow-minimap"
                />
                <Background gap={20} size={1} color="hsl(var(--border))" />
            </ReactFlow>
            <div className="absolute left-4 top-4 z-10 flex gap-2">
                <div
                    data-testid="execution-canvas-performance-profile"
                    data-layout-ms={Math.round(lastLayoutMs)}
                    data-preview-ms={Math.round(lastPreviewMs)}
                    className="inline-flex items-center rounded-md border border-border/70 bg-background/90 px-3 py-1.5 text-xs text-muted-foreground shadow-sm"
                >
                    Execution graph preview: {Math.round(lastPreviewMs)}ms. Layout: {Math.round(lastLayoutMs)}ms.
                </div>
            </div>
        </div>
    )
}
